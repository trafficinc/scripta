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
        Self::start_config(source, None, options)
    }
    fn start_config(source: &str, config: Option<&str>, options: &[&str]) -> Self {
        Self::start_files(source, config, options, &[])
    }
    fn start_files(
        source: &str,
        config: Option<&str>,
        options: &[&str],
        files: &[(&str, &str)],
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.scripta");
        fs::write(&path, source).unwrap();
        for (name, text) in files {
            let path = directory.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        if let Some(config) = config {
            fs::write(directory.path().join("server.toml"), config).unwrap();
        }
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
            .args(if config.is_some() {
                vec!["--config", "server.toml"]
            } else {
                vec![]
            })
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
            if let Some(address) = log
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .find(|event| event["event"] == "server_listening")
                .and_then(|event| event["address"].as_str().map(str::to_owned))
            {
                server.address = address.parse().unwrap();
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

#[test]
fn shipped_session_example_counts_rotates_and_logs_out_with_real_requests() {
    let source = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/web-sessions.scripta"
    ))
    .unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    let server = Server::start(&source, &["--session-cookie-secure", "false"]);
    let first = server.session_request("/", None);
    assert_eq!(first.0, 200);
    let id = response_session_id(&first.1);
    let next = server.session_request("/", Some(&id));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&next.2).unwrap()["visits"],
        2
    );
    let post = |path: &str, id: &str, body: &str| {
        parse(&server.raw(format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nCookie: SCRIPTA_SESSION={id}\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()))
    };
    let preference = post("/theme", &id, r#"{"theme":"light"}"#);
    assert_eq!(preference.0, 200);
    assert!(preference.1.contains("theme=light"));
    let rotated = post("/rotate", &id, "");
    assert_eq!(rotated.0, 303);
    let rotated_id = response_session_id(&rotated.1);
    assert_ne!(rotated_id, id);
    let current = server.session_request("/", Some(&rotated_id));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&current.2).unwrap()["visits"],
        3
    );
    let logged_out = post("/logout", &rotated_id, "");
    assert_eq!(logged_out.0, 303);
    assert!(logged_out.1.contains("SCRIPTA_SESSION=; Path=/; Max-Age=0"));
    assert!(logged_out.1.contains("theme=; Path=/; Max-Age=0"));
    let restarted = server.session_request("/", Some(&rotated_id));
    assert_ne!(response_session_id(&restarted.1), rotated_id);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&restarted.2).unwrap()["visits"],
        1
    );
}

#[test]
fn password_verification_can_authenticate_a_web_session_without_extra_capabilities() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/passwords/php-argon2id.json"
    ))
    .unwrap();
    let hash = fixtures["fixtures"][0]["hash"].as_str().unwrap();
    let source=r#"<?scripta
$r=Web::request();$form=$r->json();$valid=Password::verify($form["password"],"__FIXTURE_HASH__");
if ($valid) {Session::start();Session::regenerate();Session::set("userId",42);} else {Web::status(401);}
Web::header("Content-Type","application/json");echo Json::encode(["verified"=>$valid]);
"#.replace("__FIXTURE_HASH__",hash);
    let server = Server::start(&source, &[]);
    let correct = server.request(
        "POST",
        "/login",
        br#"{"password":"public-fixture-password"}"#,
        "application/json",
    );
    assert_eq!(correct.0, 200);
    assert_eq!(correct.2, br#"{"verified":true}"#);
    assert_eq!(response_session_id(&correct.1).len(), 64);
    let wrong = server.request(
        "POST",
        "/login",
        br#"{"password":"wrong"}"#,
        "application/json",
    );
    assert_eq!(wrong.0, 401);
    assert_eq!(wrong.2, br#"{"verified":false}"#);
    assert!(!wrong.1.to_lowercase().contains("set-cookie:"));
}

type FormPart<'a> = (&'a str, Option<&'a str>, Option<&'a str>, &'a [u8]);
fn multipart(parts: &[FormPart<'_>]) -> Vec<u8> {
    let mut body = vec![];
    for (name, filename, media, value) in parts {
        body.extend_from_slice(
            format!("--upload-boundary\r\nContent-Disposition: form-data; name=\"{name}\"")
                .as_bytes(),
        );
        if let Some(filename) = filename {
            body.extend_from_slice(format!("; filename=\"{filename}\"").as_bytes())
        }
        body.extend_from_slice(b"\r\n");
        if let Some(media) = media {
            body.extend_from_slice(format!("Content-Type: {media}\r\n").as_bytes())
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--upload-boundary--\r\n");
    body
}
const MULTIPART: &str = "multipart/form-data; boundary=upload-boundary";
fn uploads_clean(server: &Server) {
    let root = server.directory.path().join("spools");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if fs::read_dir(&root).unwrap().next().is_none() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "upload spool leaked: {:?}",
            fs::read_dir(root)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn multipart_uploads_preserve_binary_duplicates_metadata_and_explicit_destinations() {
    let server = Server::start(
        r#"<?scripta
$r=Web::request();$files=$r->uploads();
if ($r->path === "/raw") {Web::write($r->bytes());exit 0;}
if ($r->path === "/text") {echo $r->text();exit 0;}
if ($r->path === "/save") {$files[0]->moveTo("chosen.bin");echo Json::encode($files[0]->moved);exit 0;}
$items=[];foreach ($files as $file) {$items=List::push($items,[$file->fieldName,$file->filename,$file->mediaType,$file->size,$file->moved]);}
echo Json::encode(["fields"=>$r->form(),"files"=>$items]);"#,
        &["--upload-temp-dir", "spools"],
    );
    let body = multipart(&[
        ("title", None, None, "café".as_bytes()),
        ("f", Some("../../é.bin"), Some("image/png"), b"\x00\xffabc"),
        ("title", None, None, b"second"),
        ("f", Some("empty.bin"), None, b""),
        ("f", Some(""), Some("application/octet-stream"), b""),
    ]);
    let response = server.request("POST", "/", &body, MULTIPART);
    assert_eq!(response.0, 200);
    let data: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(
        data,
        serde_json::json!({"fields":[["title","café"],["title","second"]],"files":[["f","../../é.bin","image/png",5,false],["f","empty.bin",null,0,false]]})
    );
    assert_eq!(server.request("POST", "/raw", &body, MULTIPART).2, body);
    assert_eq!(server.request("POST", "/text", &body, MULTIPART).0, 500);
    assert_eq!(server.request("POST", "/save", &body, MULTIPART).2, b"true");
    assert_eq!(
        fs::read(server.directory.path().join("chosen.bin")).unwrap(),
        b"\0\xffabc"
    );
    assert_eq!(server.request("POST", "/save", &body, MULTIPART).0, 500);
    assert_eq!(
        fs::read(server.directory.path().join("chosen.bin")).unwrap(),
        b"\0\xffabc"
    );
    uploads_clean(&server);
    assert!(!server.directory.path().join("é.bin").exists());
    assert!(!fs::read_dir(server.directory.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".scripta-upload-")
    }));
}
#[test]
fn multipart_rejects_malformed_encodings_headers_boundaries_and_nested_forms() {
    let server = Server::start("<?scripta echo \"ok\";", &["--upload-temp-dir", "spools"]);
    for (body, kind, expected) in [
        (b"broken".to_vec(), MULTIPART, 400),
        (multipart(&[("a", None, None, b"\xff")]), MULTIPART, 400),
        (multipart(&[("a", Some(""), None, b"x")]), MULTIPART, 400),
        (
            multipart(&[("a", None, Some("multipart/mixed; boundary=x"), b"nested")]),
            MULTIPART,
            415,
        ),
        (
            multipart(&[("a", None, Some("text/plain; charset=latin1"), b"x")]),
            MULTIPART,
            415,
        ),
        (
            multipart(&[("a", None, None, b"x")]),
            "multipart/form-data",
            400,
        ),
        (
            multipart(&[("a", None, None, b"x")]),
            "multipart/mixed; boundary=x",
            415,
        ),
        (
            multipart(&[("a", None, None, b"x")]),
            "multipart/form-data; boundary=\"bad;boundary\"",
            400,
        ),
    ] {
        assert_eq!(server.request("POST", "/", &body, kind).0, expected);
        uploads_clean(&server)
    }
    let mut body = multipart(&[("a", Some("x"), None, b"content")]);
    body.truncate(body.len() - 25);
    assert_eq!(server.request("POST", "/", &body, MULTIPART).0, 400);
    uploads_clean(&server);
    let header = format!(
        "--upload-boundary\r\nContent-Disposition: form-data; name=\"a\"\r\nX-Pad: {}",
        "x".repeat(60000)
    );
    assert_eq!(
        server.request("POST", "/", header.as_bytes(), MULTIPART).0,
        413
    );
    uploads_clean(&server);
}
#[test]
fn multipart_exact_limits_and_limit_plus_one_and_disk_recovery() {
    let field = multipart(&[("a", None, None, b"1234")]);
    let file = multipart(&[("f", Some("x"), None, b"1234")]);
    for (options, accepted, rejected) in [
        (
            vec!["--upload-max-file-bytes", "4"],
            file.clone(),
            multipart(&[("f", Some("x"), None, b"12345")]),
        ),
        (
            vec!["--upload-max-files", "1"],
            file.clone(),
            multipart(&[("f", Some("x"), None, b""), ("f", Some("y"), None, b"")]),
        ),
        (
            vec!["--upload-max-fields", "1"],
            field.clone(),
            multipart(&[("a", None, None, b""), ("b", None, None, b"")]),
        ),
        (
            vec!["--upload-max-field-bytes", "4"],
            field.clone(),
            multipart(&[("a", None, None, b"12345")]),
        ),
        (
            vec!["--upload-max-fields-bytes", "5"],
            field.clone(),
            multipart(&[("a", None, None, b"12345")]),
        ),
    ] {
        let mut options = options;
        options.extend(["--upload-temp-dir", "spools"]);
        let server = Server::start("<?scripta echo \"ok\";", &options);
        assert_eq!(server.request("POST", "/", &accepted, MULTIPART).0, 200);
        assert_eq!(server.request("POST", "/", &rejected, MULTIPART).0, 413);
        uploads_clean(&server);
        assert_eq!(server.request("POST", "/", &accepted, MULTIPART).0, 200);
        uploads_clean(&server);
    }
    let limit = (file.len() + 4).to_string();
    let server = Server::start(
        "<?scripta echo \"ok\";",
        &[
            "--upload-temp-dir",
            "spools",
            "--upload-max-disk-bytes",
            &limit,
        ],
    );
    assert_eq!(server.request("POST", "/", &file, MULTIPART).0, 200);
    uploads_clean(&server);
    let too_big = multipart(&[("f", Some("x"), None, b"12345")]);
    assert_eq!(server.request("POST", "/", &too_big, MULTIPART).0, 503);
    uploads_clean(&server);
    assert_eq!(server.request("POST", "/", &file, MULTIPART).0, 200);
}
#[test]
fn multipart_large_files_are_spooled_and_worker_timeout_cleans_all_storage() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();$f=$r->uploads();if ($r->path === "/hang") {Date::sleep(30000);}echo str($f[0]->size);"#,
        &[
            "--upload-temp-dir",
            "spools",
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--timeout-ms",
            "2000",
            "--max-body-bytes",
            "4194304",
        ],
    );
    let bytes = vec![0xa5; 2 * 1024 * 1024];
    let body = multipart(&[("f", Some("large.bin"), None, &bytes)]);
    assert_eq!(server.request("POST", "/", &body, MULTIPART).2, b"2097152");
    uploads_clean(&server);
    assert_eq!(server.request("POST", "/hang", &body, MULTIPART).0, 504);
    uploads_clean(&server);
    // Worker recovery is proactive, but can outlast a short request deadline on
    // busy native runners. Wait for replacement instead of spending the next
    // upload's deadline in the recovery queue. Do not retry the upload itself.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let metrics: serde_json::Value =
            serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
        if metrics["ready"] == true
            && metrics["readyWorkers"] == 1
            && metrics["workerRestarts"] == 1
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not recover: {metrics}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let recovered = server.request("POST", "/", &body, MULTIPART);
    assert_eq!(recovered.0, 200);
    assert_eq!(recovered.2, b"2097152");
    uploads_clean(&server);
    // Disconnect during a partial body must release its raw spool and queue slot.
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream.write_all(format!("POST / HTTP/1.1\r\nHost: localhost\r\nContent-Type: {MULTIPART}\r\nContent-Length: 100000\r\n\r\npartial").as_bytes()).unwrap();
    drop(stream);
    thread::sleep(Duration::from_millis(50));
    uploads_clean(&server);
}

#[test]
fn shipped_upload_example_runs_a_browser_form_and_escapes_submitted_metadata() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/web-uploads.scripta"),
    )
    .unwrap();
    let server = Server::start(&source, &["--upload-temp-dir", "spools"]);
    let get = server.request("GET", "/", b"", "text/plain");
    assert_eq!(get.0, 200);
    assert!(String::from_utf8_lossy(&get.2).contains("enctype=\"multipart/form-data\""));
    let body = multipart(&[
        ("note", None, None, b"<script>"),
        ("file", Some("<image>.bin"), None, b"\xff\0"),
    ]);
    let post = server.request("POST", "/", &body, MULTIPART);
    assert_eq!(post.0, 200);
    let html = String::from_utf8(post.2).unwrap();
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("&lt;image&gt;.bin"));
    assert_eq!(
        fs::read(server.directory.path().join("received-upload.bin")).unwrap(),
        b"\xff\0"
    );
    uploads_clean(&server);
}
#[test]
fn upload_stage_disk_limit_preserves_destination_and_recovers() {
    let body = multipart(&[("f", Some("file"), None, b"1234")]);
    let limit = (body.len() + 4).to_string();
    let server = Server::start(
        r#"<?scripta $r=Web::request();if ($r->path === "/save") {$files=$r->uploads();$files[0]->moveTo("result");}echo "ok";"#,
        &[
            "--upload-temp-dir",
            "spools",
            "--upload-max-disk-bytes",
            &limit,
        ],
    );
    assert_eq!(server.request("POST", "/save", &body, MULTIPART).0, 500);
    uploads_clean(&server);
    assert!(!server.directory.path().join("result").exists());
    assert!(!fs::read_dir(server.directory.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".scripta-upload-")
    }));
    assert_eq!(server.request("POST", "/", &body, MULTIPART).0, 200);
    uploads_clean(&server);
}

#[test]
fn multipart_normalized_header_limit_is_exact() {
    let body = multipart(&[("a", None, None, b"x")]);
    let normalized = "content-disposition".len() + "form-data; name=\"a\"".len() + 4;
    for (limit, status) in [(normalized, 200), (normalized - 1, 413)] {
        let limit = limit.to_string();
        let server = Server::start(
            "<?scripta echo \"ok\";",
            &[
                "--upload-temp-dir",
                "spools",
                "--upload-max-header-bytes",
                &limit,
            ],
        );
        assert_eq!(server.request("POST", "/", &body, MULTIPART).0, status);
        uploads_clean(&server);
    }
}

struct StreamingResponse {
    reader: std::io::BufReader<TcpStream>,
    headers: String,
}
impl StreamingResponse {
    fn start(server: &Server, path: &str) -> Self {
        Self::request(server, "GET", path, b"", "text/plain")
    }
    fn request(server: &Server, method: &str, path: &str, body: &[u8], content_type: &str) -> Self {
        use std::io::BufRead;
        let mut stream = TcpStream::connect(server.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        stream
            .write_all(
                format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",body.len())
                    .as_bytes(),
            )
            .unwrap();
        stream.write_all(body).unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line)
        }
        Self { reader, headers }
    }
    fn chunk(&mut self) -> Vec<u8> {
        use std::io::BufRead;
        let mut line = String::new();
        assert_ne!(self.reader.read_line(&mut line).unwrap(), 0);
        let size = usize::from_str_radix(line.trim(), 16).unwrap();
        let mut result = vec![0; size];
        self.reader.read_exact(&mut result).unwrap();
        let mut crlf = [0; 2];
        self.reader.read_exact(&mut crlf).unwrap();
        assert_eq!(crlf, b"\r\n"[0..2]);
        result
    }
}
#[test]
fn echo_and_flush_deliver_before_execution_finishes() {
    let server = Server::start(
        r#"<?scripta
Web::header("Content-Type","application/octet-stream");
echo "first";flush();
while (!File::exists("continue")) {Date::sleep(5);}
echo "second";flush();"#,
        &[],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert!(response.headers.starts_with("HTTP/1.1 200"));
    assert!(
        response
            .headers
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
    );
    assert_eq!(response.chunk(), b"first");
    assert!(!server.directory.path().join("continue").exists());
    fs::write(server.directory.path().join("continue"), b"yes").unwrap();
    assert_eq!(response.chunk(), b"second");
    assert!(response.chunk().is_empty());
}
#[test]
fn transport_flush_commits_empty_headers_before_body_output() {
    let server = Server::start(
        r#"<?scripta function content() {return "outerinner";} $content=content();flush();
while (!File::exists("continue")) {Date::sleep(5);}
echo $content;"#,
        &[],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert!(response.headers.starts_with("HTTP/1.1 200"));
    fs::write(server.directory.path().join("continue"), b"yes").unwrap();
    assert_eq!(response.chunk(), b"outerinner");
    assert!(response.chunk().is_empty());
}
#[test]
fn streaming_session_is_saved_and_unlocked_and_header_mutations_are_rejected() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();
if ($r->path === "/read") {Session::start();echo str(Session::get("n"));exit 0;}
Session::start();Session::set("n",42);echo "first";flush();
while (!File::exists("continue")) {Date::sleep(5);}
$blocked=0;try {Session::set("n",99);} catch ($e) {$blocked+=1;}
try {Session::start();} catch ($e) {$blocked+=1;}
try {Web::header("X-Late","bad");} catch ($e) {$blocked+=1;}
try {Web::status(201);} catch ($e) {$blocked+=1;}
try {Web::redirect("/elsewhere");} catch ($e) {$blocked+=1;}
try {Web::cookie("late","bad");} catch ($e) {$blocked+=1;}
try {Web::deleteCookie("late");} catch ($e) {$blocked+=1;}
echo str($blocked);"#,
        &["--workers", "2"],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert_eq!(response.chunk(), b"first");
    let id = response_session_id(&response.headers);
    assert_eq!(server.session_request("/read", Some(&id)).2, b"42");
    fs::write(server.directory.path().join("continue"), b"yes").unwrap();
    assert_eq!(response.chunk(), b"7");
    assert!(response.chunk().is_empty());
}
#[test]
fn flush_without_active_session_prevents_late_session_creation() {
    let server = Server::start(
        r#"<?scripta flush();try {Session::start();} catch ($e) {echo "closed";}"#,
        &[],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert!(!response.headers.to_lowercase().contains("set-cookie:"));
    assert_eq!(response.chunk(), b"closed");
    assert!(response.chunk().is_empty());
}
#[test]
fn streaming_errors_and_deadlines_end_the_body_without_replacing_the_status() {
    for (ending, options) in [
        ("throw \"failed\";", vec![]),
        ("Date::sleep(5000);", vec!["--timeout-ms", "150"]),
    ] {
        let server = Server::start(
            &format!("<?scripta Web::status(202);echo \"sent\";flush();{ending}"),
            &options,
        );
        let mut response = StreamingResponse::start(&server, "/");
        assert!(response.headers.starts_with("HTTP/1.1 202"));
        assert_eq!(response.chunk(), b"sent");
        let mut tail = vec![];
        let _ = response.reader.read_to_end(&mut tail);
        assert!(!tail.windows(3).any(|w| w == b"500"));
        assert!(!tail.ends_with(b"0\r\n\r\n"));
    }
    let server = Server::start("<?scripta echo \"private\";throw \"failed\";", &[]);
    let failure = server.request("GET", "/", b"", "text/plain");
    assert_eq!(failure.0, 500);
    assert_eq!(failure.2, b"Internal Server Error\n");
}
#[test]
fn cumulative_output_limit_applies_with_and_without_flushing() {
    let server = Server::start(
        "<?scripta echo \"1234\";flush();echo \"x\";",
        &["--max-response-bytes", "4"],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert_eq!(response.chunk(), b"1234");
    let mut tail = vec![];
    let _ = response.reader.read_to_end(&mut tail);
    assert!(!tail.ends_with(b"0\r\n\r\n"));
    let server = Server::start(
        "<?scripta echo \"1234\";echo \"x\";",
        &["--max-response-bytes", "4"],
    );
    assert_eq!(server.request("GET", "/", b"", "text/plain").0, 500);
}
#[test]
fn dropping_streaming_client_stops_native_work_and_releases_upload_spools() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();if ($r->path === "/ok") {echo "ok";exit 0;}echo "sent";flush();File::writeAtomic("running","yes");Date::sleep(5000);"#,
        &["--upload-temp-dir", "spools", "--timeout-ms", "3000"],
    );
    let body = multipart(&[("file", Some("file.bin"), None, b"\0\xff")]);
    let mut response = StreamingResponse::request(&server, "POST", "/", &body, MULTIPART);
    assert_eq!(response.chunk(), b"sent");
    assert!(
        fs::read_dir(server.directory.path().join("spools"))
            .unwrap()
            .next()
            .is_some()
    );
    drop(response);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let log = fs::read_to_string(server.directory.path().join("server.log")).unwrap();
        if log.contains("replacing worker") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "disconnected worker not stopped: {log}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.request("GET", "/ok", b"", "text/plain").2, b"ok");
    uploads_clean(&server);
}
#[test]
fn slow_stream_reader_cannot_hold_the_only_worker_beyond_its_deadline() {
    let chunk = "x".repeat(65536);
    let source = format!(
        "<?scripta $r=Web::request();if ($r->path === \"/ok\") {{echo \"ok\";exit 0;}} $chunk=\"{chunk}\";for ($i=0;$i<512;$i+=1) {{echo $chunk;flush();}}"
    );
    let server = Server::start(
        &source,
        &["--max-response-bytes", "33554432", "--timeout-ms", "300"],
    );
    let response = StreamingResponse::start(&server, "/");
    // Keep the first connection open without draining its body. The replacement
    // worker must serve another connection after the first request's deadline.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let log = fs::read_to_string(server.directory.path().join("server.log")).unwrap();
        if log.contains("ending response and replacing worker") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "slow reader did not hit its request deadline"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.request("GET", "/ok", b"", "text/plain").2, b"ok");
    drop(response);
}
#[test]
fn shipped_streaming_example_delivers_every_progress_chunk() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/web-streaming.scripta"),
    )
    .unwrap();
    let server = Server::start(&source, &[]);
    let mut response = StreamingResponse::start(&server, "/");
    assert_eq!(response.chunk(), b"Starting\n");
    for step in 1..=3 {
        assert_eq!(response.chunk(), format!("Step {step}\n").as_bytes())
    }
    assert_eq!(response.chunk(), b"Finished\n");
    assert!(response.chunk().is_empty());
}
#[test]
fn streaming_binary_head_and_bodyless_statuses_preserve_http_semantics() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();if ($r->path === "/empty") {Web::status(204);}Web::header("Content-Type","application/octet-stream");Web::write(Bytes::slice(Bytes::encodeUtf8("é"),1));flush();File::writeAtomic("finished","yes");"#,
        &[],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert_eq!(response.chunk(), [0xa9]);
    assert!(response.chunk().is_empty());
    let empty = server.request("GET", "/empty", b"", "text/plain");
    assert_eq!(empty.0, 204);
    assert!(empty.2.is_empty());
    let head = server.request("HEAD", "/", b"", "text/plain");
    assert_eq!(head.0, 200);
    assert!(head.2.is_empty());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !server.directory.path().join("finished").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn streamed_upload_can_save_through_the_same_worker_control_channel() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();$files=$r->uploads();echo "first";flush();$files[0]->moveTo("saved.bin");echo "saved";"#,
        &["--upload-temp-dir", "spools"],
    );
    let body = multipart(&[("file", Some("../../untrusted.bin"), None, b"\xff\0")]);
    let mut response = StreamingResponse::request(&server, "POST", "/", &body, MULTIPART);
    assert_eq!(response.chunk(), b"first");
    assert_eq!(response.chunk(), b"saved");
    assert!(response.chunk().is_empty());
    assert_eq!(
        fs::read(server.directory.path().join("saved.bin")).unwrap(),
        b"\xff\0"
    );
    uploads_clean(&server);
}
#[test]
fn raw_request_streams_support_binary_apis_independent_cursors_and_host_cleanup() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();$input=$r->bodyStream();$out=Stream::open("saved.raw","x");$n=Stream::copy($input,$out,$r->bodySize);Stream::close($out);Stream::close($input);$second=$r->bodyStream();$first=Stream::read($second,1);Stream::close($second);echo Json::encode(["size"=>$r->bodySize,"copied"=>$n,"uploads"=>len($r->uploads()),"first"=>len($first)]);"#,
        &["--upload-temp-dir", "spools", "--max-body-bytes", "262144"],
    );
    let body = vec![0xff; 131072];
    let response = server.request("PUT", "/", &body, "application/octet-stream");
    assert_eq!(response.0, 200);
    let data: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(
        data,
        serde_json::json!({"size":131072,"copied":131072,"uploads":0,"first":1})
    );
    assert_eq!(
        fs::read(server.directory.path().join("saved.raw")).unwrap(),
        body
    );
    uploads_clean(&server);
}
#[test]
fn attachment_downloads_use_safe_headers_binary_chunks_and_precommit_validation() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();$file=Stream::open("payload.bin");if ($r->path === "/bad") {Web::download($file,"bad\r\nInjected: yes");}else {Web::download($file,"résumé.bin");}Stream::close($file);exit 0;"#,
        &[],
    );
    let data = vec![0xa5; 131072];
    fs::write(server.directory.path().join("payload.bin"), &data).unwrap();
    let mut response = StreamingResponse::start(&server, "/");
    let headers = response.headers.to_lowercase();
    assert!(headers.contains("content-type: application/octet-stream"));
    assert!(headers.contains("x-content-type-options: nosniff"));
    assert!(headers.contains("filename*=utf-8''r%c3%a9sum%c3%a9.bin"));
    let mut received = vec![];
    loop {
        let chunk = response.chunk();
        if chunk.is_empty() {
            break;
        }
        received.extend(chunk)
    }
    assert_eq!(received, data);
    let bad = server.request("GET", "/bad", b"", "text/plain");
    assert_eq!(bad.0, 500);
    assert!(!bad.1.to_lowercase().contains("injected:"));
    assert_eq!(bad.2, b"Internal Server Error\n");
    let head = server.request("HEAD", "/", b"", "text/plain");
    assert_eq!(head.0, 200);
    assert!(head.2.is_empty());
}
#[test]
fn known_oversized_downloads_fail_cleanly_before_headers_commit() {
    let server = Server::start(
        "<?scripta $file=Stream::open(\"large\");Web::download($file,\"large.bin\");",
        &["--max-response-bytes", "4"],
    );
    fs::write(server.directory.path().join("large"), b"12345").unwrap();
    let response = server.request("GET", "/", b"", "text/plain");
    assert_eq!(response.0, 500);
    assert_eq!(response.2, b"Internal Server Error\n");
    assert!(!response.1.to_lowercase().contains("content-disposition:"));
}
#[test]
fn shipped_web_transfer_example_roundtrips_raw_binary_and_refuses_overwrite() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/web-transfers.scripta"),
    )
    .unwrap();
    let server = Server::start(&source, &["--upload-temp-dir", "spools"]);
    let bytes = b"\0\xff\x01\x02";
    let response = server.request("PUT", "/raw", bytes, "application/octet-stream");
    assert_eq!(response.0, 201);
    assert_eq!(
        fs::read(server.directory.path().join("raw-upload.bin")).unwrap(),
        bytes
    );
    uploads_clean(&server);
    assert_eq!(
        server
            .request("PUT", "/raw", b"overwrite", "application/octet-stream")
            .0,
        500
    );
    assert_eq!(
        fs::read(server.directory.path().join("raw-upload.bin")).unwrap(),
        bytes
    );
    let mut download = StreamingResponse::start(&server, "/download");
    assert_eq!(download.chunk(), bytes);
    assert!(download.chunk().is_empty());
}
#[test]
fn rejected_upload_never_runs_application_and_request_workers_cannot_bind_listeners() {
    let server = Server::start(
        "<?scripta File::writeAtomic(\"ran\",\"yes\");echo \"ok\";",
        &[
            "--upload-max-file-bytes",
            "1",
            "--upload-temp-dir",
            "spools",
        ],
    );
    let body = multipart(&[("file", Some("x"), None, b"12")]);
    assert_eq!(server.request("POST", "/", &body, MULTIPART).0, 413);
    assert!(!server.directory.path().join("ran").exists());
    uploads_clean(&server);
    let server = Server::start("<?scripta Socket::listen(\"127.0.0.1\",0);", &[]);
    assert_eq!(server.request("GET", "/", b"", "text/plain").0, 500);
}

#[test]
fn configuration_file_controls_real_requests_and_cli_overrides_file_limits() {
    let config = include_str!("../../../examples/serve.toml");
    let server = Server::start_config(
        "<?scripta Web::write(Web::request()->bytes());",
        Some(config),
        &["--max-body-bytes", "3"],
    );
    assert_eq!(
        server
            .request("POST", "/", b"abc", "application/octet-stream")
            .2,
        b"abc"
    );
    assert_eq!(
        server
            .request("POST", "/", b"abcd", "application/octet-stream")
            .0,
        413
    );
}

#[test]
fn configured_connection_and_header_timeouts_close_connections() {
    let server = Server::start(
        "<?scripta echo \"ok\";",
        &[
            "--connection-timeout-ms",
            "300",
            "--header-timeout-ms",
            "100",
        ],
    );
    let start = Instant::now();
    let output = server.raw(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(parse(&output).2, b"ok");
    assert!(start.elapsed() < Duration::from_secs(3));
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(b"GET / HTTP/1.1\r\nHost:").unwrap();
    let start = Instant::now();
    let mut output = vec![];
    if let Err(error) = stream.read_to_end(&mut output) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(output.is_empty());
}

#[cfg(unix)]
#[test]
fn configured_drain_deadline_bounds_shutdown_with_active_work() {
    let mut server = Server::start(
        "<?scripta File::writeAtomic(\"started\",\"yes\"); Date::sleep(5000); echo \"late\";",
        &["--timeout-ms", "10000", "--drain-timeout-ms", "100"],
    );
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(3);
    while server.child.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "configured drain deadline was not applied"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let mut output = vec![];
    if let Err(error) = stream.read_to_end(&mut output) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    assert!(!output.windows(4).any(|w| w == b"late"));
}

const PROXY_APP: &str = r#"<?scripta
$r=Web::request();
if ($r->path === "/redirect") {Web::redirect($r->scheme . "://" . $r->host . "/login",303);exit 0;}
if ($r->path === "/cookie") {Session::start();Web::cookie("preference","ok");}
Web::header("Content-Type","application/json");
echo Json::encode(["peer"=>$r->peerAddress,"client"=>$r->clientAddress,"scheme"=>$r->scheme,"host"=>$r->host,"raw"=>$r->header("X-Forwarded-For")]);
"#;
fn proxy_request(server: &Server, host: &str, path: &str, fields: &str) -> (u16, String, Vec<u8>) {
    parse(
        &server.raw(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{fields}\r\n")
                .as_bytes(),
        ),
    )
}
#[test]
fn direct_clients_cannot_forge_identity_or_bypass_allowed_hosts() {
    let server = Server::start(PROXY_APP, &["--allowed-hosts", "localhost"]);
    let response = proxy_request(
        &server,
        "LOCALHOST:8080",
        "/",
        "X-Forwarded-For: malformed\r\nX-Forwarded-For: 192.0.2.1\r\nX-Forwarded-Host: evil.example\r\nX-Forwarded-Proto: https\r\nForwarded: for=192.0.2.1;proto=https\r\n",
    );
    assert_eq!(response.0, 200);
    let info: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(info["client"], "127.0.0.1");
    assert_eq!(info["scheme"], "http");
    assert_eq!(info["host"], "localhost:8080");
    let peer: SocketAddr = info["peer"].as_str().unwrap().parse().unwrap();
    assert!(peer.ip().is_loopback());
    assert_ne!(peer.port(), 0);
    assert_eq!(
        proxy_request(
            &server,
            "evil.example",
            "/",
            "X-Forwarded-Host: localhost\r\n"
        )
        .0,
        400
    );
    assert_eq!(proxy_request(&server, "localhost.evil", "/", "").0, 400);
    assert_eq!(proxy_request(&server, "user@localhost", "/", "").0, 400);
}
#[test]
fn trusted_proxy_metadata_survives_ipc_drives_redirects_and_keeps_cookies_secure() {
    let server = Server::start(
        PROXY_APP,
        &[
            "--trusted-proxies",
            "127.0.0.1,10.0.0.0/8",
            "--allowed-hosts",
            "app.example.com:443",
        ],
    );
    let fields = "X-Forwarded-For: 203.0.113.99, 192.0.2.1, 10.1.2.3\r\nX-Forwarded-Host: APP.EXAMPLE.COM.:443\r\nX-Forwarded-Proto: https\r\n";
    let response = proxy_request(&server, "backend.internal", "/", fields);
    assert_eq!(response.0, 200);
    let info: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(info["client"], "192.0.2.1");
    assert_eq!(info["scheme"], "https");
    assert_eq!(info["host"], "app.example.com:443");
    assert_eq!(info["raw"], "203.0.113.99, 192.0.2.1, 10.1.2.3");
    assert!(info["peer"].as_str().unwrap().starts_with("127.0.0.1:"));
    let redirect = proxy_request(&server, "backend.internal", "/redirect", fields);
    assert_eq!(redirect.0, 303);
    assert!(redirect.1.contains("https://app.example.com:443/login"));
    let cookie = proxy_request(
        &server,
        "backend.internal",
        "/cookie",
        &fields.replace("Proto: https", "Proto: http"),
    );
    assert_eq!(cookie.0, 200);
    let cookies: Vec<_> = cookie
        .1
        .lines()
        .filter(|s| s.to_ascii_lowercase().starts_with("set-cookie:"))
        .collect();
    assert_eq!(cookies.len(), 2);
    assert!(cookies.iter().all(|s| s.contains("; Secure")));
    assert_eq!(
        proxy_request(
            &server,
            "backend.internal",
            "/",
            &fields.replace("APP.EXAMPLE.COM.:443", "evil.example:443")
        )
        .0,
        400
    );
}
#[test]
fn malformed_trusted_proxy_headers_are_rejected_before_application_execution() {
    let server = Server::start(
        "<?scripta File::writeAtomic(\"executed\",\"bad\");echo \"bad\";",
        &["--trusted-proxies", "127.0.0.1"],
    );
    for fields in [
        "X-Forwarded-For: 192.0.2.1,\r\n",
        "X-Forwarded-For: 192.0.2.1\r\nX-Forwarded-For: 192.0.2.2\r\n",
        "X-Forwarded-Proto: https,http\r\n",
        "X-Forwarded-Host: a.example,b.example\r\n",
        "X-Forwarded-Host: a.example\r\nX-Forwarded-Host: b.example\r\n",
        "X-Forwarded-Proto: https\r\nX-Forwarded-Proto: http\r\n",
    ] {
        assert_eq!(
            proxy_request(&server, "localhost", "/", fields).0,
            400,
            "{fields}"
        );
        assert!(!server.directory.path().join("executed").exists());
    }
}
#[test]
fn shipped_proxy_example_runs_through_a_sanitizing_local_proxy() {
    let source = include_str!("../../../examples/web-proxy.scripta");
    assert_eq!(scr_cli::formatter::format(source).unwrap(), source);
    let config = include_str!("../../../examples/proxy-serve.toml");
    let server = Server::start_config(source, Some(config), &[]);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let backend = server.address;
    let (prefix_read, prefix_ready) = std::sync::mpsc::channel();
    let proxy = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut client, peer) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("proxy accept: {e}"),
            }
        };
        // Accepted sockets may inherit the listener's nonblocking mode.
        // This synchronous fixture expects reads to wait for the next bytes.
        client.set_nonblocking(false).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut received = vec![];
        let mut reported_prefix = false;
        while !received.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            client.read_exact(&mut byte).unwrap();
            received.push(byte[0]);
            assert!(received.len() < 32768);
            if !reported_prefix && received.ends_with(b"\r\n") {
                prefix_read.send(()).unwrap();
                reported_prefix = true;
            }
        }
        // Controlled front end overwrites incoming identity assertions, just as
        // deployment requires. This fixture is not a production HTTP proxy.
        let mut upstream = TcpStream::connect(backend).unwrap();
        upstream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        upstream.write_all(format!("GET / HTTP/1.1\r\nHost: backend.internal\r\nConnection: close\r\nX-Forwarded-For: {}\r\nX-Forwarded-Host: app.example.com\r\nX-Forwarded-Proto: https\r\n\r\n",peer.ip()).as_bytes()).unwrap();
        let mut response = vec![];
        upstream
            .take(1024 * 1024)
            .read_to_end(&mut response)
            .unwrap();
        client.write_all(&response).unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client.write_all(b"GET / HTTP/1.1\r\nHost: app.example.com\r\nX-Forwarded-For: 203.0.113.99\r\nX-Forwarded-Host: evil.example\r\n").unwrap();
    prefix_ready.recv_timeout(Duration::from_secs(3)).unwrap();
    // Exercise a header arriving in separate writes after the proxy starts
    // reading. Its accepted socket must support blocking reads on every OS.
    thread::sleep(Duration::from_millis(50));
    client.write_all(b"\r\n").unwrap();
    let mut response = vec![];
    client.read_to_end(&mut response).unwrap();
    proxy.join().unwrap();
    let (status, _, body) = parse(&response);
    assert_eq!(status, 200);
    let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["client"], "127.0.0.1");
    assert_eq!(info["scheme"], "https");
    assert_eq!(info["host"], "app.example.com");
}
#[test]
fn ipv6_peer_and_forwarded_addresses_are_preserved_when_loopback_is_available() {
    let probe = match std::net::TcpListener::bind("[::1]:0") {
        Ok(listener) => listener,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
            ) =>
        {
            return;
        }
        Err(e) => panic!("IPv6 probe: {e}"),
    };
    drop(probe);
    let server = Server::start(
        PROXY_APP,
        &[
            "--host",
            "::1",
            "--trusted-proxies",
            "::1/128",
            "--allowed-hosts",
            "[::1]",
        ],
    );
    let response = proxy_request(
        &server,
        "[::1]:8080",
        "/",
        "X-Forwarded-For: 2001:db8::9\r\n",
    );
    assert_eq!(response.0, 200);
    let info: serde_json::Value = serde_json::from_slice(&response.2).unwrap();
    assert_eq!(info["client"], "2001:db8::9");
    assert!(info["peer"].as_str().unwrap().starts_with("[::1]:"));
}

fn json_events(server: &Server) -> Vec<serde_json::Value> {
    let bytes = fs::read(server.directory.path().join("server.log")).unwrap();
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
    std::str::from_utf8(&bytes[..end])
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("invalid JSON log: {e}: {line}"))
        })
        .collect()
}
fn wait_event(
    server: &Server,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(event) = json_events(server).into_iter().find(&predicate) {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "missing log event: {:?}",
            json_events(server)
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn health(server: &Server, path: &str) -> (u16, String, Vec<u8>) {
    let event = wait_event(server, |event| event["event"] == "health_listening");
    let mut stream = TcpStream::connect(event["address"].as_str().unwrap()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
    let mut bytes = vec![];
    stream.read_to_end(&mut bytes).unwrap();
    parse(&bytes)
}
fn response_id(headers: &str) -> String {
    headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("x-request-id")
                .then(|| value.trim().to_owned())
        })
        .expect("missing request ID")
}
#[test]
fn json_logs_correlate_requests_diagnostics_and_exclude_default_sensitive_content() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();if ($r->path === "/error") {throw $r->text();}Web::header("X-Request-ID","FORGED_RESPONSE");echo $r->requestId;"#,
        &["--log-format", "json"],
    );
    let good = proxy_request(
        &server,
        "localhost",
        "/",
        "X-Request-ID: INCOMING_ID_SECRET\r\nAuthorization: Bearer HEADER_SECRET\r\nCookie: auth=COOKIE_SECRET\r\n",
    );
    let first = response_id(&good.1);
    assert_eq!(good.2, first.as_bytes());
    assert!(first.len() <= 64);
    assert!(!first.contains("SECRET"));
    let error = server.request(
        "POST",
        "/error?secret=QUERY_SECRET",
        b"BODY_SECRET",
        "text/plain",
    );
    assert_eq!(error.0, 500);
    let second = response_id(&error.1);
    assert_ne!(first, second);
    wait_event(&server, |e| {
        e["event"] == "response_finished" && e["requestId"] == second
    });
    let events = json_events(&server);
    assert!(
        events
            .iter()
            .any(|e| e["event"] == "request_diagnostic" && e["requestId"] == second)
    );
    for id in [&first, &second] {
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event"] == "response_headers" && e["requestId"] == *id)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event"] == "response_finished" && e["requestId"] == *id)
                .count(),
            1
        );
    }
    let log = fs::read_to_string(server.directory.path().join("server.log")).unwrap();
    for secret in [
        "INCOMING_ID_SECRET",
        "HEADER_SECRET",
        "COOKIE_SECRET",
        "QUERY_SECRET",
        "BODY_SECRET",
        "FORGED_RESPONSE",
    ] {
        assert!(!log.contains(secret), "leaked {secret}");
    }
}
#[test]
fn streaming_logs_distinguish_header_readiness_completion_and_failure() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();echo "first";flush();if ($r->path === "/error") {throw "SOURCE_SECRET";}File::writeAtomic("running","yes");while (!File::exists("continue")) {Date::sleep(5);}echo "last";"#,
        &[
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--timeout-ms",
            "3000",
        ],
    );
    let mut response = StreamingResponse::start(&server, "/");
    assert_eq!(response.chunk(), b"first");
    let id = response_id(&response.headers);
    wait_event(&server, |e| {
        e["event"] == "response_headers" && e["requestId"] == id
    });
    assert!(
        !json_events(&server)
            .iter()
            .any(|e| e["event"] == "response_finished" && e["requestId"] == id)
    );
    fs::write(server.directory.path().join("continue"), "yes").unwrap();
    assert_eq!(response.chunk(), b"last");
    assert!(response.chunk().is_empty());
    let complete = wait_event(&server, |e| {
        e["event"] == "response_finished" && e["requestId"] == id
    });
    assert_eq!(complete["outcome"], "complete");
    assert_eq!(complete["bodyBytes"], 9);
    let mut failed = StreamingResponse::start(&server, "/error");
    assert_eq!(failed.chunk(), b"first");
    let id = response_id(&failed.headers);
    let mut tail = vec![];
    let _ = failed.reader.read_to_end(&mut tail);
    let event = wait_event(&server, |e| {
        e["event"] == "response_finished" && e["requestId"] == id
    });
    assert_eq!(event["status"], 200);
    assert_eq!(event["outcome"], "failed");
    assert_eq!(event["bodyBytes"], 5);
    assert!(
        !fs::read_to_string(server.directory.path().join("server.log"))
            .unwrap()
            .contains("SOURCE_SECRET")
    );
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["responsesComplete"], 1);
    assert_eq!(metrics["responsesFailed"], 1);
    assert_eq!(metrics["activeRequests"], 0);
}
#[test]
fn private_health_routes_do_not_execute_or_reserve_application_routes() {
    let server = Server::start(
        "<?scripta File::writeAtomic(\"executed\",\"yes\");echo \"application\";",
        &["--log-format", "json", "--health-address", "127.0.0.1:0"],
    );
    assert_eq!(health(&server, "/livez").0, 200);
    assert_eq!(health(&server, "/readyz").0, 200);
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["requests"], 0);
    assert_eq!(metrics["readyWorkers"], 1);
    assert!(!server.directory.path().join("executed").exists());
    let address = wait_event(&server, |e| e["event"] == "health_listening")["address"]
        .as_str()
        .unwrap()
        .to_owned();
    for (method, body, expected) in [("HEAD", "", 200), ("POST", "", 405), ("GET", "x", 400)] {
        let mut stream = TcpStream::connect(&address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.write_all(format!("{method} /livez HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).unwrap();
        let mut response = vec![];
        stream.read_to_end(&mut response).unwrap();
        let response = parse(&response);
        assert_eq!(response.0, expected);
        if method == "HEAD" {
            assert!(response.2.is_empty());
        }
    }
    assert_eq!(health(&server, "/unknown").0, 404);
    assert_eq!(health(&server, "/metrics?secret=hidden").0, 400);
    assert_eq!(
        server.request("GET", "/livez", b"", "text/plain").2,
        b"application"
    );
    assert!(server.directory.path().join("executed").exists());
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["requests"], 1);
}
#[test]
fn disconnected_streams_and_timeouts_have_correlated_events_and_deduplicated_counters() {
    let server = Server::start(
        r#"<?scripta $r=Web::request();if ($r->path === "/ok") {echo "ok";exit 0;}if ($r->path === "/stream") {echo "first";flush();}Date::sleep(5000);"#,
        &[
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--timeout-ms",
            "250",
        ],
    );
    // Recovery is proactive. Probe a stable state after each failure instead of
    // assuming a health request beats the replacement timer on every platform.
    let recovered = |restarts| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let metrics: serde_json::Value =
                serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
            if metrics["ready"] == true
                && metrics["readyWorkers"] == 1
                && metrics["workerRestarts"] == restarts
                && [
                    "activeRequests",
                    "queuedRequests",
                    "busyWorkers",
                    "recoveringWorkers",
                ]
                .iter()
                .all(|name| metrics[*name] == 0)
            {
                return metrics;
            }
            assert!(
                Instant::now() < deadline,
                "recovery did not finish: {metrics}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    };
    let mut response = StreamingResponse::start(&server, "/stream");
    assert_eq!(response.chunk(), b"first");
    let id = response_id(&response.headers);
    drop(response);
    let event = wait_event(&server, |e| {
        e["event"] == "response_finished" && e["requestId"] == id
    });
    assert_eq!(event["outcome"], "interrupted");
    let event = wait_event(&server, |e| {
        e["event"] == "worker_failure" && e["requestId"] == id
    });
    assert_eq!(event["reason"], "client_disconnect");
    let metrics = recovered(1);
    assert_eq!(metrics["timeouts"], 0);
    assert_eq!(metrics["workerFailures"], 1);
    assert_eq!(server.request("GET", "/ok", b"", "text/plain").0, 200);
    let response = server.request("GET", "/slow", b"", "text/plain");
    assert_eq!(response.0, 504);
    let id = response_id(&response.1);
    let event = wait_event(&server, |e| {
        e["event"] == "worker_failure" && e["requestId"] == id
    });
    assert_eq!(event["reason"], "timeout");
    let metrics = recovered(2);
    assert_eq!(metrics["timeouts"], 1);
    assert_eq!(metrics["responsesInterrupted"], 1);
    assert_eq!(metrics["workerFailures"], 2);
}
#[test]
fn explicit_application_logs_are_escaped_bounded_and_correlated() {
    let server = Server::start(
        r#"<?scripta for ($i=0;$i<30;$i+=1) {Log::info("quoted\"\nvalue");}echo Web::request()->requestId;"#,
        &["--log-format", "json"],
    );
    let response = server.request("GET", "/", b"", "text/plain");
    assert_eq!(response.0, 200);
    let id = response_id(&response.1);
    wait_event(&server, |e| {
        e["event"] == "response_finished" && e["requestId"] == id
    });
    let logs: Vec<_> = json_events(&server)
        .into_iter()
        .filter(|e| e["event"] == "application_log")
        .collect();
    assert_eq!(logs.len(), 16);
    assert!(logs.iter().all(|e| e["requestId"] == id));
    assert!(
        logs.iter()
            .all(|e| e["message"].as_str().unwrap().len() <= 4100)
    );
}

#[test]
fn readiness_and_counters_report_queue_overload_and_recovery() {
    let server = Server::start(
        r#"<?scripta if (Web::request()->path === "/hold") {echo "first";flush();while (!File::exists("continue")) {Date::sleep(5);}}else {echo "ok";}"#,
        &[
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--queue",
            "1",
            "--timeout-ms",
            "3000",
        ],
    );
    let mut hold = StreamingResponse::start(&server, "/hold");
    assert_eq!(hold.chunk(), b"first");
    let mut queued = TcpStream::connect(server.address).unwrap();
    queued
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    queued
        .write_all(b"GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let metrics: serde_json::Value =
            serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
        if metrics["queuedRequests"] == 1 {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(health(&server, "/readyz").0, 503);
    assert_eq!(server.request("GET", "/ok", b"", "text/plain").0, 503);
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["overload"], 1);
    assert_eq!(metrics["busyWorkers"], 1);
    fs::write(server.directory.path().join("continue"), "yes").unwrap();
    assert!(hold.chunk().is_empty());
    let mut response = vec![];
    queued.read_to_end(&mut response).unwrap();
    assert_eq!(parse(&response).0, 200);
    assert_eq!(health(&server, "/readyz").0, 200);
}

#[cfg(unix)]
#[test]
fn health_reports_draining_without_preventing_active_request_completion() {
    let mut server = Server::start(
        r#"<?scripta File::writeAtomic("running","yes");while (!File::exists("continue")) {Date::sleep(5);}echo "done";"#,
        &[
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--drain-timeout-ms",
            "2000",
            "--timeout-ms",
            "5000",
        ],
    );
    let mut stream = TcpStream::connect(server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !server.directory.path().join("running").exists() {
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
    wait_event(&server, |e| e["event"] == "server_draining");
    assert_eq!(health(&server, "/readyz").0, 503);
    assert_eq!(health(&server, "/livez").0, 200);
    fs::write(server.directory.path().join("continue"), "yes").unwrap();
    let mut response = vec![];
    stream.read_to_end(&mut response).unwrap();
    assert_eq!(parse(&response).2, b"done");
    let deadline = Instant::now() + Duration::from_secs(3);
    while server.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let stopped = wait_event(&server, |e| e["event"] == "server_stopped");
    assert_eq!(stopped["readyWorkers"], 0);
    assert_eq!(stopped["activeRequests"], 0);
}

#[test]
fn shipped_observability_configuration_runs_application_and_private_probes() {
    let config = include_str!("../../../examples/observe-serve.toml");
    let source = include_str!("../../../examples/web-proxy.scripta");
    let server = Server::start_config(source, Some(config), &["--health-address", "127.0.0.1:0"]);
    assert_eq!(health(&server, "/livez").0, 200);
    assert_eq!(health(&server, "/readyz").0, 200);
    let response = server.request("GET", "/", b"", "text/plain");
    assert_eq!(response.0, 200);
    let id = response_id(&response.1);
    wait_event(&server, |e| {
        e["event"] == "response_finished" && e["requestId"] == id
    });
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["requests"], 1);
    assert_eq!(metrics["responsesComplete"], 1);
}

#[cfg(unix)]
fn terminate(server: &Server) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &server.child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
}
#[cfg(unix)]
fn wait_stopped(server: &mut Server) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "shutdown exceeded its allowance");
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn shutdown_closes_idle_keepalive_and_partial_headers_promptly() {
    use std::io::BufRead;
    let mut server = Server::start("<?scripta echo \"ok\";", &["--drain-timeout-ms", "5000"]);
    let mut idle = TcpStream::connect(server.address).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    idle.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut idle = std::io::BufReader::new(idle);
    loop {
        let mut line = String::new();
        assert_ne!(idle.read_line(&mut line).unwrap(), 0);
        if line == "\r\n" {
            break;
        }
    }
    let mut body = [0; 2];
    idle.read_exact(&mut body).unwrap();
    assert_eq!(&body, b"ok");
    let mut partial = TcpStream::connect(server.address).unwrap();
    partial
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    partial.write_all(b"GET / HTTP/1.1\r\nHost:").unwrap();
    let start = Instant::now();
    terminate(&server);
    assert_eq!(idle.read(&mut body).unwrap(), 0);
    let mut output = vec![];
    if let Err(error) = partial.read_to_end(&mut output) {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    assert!(output.is_empty());
    wait_stopped(&mut server);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn shutdown_drains_admitted_queue_and_active_keepalive_but_rejects_pipeline() {
    let mut server = Server::start(
        r#"<?scripta
$r=Web::request();
if ($r->path === "/hold") {File::writeAtomic("running","yes");while (!File::exists("continue")) {Date::sleep(5);}}
if ($r->path === "/late") {File::writeAtomic("late","bad");}
echo $r->path;
"#,
        &[
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--drain-timeout-ms",
            "2000",
        ],
    );
    let mut active = TcpStream::connect(server.address).unwrap();
    active
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    active.write_all(b"GET /hold HTTP/1.1\r\nHost: localhost\r\n\r\nGET /late HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !server.directory.path().join("running").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let mut queued = TcpStream::connect(server.address).unwrap();
    queued
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    queued
        .write_all(b"GET /queued HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    loop {
        let metrics: serde_json::Value =
            serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
        if metrics["queuedRequests"] == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "{metrics}");
        thread::sleep(Duration::from_millis(5));
    }
    terminate(&server);
    wait_event(&server, |e| e["event"] == "server_draining");
    terminate(&server); // Repeated signals neither force exit nor extend the budget.
    fs::write(server.directory.path().join("continue"), "yes").unwrap();
    let mut output = vec![];
    active.read_to_end(&mut output).unwrap();
    assert_eq!(parse(&output).0, 200);
    assert!(output.windows(5).any(|w| w == b"/hold"));
    assert_eq!(
        String::from_utf8_lossy(&output).matches("HTTP/1.1").count(),
        1
    );
    output.clear();
    queued.read_to_end(&mut output).unwrap();
    assert_eq!(parse(&output).0, 200);
    assert!(output.windows(7).any(|w| w == b"/queued"));
    assert!(!server.directory.path().join("late").exists());
    wait_stopped(&mut server);
}

#[cfg(unix)]
#[test]
fn drain_expiry_interrupts_stream_reaps_child_cleans_spools_and_keeps_saved_session() {
    let storage = tempfile::tempdir().unwrap();
    let sessions = storage.path().join("sessions");
    let mut server = Server::start(
        r#"<?scripta Session::start();Session::set("count",7);File::writeAtomic("pid",str(Process::currentId()));echo "saved";flush();Date::sleep(10000);echo "late";"#,
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--upload-temp-dir",
            "spools",
            "--log-format",
            "json",
            "--timeout-ms",
            "20000",
            "--drain-timeout-ms",
            "200",
        ],
    );
    let mut stream = StreamingResponse::request(
        &server,
        "POST",
        "/",
        b"owned upload",
        "application/octet-stream",
    );
    let id = response_session_id(&stream.headers);
    assert_eq!(stream.chunk(), b"saved");
    let pid = fs::read_to_string(server.directory.path().join("pid")).unwrap();
    assert!(
        fs::read_dir(server.directory.path().join("spools"))
            .unwrap()
            .next()
            .is_some()
    );
    terminate(&server);
    wait_event(&server, |e| e["event"] == "server_draining");
    terminate(&server);
    wait_stopped(&mut server);
    let mut remaining = vec![];
    let _ = stream.reader.read_to_end(&mut remaining);
    assert!(!remaining.windows(4).any(|w| w == b"late"));
    assert!(
        !remaining.ends_with(b"0\r\n\r\n"),
        "truncated stream reported successful completion"
    );
    uploads_clean(&server);
    assert!(
        !Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let replacement = Server::start(
        "<?scripta Session::start();echo str(Session::get(\"count\",0));",
        &["--session-dir", sessions.to_str().unwrap()],
    );
    assert_eq!(replacement.session_request("/", Some(&id)).2, b"7");
}

#[cfg(unix)]
#[test]
fn replacement_readiness_frontend_switch_and_old_instance_drain() {
    let mut old = Server::start(
        r#"<?scripta if (Web::request()->path === "/hold") {echo "old";flush();while (!File::exists("continue")) {Date::sleep(5);}}else {echo "old";}"#,
        &[
            "--workers",
            "2",
            "--health-address",
            "127.0.0.1:0",
            "--log-format",
            "json",
        ],
    );
    let mut held = StreamingResponse::start(&old, "/hold");
    assert_eq!(held.chunk(), b"old");
    let replacement = Server::start(
        "<?scripta echo \"new\";",
        &["--health-address", "127.0.0.1:0", "--log-format", "json"],
    );
    assert_eq!(health(&replacement, "/readyz").0, 200);
    // A controlled front-end fixture forwards to the selected backend. Switching
    // this target models a proxy reload without installing a system service.
    let front = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = front.local_addr().unwrap();
    let targets = [old.address, replacement.address];
    let proxy = thread::spawn(move || {
        for target in targets {
            let (mut client, _) = front.accept().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = vec![];
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                client.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let mut backend = TcpStream::connect(target).unwrap();
            backend
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            backend.write_all(&request).unwrap();
            std::io::copy(&mut backend, &mut client).unwrap();
        }
    });
    let probe = || {
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = vec![];
        client.read_to_end(&mut response).unwrap();
        parse(&response)
    };
    assert_eq!(probe().2, b"old");
    assert_eq!(probe().2, b"new");
    terminate(&old);
    wait_event(&old, |e| e["event"] == "server_draining");
    assert_eq!(health(&old, "/readyz").0, 503);
    fs::write(old.directory.path().join("continue"), "yes").unwrap();
    assert!(held.chunk().is_empty());
    wait_stopped(&mut old);
    assert_eq!(replacement.request("GET", "/", b"", "text/plain").2, b"new");
    proxy.join().unwrap();
}

#[cfg(unix)]
#[test]
fn drain_expiry_discards_queued_and_partial_bodies_and_uncommitted_session_changes() {
    let storage = tempfile::tempdir().unwrap();
    let sessions = storage.path().join("sessions");
    let mut server = Server::start(
        r#"<?scripta
$r=Web::request();
if ($r->path === "/queued") {File::writeAtomic("unexpected","bad");exit 0;}
Session::start();
if ($r->path === "/seed") {Session::set("count",1);}
if ($r->path === "/hold") {Session::set("count",999);File::writeAtomic("running","yes");Date::sleep(10000);}
echo str(Session::get("count",0));
"#,
        &[
            "--session-dir",
            sessions.to_str().unwrap(),
            "--upload-temp-dir",
            "spools",
            "--health-address",
            "127.0.0.1:0",
            "--log-format",
            "json",
            "--timeout-ms",
            "20000",
            "--drain-timeout-ms",
            "200",
        ],
    );
    let seed = server.session_request("/seed", None);
    assert_eq!(seed.2, b"1");
    let id = response_session_id(&seed.1);
    let mut active = TcpStream::connect(server.address).unwrap();
    active
        .write_all(
            format!(
                "GET /hold HTTP/1.1\r\nHost: localhost\r\nCookie: SCRIPTA_SESSION={id}\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !server.directory.path().join("running").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let mut queued = TcpStream::connect(server.address).unwrap();
    queued
        .write_all(b"POST /queued HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\n\r\ndata")
        .unwrap();
    let mut partial = TcpStream::connect(server.address).unwrap();
    partial
        .write_all(b"POST /partial HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1000\r\n\r\npart")
        .unwrap();
    loop {
        let metrics: serde_json::Value =
            serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
        // Queue gauges include body-reception reservations. Ensure both requests
        // crossed admission before signaling, without waiting for the partial body.
        if metrics["queuedRequests"] == 2 {
            break;
        }
        assert!(Instant::now() < deadline, "{metrics}");
        thread::sleep(Duration::from_millis(5));
    }
    terminate(&server);
    wait_stopped(&mut server);
    uploads_clean(&server);
    assert!(!server.directory.path().join("unexpected").exists());
    let replacement = Server::start(
        "<?scripta Session::start();echo str(Session::get(\"count\",0));",
        &["--session-dir", sessions.to_str().unwrap()],
    );
    assert_eq!(replacement.session_request("/", Some(&id)).2, b"1");
}

#[test]
fn shipped_deployment_configs_serve_with_private_local_fixture_storage() {
    for config in [
        include_str!("../../../examples/deploy/blue.toml"),
        include_str!("../../../examples/deploy/green.toml"),
    ] {
        let server = Server::start_config(
            include_str!("../../../examples/web-proxy.scripta"),
            Some(config),
            &[
                "--health-address",
                "127.0.0.1:0",
                "--session-dir",
                "sessions",
                "--upload-temp-dir",
                "spools",
            ],
        );
        assert_eq!(health(&server, "/readyz").0, 200);
        let response = parse(
            &server.raw(b"GET / HTTP/1.1\r\nHost: app.example.com\r\nConnection: close\r\n\r\n"),
        );
        assert_eq!(response.0, 200);
        assert!(server.directory.path().join("sessions").is_dir());
        uploads_clean(&server);
    }
}

#[test]
fn worker_recycling_counts_completed_requests_and_preserves_the_source_snapshot() {
    let server = Server::start(
        "<?scripta echo str(Process::currentId());",
        &["--worker-max-requests", "2", "--log-format", "json"],
    );
    let first = server.request("GET", "/", b"", "text/plain");
    assert_eq!(first.0, 200);
    assert_eq!(first.2, server.request("GET", "/", b"", "text/plain").2);
    wait_event(&server, |e| e["event"] == "worker_recycled");
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"changed\";",
    )
    .unwrap();
    let next = server.request("GET", "/", b"", "text/plain");
    assert_eq!(next.0, 200);
    assert_ne!(first.2, next.2);
    assert_ne!(next.2, b"changed");
    assert_eq!(next.2, server.request("GET", "/", b"", "text/plain").2);
}

#[test]
fn recycling_waits_for_a_successful_stream_and_releases_its_upload() {
    let server = Server::start(
        r#"<?scripta echo "first";flush();while (!File::exists("continue")) {Date::sleep(5);}echo "last";"#,
        &[
            "--worker-max-requests",
            "1",
            "--upload-temp-dir",
            "spools",
            "--health-address",
            "127.0.0.1:0",
            "--log-format",
            "json",
        ],
    );
    let mut response = StreamingResponse::request(
        &server,
        "POST",
        "/",
        b"upload owned by this request",
        "text/plain",
    );
    assert_eq!(response.chunk(), b"first");
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["workerRecycles"], 0);
    assert_eq!(metrics["busyWorkers"], 1);
    fs::write(server.directory.path().join("continue"), "yes").unwrap();
    assert_eq!(response.chunk(), b"last");
    assert!(response.chunk().is_empty());
    wait_event(&server, |e| e["event"] == "worker_recycled");
    uploads_clean(&server);
    let deadline = Instant::now() + Duration::from_secs(3);
    while health(&server, "/readyz").0 != 200 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let metrics: serde_json::Value =
        serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
    assert_eq!(metrics["workerRecycles"], 1);
    assert_eq!(metrics["workerRestarts"], 1);
}

#[cfg(unix)]
fn kill_worker(pid: u64) {
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
}

#[cfg(unix)]
#[test]
fn all_idle_workers_recover_without_an_application_request_and_backoff_failed_starts() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::start(
        "<?scripta echo str(Process::currentId());",
        &[
            "--workers",
            "2",
            "--session-dir",
            "sessions",
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
        ],
    );
    let sessions = server.directory.path().join("sessions");
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o755)).unwrap();
    let pids: Vec<_> = json_events(&server)
        .iter()
        .filter(|e| e["event"] == "worker_started")
        .map(|e| e["workerPid"].as_u64().unwrap())
        .collect();
    assert_eq!(pids.len(), 2);
    for pid in &pids {
        kill_worker(*pid);
    }
    wait_event(&server, |e| e["event"] == "worker_startup_failed");
    assert_eq!(health(&server, "/readyz").0, 503);
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let metrics: serde_json::Value =
            serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
        assert_eq!(metrics["requests"], 0);
        assert_eq!(metrics["configuredWorkers"], 2);
        assert_eq!(metrics["readyWorkers"], 0);
        assert_eq!(metrics["recoveringWorkers"], 2);
        if metrics["workerStartupFailures"].as_u64().unwrap() >= 6 {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let delays: Vec<_> = json_events(&server)
        .iter()
        .filter(|e| e["event"] == "worker_recovering")
        .map(|e| e["delayMs"].as_u64().unwrap())
        .collect();
    assert!(
        delays.contains(&100)
            && delays.contains(&200)
            && delays.contains(&400)
            && delays.contains(&800),
        "{delays:?}"
    );
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    loop {
        let metrics: serde_json::Value =
            serde_json::from_slice(&health(&server, "/metrics").2).unwrap();
        if metrics["readyWorkers"] == 2 && metrics["recoveringWorkers"] == 0 {
            assert_eq!(metrics["requests"], 0);
            break;
        }
        assert!(Instant::now() < deadline, "{metrics}");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(health(&server, "/readyz").0, 200);
    let response = server.request("GET", "/", b"", "text/plain");
    assert_eq!(response.0, 200);
    let pid = std::str::from_utf8(&response.2)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let before = json_events(&server).len();
    kill_worker(pid);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(event) = json_events(&server)[before..]
            .iter()
            .find(|e| e["event"] == "worker_recovering")
        {
            assert_eq!(event["delayMs"], 100);
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn shutdown_cancels_pending_worker_recovery_without_a_late_restart() {
    use std::os::unix::fs::PermissionsExt;
    let mut server = Server::start(
        "<?scripta echo \"ok\";",
        &[
            "--session-dir",
            "sessions",
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--drain-timeout-ms",
            "100",
        ],
    );
    let pid = wait_event(&server, |e| e["event"] == "worker_started")["workerPid"]
        .as_u64()
        .unwrap();
    fs::set_permissions(
        server.directory.path().join("sessions"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    kill_worker(pid);
    wait_event(&server, |e| e["event"] == "worker_startup_failed");
    terminate(&server);
    wait_stopped(&mut server);
    let events = json_events(&server);
    let draining = events
        .iter()
        .position(|e| e["event"] == "server_draining")
        .unwrap();
    assert!(
        !events[draining..]
            .iter()
            .any(|e| e["event"] == "worker_started")
    );
    let stopped = events
        .iter()
        .find(|e| e["event"] == "server_stopped")
        .unwrap();
    assert_eq!(stopped["readyWorkers"], 0);
    assert_eq!(stopped["recoveringWorkers"], 0);
}

#[test]
fn request_count_recycling_preserves_committed_sessions_between_workers() {
    let server = Server::start(SESSION_APP, &["--worker-max-requests", "1"]);
    let first = server.session_request("/increment", None);
    assert_eq!(first.0, 200);
    let id = response_session_id(&first.1);
    let second = server.session_request("/increment", Some(&id));
    assert_eq!(second.0, 200);
    assert_eq!(response_session_id(&second.1), id);
    let first: serde_json::Value = serde_json::from_slice(&first.2).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second.2).unwrap();
    assert_eq!(first["count"], 1);
    assert_eq!(second["count"], 2);
    assert_ne!(first["pid"], second["pid"]);
}

#[test]
fn watch_reloads_entry_and_includes_and_retains_the_last_good_generation() {
    let server = Server::start_files(
        "<?scripta require \"part.inc\";echo $version;",
        None,
        &[
            "--watch",
            "--watch-debounce-ms",
            "100",
            "--log-format",
            "json",
        ],
        &[("part.inc", "<?scripta $version=\"old\";")],
    );
    assert_eq!(server.request("GET", "/", b"", "text/plain").2, b"old");
    fs::write(
        server.directory.path().join("part.inc"),
        "<?scripta echo ; // PRIVATE_SOURCE",
    )
    .unwrap();
    let failure = wait_event(&server, |e| e["event"] == "reload_failed");
    assert_eq!(failure["reason"], "source_validation");
    assert!(failure["diagnostic"].as_str().unwrap().contains("part.inc"));
    assert!(!failure.to_string().contains("PRIVATE_SOURCE"));
    assert_eq!(server.request("GET", "/", b"", "text/plain").2, b"old");
    fs::write(
        server.directory.path().join("part.inc"),
        "<?scripta $version=\"new\";",
    )
    .unwrap();
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta require \"part.inc\";echo \"entry-\" . $version;",
    )
    .unwrap();
    wait_event(&server, |e| e["event"] == "reload_ready");
    assert_eq!(
        server.request("GET", "/", b"", "text/plain").2,
        b"entry-new"
    );
    assert_eq!(
        json_events(&server)
            .iter()
            .filter(|e| e["event"] == "reload_ready")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn watch_drains_inflight_requests_using_their_original_includes_and_cleans_snapshots() {
    let mut server = Server::start_files(
        r#"<?scripta echo "first";flush();File::writeAtomic("running","yes");while (!File::exists("continue")) {Date::sleep(5);}require "part.inc";echo $version;"#,
        None,
        &[
            "--watch",
            "--watch-debounce-ms",
            "50",
            "--log-format",
            "json",
            "--health-address",
            "127.0.0.1:0",
            "--upload-temp-dir",
            "spools",
            "--drain-timeout-ms",
            "2000",
        ],
        &[("part.inc", "<?scripta $version=\"old\";")],
    );
    let root = wait_event(&server, |e| e["event"] == "watch_ready")["snapshotRoot"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut held = StreamingResponse::request(&server, "POST", "/", b"body", "text/plain");
    assert_eq!(held.chunk(), b"first");
    fs::write(
        server.directory.path().join("part.inc"),
        "<?scripta $version=\"new\";",
    )
    .unwrap();
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta require \"part.inc\";echo $version;",
    )
    .unwrap();
    wait_event(&server, |e| e["event"] == "reload_draining");
    assert_eq!(health(&server, "/readyz").0, 503);
    assert_eq!(health(&server, "/livez").0, 200);
    fs::write(server.directory.path().join("continue"), "yes").unwrap();
    assert_eq!(held.chunk(), b"old");
    assert!(held.chunk().is_empty());
    let next = wait_event(&server, |e| e["event"] == "reload_ready");
    assert_eq!(server.request("GET", "/", b"", "text/plain").2, b"new");
    assert!(!std::path::Path::new(&root).exists());
    uploads_clean(&server);
    let next_root = next["snapshotRoot"].as_str().unwrap();
    terminate(&server);
    wait_stopped(&mut server);
    assert!(!std::path::Path::new(next_root).exists());
}

#[test]
fn ordinary_serve_has_no_watcher_or_snapshot_and_keeps_explicit_entry_reload() {
    let server = Server::start_files(
        "<?scripta require \"part.inc\";echo \"entry-\" . $version;",
        None,
        &["--log-format", "json"],
        &[("part.inc", "<?scripta $version=\"old\";")],
    );
    assert_eq!(
        server.request("GET", "/", b"", "text/plain").2,
        b"entry-old"
    );
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"changed\";",
    )
    .unwrap();
    fs::write(
        server.directory.path().join("part.inc"),
        "<?scripta $version=\"new\";",
    )
    .unwrap();
    thread::sleep(Duration::from_millis(400));
    assert_eq!(
        server.request("GET", "/", b"", "text/plain").2,
        b"entry-new"
    );
    assert!(!json_events(&server).iter().any(|e| matches!(
        e["event"].as_str(),
        Some("watch_ready" | "reload_ready" | "reload_draining")
    )));
}

#[cfg(unix)]
#[test]
fn watch_failed_worker_startup_keeps_old_generation_and_another_edit_retries() {
    use std::os::unix::fs::PermissionsExt;
    let server = Server::start(
        "<?scripta echo \"old\";",
        &[
            "--watch",
            "--watch-debounce-ms",
            "50",
            "--session-dir",
            "sessions",
            "--log-format",
            "json",
        ],
    );
    let sessions = server.directory.path().join("sessions");
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"candidate\";",
    )
    .unwrap();
    wait_event(&server, |e| {
        e["event"] == "reload_failed" && e["reason"] == "worker_startup"
    });
    assert_eq!(
        json_events(&server)
            .iter()
            .filter(|e| e["event"] == "worker_started")
            .count(),
        1
    );
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(server.request("GET", "/", b"", "text/plain").2, b"old");
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"recovered\";",
    )
    .unwrap();
    wait_event(&server, |e| e["event"] == "reload_ready");
    assert_eq!(
        server.request("GET", "/", b"", "text/plain").2,
        b"recovered"
    );
}

#[cfg(unix)]
#[test]
fn shutdown_during_watch_drain_reaps_staged_children_and_removes_snapshot() {
    let mut server = Server::start(
        "<?scripta echo \"first\";flush();Date::sleep(10000);",
        &[
            "--watch",
            "--watch-debounce-ms",
            "50",
            "--drain-timeout-ms",
            "300",
            "--log-format",
            "json",
        ],
    );
    let root = wait_event(&server, |e| e["event"] == "watch_ready")["snapshotRoot"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut stream = StreamingResponse::start(&server, "/");
    assert_eq!(stream.chunk(), b"first");
    fs::write(
        server.directory.path().join("app.scripta"),
        "<?scripta echo \"next\";",
    )
    .unwrap();
    wait_event(&server, |e| e["event"] == "reload_draining");
    terminate(&server);
    wait_stopped(&mut server);
    assert!(!std::path::Path::new(&root).exists());
    for event in json_events(&server)
        .iter()
        .filter(|e| e["event"] == "worker_started")
    {
        let pid = event["workerPid"].as_u64().unwrap();
        assert!(
            !Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        );
    }
    assert!(
        !json_events(&server)
            .iter()
            .any(|e| e["event"] == "reload_ready")
    );
}

#[test]
fn shipped_watch_example_executes_an_include_edit_with_configured_polling() {
    let source = include_str!("../../../examples/watch/main.scripta");
    let helper = include_str!("../../../examples/watch/message.inc");
    for text in [source, helper] {
        assert_eq!(scr_cli::formatter::format(text).unwrap(), text);
    }
    let server = Server::start_files(
        source,
        Some(include_str!("../../../examples/watch/serve.toml")),
        &["--health-address", "127.0.0.1:0"],
        &[("message.inc", helper)],
    );
    assert_eq!(
        server.request("GET", "/", b"", "text/plain").2,
        b"Edit message.inc, save, then refresh.\n"
    );
    fs::write(
        server.directory.path().join("message.inc"),
        "<?scripta\n$message = \"Reloaded.\\n\";\n",
    )
    .unwrap();
    wait_event(&server, |e| e["event"] == "reload_ready");
    assert_eq!(
        server.request("GET", "/", b"", "text/plain").2,
        b"Reloaded.\n"
    );
    assert_eq!(health(&server, "/readyz").0, 200);
}

#[test]
fn tag_free_web_entry_and_include_execute_in_persistent_workers() {
    let server = Server::start_files(
        "require_once 'helper.scripta'; Web::header('Content-Type', 'text/plain'); echo message() . Web::request()->path;",
        None,
        &[],
        &[(
            "helper.scripta",
            "function message() { return 'tag free '; }",
        )],
    );
    for path in ["/first", "/second"] {
        let (status, _, body) = server.request("GET", path, b"", "text/plain");
        assert_eq!(status, 200);
        assert_eq!(body, format!("tag free {path}").as_bytes());
    }
}

#[test]
fn database_extensions_are_registered_in_persistent_web_workers() {
    let source = if cfg!(feature = "mysql") {
        "try { Db::connect('mysql:host=;dbname=unused','fixture','secret'); } catch (TypeError $e) { echo 'mysql enabled'; }"
    } else {
        "try { Db::connect('mysql:host=;dbname=unused','fixture','secret'); } catch (DbUnsupportedError $e) { echo 'mysql disabled'; }"
    };
    let server = Server::start(source, &[]);
    for _ in 0..2 {
        let (status, _, body) = server.request("GET", "/", b"", "text/plain");
        assert_eq!(status, 200);
        assert_eq!(
            body,
            if cfg!(feature = "mysql") {
                b"mysql enabled".as_slice()
            } else {
                b"mysql disabled".as_slice()
            }
        );
    }
}

#[test]
#[cfg(feature = "mysql")]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn mysql_live_connections_work_in_persistent_web_workers() {
    assert!(
        std::env::var_os("SCR_MYSQL_FIXTURE_CA").is_some(),
        "run mysql_fixture.py"
    );
    let server = Server::start(
        r#"
$db=Db::connect(Env::get("SCR_MYSQL_FIXTURE_DSN"),Env::get("SCR_MYSQL_FIXTURE_USER"),Env::get("SCR_MYSQL_FIXTURE_PASSWORD"),["tlsCaFile"=>Env::get("SCR_MYSQL_FIXTURE_CA")]);
try { $s=Db::prepare($db,"SELECT ? AS n"); Db::execute($s,[42]); echo str(Db::fetchColumn($s)); } finally { Db::close($db); }
"#,
        &[],
    );
    for _ in 0..2 {
        let (status, _, body) = server.request("GET", "/", b"", "text/plain");
        assert_eq!(status, 200);
        assert_eq!(body, b"42");
    }
}
