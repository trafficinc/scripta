use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Server {
    child: Child,
    directory: tempfile::TempDir,
    address: SocketAddr,
}
impl Server {
    fn start(source: &str, options: &[&str]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.scripta");
        fs::write(&path, source).unwrap();
        let log = fs::File::create(directory.path().join("server.log")).unwrap();
        let worker_count = if options.contains(&"--workers") {
            vec![]
        } else {
            vec!["--workers", "1"]
        };
        let child = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg("serve")
            .arg(path)
            .args(["--port", "0"])
            .args(worker_count)
            .args(options)
            .env("SCR_MAX_STEPS", "100000000000")
            .current_dir(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut server = Self {
            child,
            directory,
            address: "127.0.0.1:0".parse().unwrap(),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let log = fs::read_to_string(server.directory.path().join("server.log")).unwrap();
            if let Some(line) = log
                .lines()
                .find(|l| l.starts_with("Scripta listening on http://") && l.ends_with(" workers)"))
            {
                server.address = line
                    .trim_start_matches("Scripta listening on http://")
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                return server;
            }
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "server failed: {log}"
            );
            assert!(Instant::now() < deadline, "server startup timed out: {log}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn raw(&self, request: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect(self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(request).unwrap();
        let mut output = vec![];
        stream.read_to_end(&mut output).unwrap();
        output
    }
    fn request(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        content_type: &str,
    ) -> (u16, String, Vec<u8>) {
        let mut request=format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
        request.extend_from_slice(body);
        parse(&self.raw(&request))
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "server log: {}",
                fs::read_to_string(self.directory.path().join("server.log")).unwrap_or_default()
            );
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn parse(bytes: &[u8]) -> (u16, String, Vec<u8>) {
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("missing response headers");
    let headers = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, headers, bytes[split + 4..].to_vec())
}
#[test]
fn serves_requests_redirects_binary_and_isolated_state_on_persistent_workers() {
    let server = Server::start(
        r#"<?scripta
$r=Web::request();
if ($r->path === "/redirect") {Web::redirect("/done",303);exit 0;}
if ($r->path === "/binary") {Web::header("Content-Type","application/octet-stream");Web::write($r->bytes());exit 0;}
if ($r->path === "/form") {Web::header("Content-Type","application/json");echo Json::encode($r->form());exit 0;}
if ($r->path === "/empty") {Web::status(204);echo "discard";exit 0;}
if ($r->path === "/post") {Web::header("Content-Type","application/json");echo Json::encode($r->json());exit 0;}
if ($r->path === "/mutate") {$secret=42;Env::set("SCR_WEB_ISOLATED","changed");}
if ($r->path === "/leak") {echo $secret;exit 0;}
Web::header("Content-Type","application/json");
Web::addHeader("Set-Cookie","a=1");Web::addHeader("Set-Cookie","b=2");
echo Json::encode(["pid"=>Process::currentId(),"path"=>$r->path,"method"=>$r->method,"query"=>$r->query(),"env"=>Env::get("SCR_WEB_ISOLATED")]);
"#,
        &[],
    );
    let (status, headers, body) = server.request("GET", "/hello?a=1&a=2", b"", "text/plain");
    assert_eq!(status, 200);
    assert_eq!(headers.to_lowercase().matches("set-cookie:").count(), 2);
    let first: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(first["path"], "/hello");
    assert_eq!(first["query"], serde_json::json!([["a", "1"], ["a", "2"]]));
    let second: serde_json::Value =
        serde_json::from_slice(&server.request("GET", "/", b"", "text/plain").2).unwrap();
    assert_eq!(first["pid"], second["pid"]);
    let redirect = server.request("POST", "/redirect", b"", "text/plain");
    assert_eq!(redirect.0, 303);
    assert!(redirect.1.to_lowercase().contains("location: /done"));
    assert!(redirect.2.is_empty());
    assert_eq!(
        server
            .request("POST", "/binary", b"\xff\x00", "application/octet-stream")
            .2,
        b"\xff\x00"
    );
    assert_eq!(
        server
            .request("POST", "/post", br#"{"n":1}"#, "application/json")
            .2,
        br#"{"n":1}"#
    );
    assert_eq!(
        server
            .request(
                "POST",
                "/form",
                b"a=1&a=2",
                "application/x-www-form-urlencoded"
            )
            .2,
        br#"[["a","1"],["a","2"]]"#
    );
    assert_eq!(server.request("HEAD", "/", b"", "text/plain").2, b"");
    assert!(
        server
            .request("GET", "/empty", b"", "text/plain")
            .2
            .is_empty()
    );
    assert_eq!(server.request("GET", "/mutate", b"", "text/plain").0, 500); // environment mutation is disabled
    assert_eq!(server.request("GET", "/leak", b"", "text/plain").0, 500);
    let final_response: serde_json::Value =
        serde_json::from_slice(&server.request("GET", "/", b"", "text/plain").2).unwrap();
    assert!(final_response["env"].is_null());
    assert_eq!(first["pid"], final_response["pid"]);
    // Entry code is a startup snapshot, even if its file changes underneath the worker.
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"changed\";",
    )
    .unwrap();
    assert_eq!(server.request("GET", "/", b"", "text/plain").0, 200);
}
#[test]
fn rejects_bad_requests_and_exact_limits_without_partial_error_output() {
    let server = Server::start(
        r#"<?scripta $r=Web::request(); if ($r->path === "/error") {echo "private partial output";echo $undefined;} elseif ($r->path === "/big") {echo "123456789";} elseif ($r->path === "/caught") {try {Web::write("123456789");} catch (Error $e) {}} elseif ($r->path === "/exact") {echo "12345678";} else {Web::write($r->bytes());}"#,
        &["--max-body-bytes", "4", "--max-response-bytes", "8"],
    );
    assert_eq!(
        server.request("POST", "/", b"1234", "text/plain").2,
        b"1234"
    );
    assert_eq!(server.request("POST", "/", b"12345", "text/plain").0, 413);
    for path in ["/error", "/big", "/caught"] {
        let response = server.request("GET", path, b"", "text/plain");
        assert_eq!(response.0, 500);
        assert_eq!(response.2, b"Internal Server Error\n");
    }
    assert_eq!(
        server.request("GET", "/exact", b"", "text/plain").2,
        b"12345678"
    );
    assert_eq!(
        parse(&server.raw(b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n")).0,
        400
    );
    let chunked=b"POST / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n123\r\n2\r\n45\r\n0\r\n\r\n";
    assert_eq!(parse(&server.raw(chunked)).0, 413);
    let bad=b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx";
    assert_eq!(parse(&server.raw(bad)).0, 400);
    assert_eq!(server.request("GET", "/", b"", "text/plain").0, 200);
}
#[test]
fn deadline_kills_blocked_worker_and_replacement_uses_same_snapshot() {
    let server = Server::start(
        r#"<?scripta $r=Web::request(); if ($r->path === "/slow") {Date::sleep(5000);} echo str(Process::currentId());"#,
        &["--timeout-ms", "750"],
    );
    let first = server.request("GET", "/", b"", "text/plain");
    assert_eq!(first.0, 200);
    let start = Instant::now();
    assert_eq!(server.request("GET", "/slow", b"", "text/plain").0, 504);
    assert!(start.elapsed() < Duration::from_secs(3));
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"changed\";",
    )
    .unwrap();
    let next = server.request("GET", "/", b"", "text/plain");
    assert_eq!(next.0, 200);
    assert_ne!(first.2, next.2);
    assert_ne!(next.2, b"changed");
}
#[test]
fn serve_help_and_invalid_configuration_are_clear() {
    let help = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .args(["serve", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--timeout-ms"));
    let source = tempfile::NamedTempFile::new().unwrap();
    fs::write(source.path(), "<?scripta echo \"ok\";").unwrap();
    for arguments in [
        vec!["--workers", "0"],
        vec!["--workers", "65"],
        vec!["--timeout-ms", "0"],
        vec!["--unknown", "1"],
        vec!["--port", "no"],
        vec!["--port", "0", "--port", "1"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg("serve")
            .arg(source.path())
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn separate_workers_run_concurrently_without_sharing_runtime_objects() {
    let server = Server::start(
        r#"<?scripta $r=Web::request(); $mine="";$other=""; if ($r->path === "/a") {$mine="a";$other="b";} else {$mine="b";$other="a";} File::writeAtomic($mine,"ready"); while (!File::exists($other)) {Date::sleep(10);} echo str(Process::currentId());"#,
        &["--workers", "2", "--timeout-ms", "3000"],
    );
    std::thread::scope(|scope| {
        let first = scope.spawn(|| server.request("GET", "/a", b"", "text/plain"));
        let second = scope.spawn(|| server.request("GET", "/b", b"", "text/plain"));
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert_eq!(first.0, 200);
        assert_eq!(second.0, 200);
        assert_ne!(first.2, second.2);
    });
}
#[test]
fn bounded_queue_rejects_overload_and_recovers() {
    let server = Server::start(
        "<?scripta Date::sleep(300); echo \"ok\";",
        &["--queue", "1", "--timeout-ms", "2000"],
    );
    let mut streams = vec![];
    for _ in 0..12 {
        let mut stream = TcpStream::connect(server.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        streams.push(stream);
    }
    let mut statuses = vec![];
    for mut stream in streams {
        let mut output = vec![];
        stream.read_to_end(&mut output).unwrap();
        statuses.push(parse(&output).0);
    }
    assert!(statuses.contains(&503), "{statuses:?}");
    assert!(statuses.contains(&200), "{statuses:?}");
    assert!(statuses.iter().all(|s| [200, 503].contains(s)));
    assert_eq!(server.request("GET", "/", b"", "text/plain").0, 200);
}
#[cfg(unix)]
#[test]
fn termination_drains_an_active_request_before_exiting() {
    let mut server = Server::start(
        "<?scripta File::writeAtomic(\"started\",\"yes\"); Date::sleep(300); echo \"finished\";",
        &["--timeout-ms", "1500"],
    );
    let address = server.address;
    let request = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut output = vec![];
        stream.read_to_end(&mut output).unwrap();
        parse(&output)
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !server.directory.path().join("started").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        Command::new("kill")
            .args(["-TERM", &server.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let response = request.join().unwrap();
    assert_eq!(response.0, 200);
    assert_eq!(response.2, b"finished");
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "server failed to drain");
        thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(feature = "sqlite")]
#[test]
fn shipped_web_example_runs_html_json_redirects_and_sqlite() {
    let source = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/web-app.scripta"
    ))
    .unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    let server = Server::start(&source, &[]);
    let home = server.request("GET", "/", b"", "text/plain");
    assert_eq!(home.0, 200);
    assert!(
        String::from_utf8(home.2)
            .unwrap()
            .contains("Scripta is serving HTTP")
    );
    let status: serde_json::Value =
        serde_json::from_slice(&server.request("GET", "/api/status", b"", "text/plain").2).unwrap();
    assert_eq!(status["status"], "ok");
    assert_eq!(server.request("GET", "/redirect", b"", "text/plain").0, 302);
    assert_eq!(
        server
            .request(
                "POST",
                "/api/echo",
                br#"{"hello":"world"}"#,
                "application/json"
            )
            .2,
        b"{\"hello\":\"world\"}\n"
    );
    assert_eq!(
        server
            .request("POST", "/api/echo", b"{", "application/json")
            .0,
        400
    );
    for count in [1, 2] {
        let response = server.request("POST", "/api/visits", b"", "text/plain");
        assert_eq!(response.0, 200, "{}", String::from_utf8_lossy(&response.2));
        let body: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
        assert_eq!(body["total"], count);
    }
    assert!(server.directory.path().join("web-demo.sqlite").exists());
    assert_eq!(server.request("GET", "/missing", b"", "text/plain").0, 404);
}

#[cfg(unix)]
#[test]
fn workers_exit_when_their_supervisor_dies_during_native_work() {
    let mut server = Server::start(
        "<?scripta File::writeAtomic(\"worker-pid\",str(Process::currentId())); Date::sleep(5000); echo \"done\";",
        &[],
    );
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let pidfile = server.directory.path().join("worker-pid");
    while !pidfile.exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let pid = fs::read_to_string(pidfile).unwrap();
    assert!(pid.parse::<u32>().unwrap() > 0);
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let alive = Command::new("kill")
            .args(["-0", &pid])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        if Instant::now() >= deadline {
            let _ = Command::new("kill").args(["-KILL", &pid]).status();
            panic!("worker outlived its supervisor while blocked in native code");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

impl Server {
    fn session_request(&self, path: &str, id: Option<&str>) -> (u16, String, Vec<u8>) {
        let cookie = id
            .map(|id| format!("Cookie: SCRIPTA_SESSION={id}\r\n"))
            .unwrap_or_default();
        parse(
            &self.raw(
                format!(
                    "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{cookie}\r\n"
                )
                .as_bytes(),
            ),
        )
    }
}
fn response_session_id(headers: &str) -> String {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, value)| {
            key.eq_ignore_ascii_case("set-cookie") && value.trim().starts_with("SCRIPTA_SESSION=")
        })
        .unwrap()
        .1
        .trim()
        .strip_prefix("SCRIPTA_SESSION=")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}
const SESSION_APP: &str = r#"<?scripta
$r=Web::request();
if ($r->path === "/cookies") {Web::header("Content-Type","application/json");Web::cookie("custom","hello 世界",["secure"=>false]);echo Json::encode($r->cookies());exit 0;}
Session::start();
if ($r->path === "/rotate") {Session::regenerate();}
if ($r->path === "/destroy") {Session::destroy();echo "destroyed";exit 0;}
if ($r->path === "/error") {Session::set("count",999);echo $missing;}
if ($r->path === "/slow") {Session::set("count",999);Date::sleep(5000);}
if ($r->path === "/increment") {$n=Session::get("count",0);Date::sleep(25);Session::set("count",$n+1);}
Web::header("Content-Type","application/json");echo Json::encode(["count"=>Session::get("count",0),"pid"=>Process::currentId()]);
"#;
#[test]
fn cookie_and_session_requests_share_data_across_workers_and_revoke_rotated_ids() {
    let server = Server::start(SESSION_APP, &["--workers", "2"]);
    let first = server.session_request("/increment", None);
    assert_eq!(first.0, 200, "{}", String::from_utf8_lossy(&first.2));
    let id = response_session_id(&first.1);
    assert!(first.1.contains("Secure; HttpOnly; SameSite=Lax"));
    assert!(first.1.to_lowercase().contains("cache-control: no-store"));
    let count = |reply: &(u16, String, Vec<u8>)| -> i64 {
        serde_json::from_slice::<serde_json::Value>(&reply.2).unwrap()["count"]
            .as_i64()
            .unwrap()
    };
    assert_eq!(count(&first), 1);
    std::thread::scope(|scope| {
        let requests: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| server.session_request("/increment", Some(&id))))
            .collect();
        let mut counts = vec![];
        let mut pids = std::collections::BTreeSet::new();
        for request in requests {
            let reply = request.join().unwrap();
            assert_eq!(reply.0, 200);
            counts.push(count(&reply));
            let json: serde_json::Value = serde_json::from_slice(&reply.2).unwrap();
            pids.insert(json["pid"].as_i64().unwrap());
        }
        counts.sort();
        assert_eq!(counts, vec![2, 3, 4, 5]);
        assert_eq!(pids.len(), 2);
    });
    let rotated = server.session_request("/rotate", Some(&id));
    assert_eq!(rotated.0, 200);
    assert_eq!(count(&rotated), 5);
    let next = response_session_id(&rotated.1);
    assert_ne!(id, next);
    let old = server.session_request("/", Some(&id));
    assert_eq!(count(&old), 0);
    assert_ne!(response_session_id(&old.1), id);
    let destroyed = server.session_request("/destroy", Some(&next));
    assert_eq!(destroyed.0, 200);
    assert!(destroyed.1.contains("Max-Age=0"));
    assert_eq!(count(&server.session_request("/", Some(&next))), 0);
    let cookies=parse(&server.raw(b"GET /cookies HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nCookie: custom=hello%20world; custom=second\r\n\r\n"));
    assert_eq!(cookies.0, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&cookies.2).unwrap(),
        serde_json::json!([["custom", "hello world"], ["custom", "second"]])
    );
    assert!(cookies.1.contains("custom=hello%20%E4%B8%96%E7%95%8C"));
}
#[test]
fn session_worker_timeout_rolls_back_data_and_releases_file_locks() {
    let server = Server::start(SESSION_APP, &["--timeout-ms", "750"]);
    let first = server.session_request("/increment", None);
    assert_eq!(first.0, 200);
    let id = response_session_id(&first.1);
    assert_eq!(server.session_request("/error", Some(&id)).0, 500);
    let error = server.session_request("/slow", Some(&id));
    assert_eq!(error.0, 504);
    assert!(!error.1.to_lowercase().contains("set-cookie"));
    let next = server.session_request("/increment", Some(&id));
    assert_eq!(next.0, 200);
    let json: serde_json::Value = serde_json::from_slice(&next.2).unwrap();
    assert_eq!(json["count"], 2);
    assert_eq!(response_session_id(&next.1), id);
}
#[test]
fn configured_session_directory_survives_host_restart_and_cookie_security_is_explicit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("sessions");
    let path = root.to_str().unwrap();
    let first = Server::start(
        SESSION_APP,
        &["--session-dir", path, "--session-cookie-secure", "false"],
    );
    let reply = first.session_request("/increment", None);
    assert_eq!(reply.0, 200);
    assert!(!reply.1.contains("; Secure"));
    let id = response_session_id(&reply.1);
    drop(first);
    let second = Server::start(SESSION_APP, &["--session-dir", path]);
    let reply = second.session_request("/increment", Some(&id));
    assert_eq!(reply.0, 200);
    let json: serde_json::Value = serde_json::from_slice(&reply.2).unwrap();
    assert_eq!(json["count"], 2);
    assert_eq!(response_session_id(&reply.1), id);
}
#[test]
fn session_idle_expiry_and_quota_are_enforced_by_host_options() {
    let server = Server::start(
        SESSION_APP,
        &["--session-ttl-seconds", "1", "--session-max-records", "1"],
    );
    let first = server.session_request("/increment", None);
    assert_eq!(first.0, 200);
    assert!(first.1.contains("Max-Age=1"));
    let id = response_session_id(&first.1);
    assert_eq!(server.session_request("/", None).0, 500);
    let rotated = server.session_request("/rotate", Some(&id));
    assert_eq!(rotated.0, 200);
    let id = response_session_id(&rotated.1);
    thread::sleep(Duration::from_millis(1200));
    let replacement = server.session_request("/", Some(&id));
    assert_eq!(replacement.0, 200);
    assert_ne!(response_session_id(&replacement.1), id);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&replacement.2).unwrap()["count"],
        0
    );
}
