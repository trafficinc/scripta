use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::{FilesystemPolicy, StdlibOptions};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn runtime(path: Option<&Path>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            script_path: path.map(Path::to_owned),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<String, RuntimeError> {
    let mut output = Vec::new();
    runtime.execute_source(&format!("<?scr {source}"), &mut output)?;
    Ok(String::from_utf8(output).unwrap())
}
fn error(runtime: &mut Runtime, source: &str, kind: &str) -> RuntimeError {
    let error = run(runtime, source).unwrap_err();
    assert!(
        error.message.starts_with(&format!("{kind}:")),
        "{source}: {error}"
    );
    error
}
#[test]
fn json_roundtrips_structures_numbers_unicode_and_empty_objects() {
    let mut runtime = runtime(None);
    run(&mut runtime, r#"
        $data = ["null" => null, "bool" => true, "list" => [1, 2.5, "世界", false], "integer" => 9223372036854775807];
        $encoded = Json::encode($data); $decoded = Json::decode($encoded); $pretty = Json::pretty($data);
        $object = Json::decode("{}"); $array = Json::decode("[]");
        $float = Json::decode("1.0"); $before = Json::decode("-9223372036854775808");
    "#).unwrap();
    assert_eq!(runtime.get_global("data"), runtime.get_global("decoded"));
    assert!(matches!(runtime.get_global("pretty"), Some(Value::String(s)) if s.contains('\n')));
    assert_eq!(runtime.get_global("object"), Some(&Value::Map(vec![])));
    assert_eq!(runtime.get_global("array"), Some(&Value::List(vec![])));
    assert_eq!(runtime.get_global("float"), Some(&Value::Float(1.0)));
    assert_eq!(runtime.get_global("before"), Some(&Value::Int(i64::MIN)));
    let failure = error(&mut runtime, r#"Json::decode("{\n  bad}");"#, "JsonError");
    assert!(failure.message.contains("line 2, column 3"));
    for source in [
        r#"Json::decode("NaN");"#,
        r#"Json::decode("1e999");"#,
        r#"Json::decode("18446744073709551616");"#,
        "Json::encode([1 => true]);",
        "Json::encode(str);",
    ] {
        error(&mut runtime, source, "JsonError");
    }
    error(&mut runtime, "Json::decode(1);", "TypeError");
    assert_eq!(
        run(
            &mut runtime,
            r#"try { Json::decode("bad"); } catch (JsonError $e) { echo "caught"; }"#
        )
        .unwrap(),
        "caught"
    );
}

#[test]
fn modules_execute_in_order_share_caller_scope_and_once_uses_canonical_identity() {
    let dir = tempfile::tempdir().unwrap();
    let lib = dir.path().join("lib");
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(lib.join("helper.scr"), r#"<?scr echo "helper;"; $count += 1; function greet($name) { return "hello " . $name; } require_once "config.scr";"#).unwrap();
    std::fs::write(
        lib.join("config.scr"),
        r#"<?scr echo "config;"; $configured = true;"#,
    )
    .unwrap();
    std::fs::write(lib.join("repeat.scr"), "<?scr $count += 1;").unwrap();
    let mut runtime = runtime(Some(&dir.path().join("main.scr")));
    let output = run(&mut runtime, r#"$count = 0; echo "start;"; require "lib/helper.scr"; require_once "lib/../lib/helper.scr"; require_once "lib/config.scr"; require "lib/repeat.scr"; require "lib/repeat.scr"; echo greet("world");"#).unwrap();
    assert_eq!(output, "start;helper;config;hello world");
    assert_eq!(runtime.get_global("count"), Some(&Value::Int(3)));
    assert_eq!(runtime.get_global("configured"), Some(&Value::Bool(true)));
    // Loaded-set state is retained across execute calls on the same runtime.
    assert_eq!(
        run(&mut runtime, r#"require_once "lib/helper.scr";"#).unwrap(),
        ""
    );
    std::fs::write(lib.join("local.scr"), "<?scr $local += 2;").unwrap();
    std::fs::write(
        lib.join("function.scr"),
        r#"<?scr function helper() { $local = 3; require "local.scr"; return $local; }"#,
    )
    .unwrap();
    assert_eq!(
        run(
            &mut runtime,
            r#"require "lib/function.scr"; echo str(helper());"#
        )
        .unwrap(),
        "5"
    );
    // Included declarations become available only when encountered.
    std::fs::write(lib.join("order.scr"), "<?scr later(); function later() {}").unwrap();
    assert!(
        run(&mut runtime, r#"require "lib/order.scr";"#)
            .unwrap_err()
            .message
            .contains("undefined function later")
    );
}

#[test]
fn include_failures_cycles_retry_and_traces_retain_original_files() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.scr");
    let mut runtime = runtime(Some(&main));
    error(
        &mut runtime,
        r#"require "missing.scr";"#,
        "ModuleNotFoundError",
    );
    error(&mut runtime, "require 1;", "TypeError");
    error(
        &mut runtime,
        r#"require "https://example.com/mod.scr";"#,
        "ModuleError",
    );
    let bad = dir.path().join("bad.scr");
    std::fs::write(&bad, "echo 'missing tag';").unwrap();
    let failure = error(
        &mut runtime,
        r#"require_once "bad.scr";"#,
        "ModuleSyntaxError",
    );
    assert_eq!(
        failure.source.as_ref().unwrap().path,
        std::fs::canonicalize(&bad).unwrap()
    );
    std::fs::write(&bad, "<?scr echo \"retry\";").unwrap();
    assert_eq!(
        run(&mut runtime, r#"require_once "bad.scr";"#).unwrap(),
        "retry"
    );
    std::fs::write(dir.path().join("a.scr"), r#"<?scr require "b.scr";"#).unwrap();
    std::fs::write(dir.path().join("b.scr"), r#"<?scr require_once "a.scr";"#).unwrap();
    let failure = error(&mut runtime, r#"require "a.scr";"#, "CircularIncludeError");
    let rendered = failure.render("main.scr", "<?scr require \"a.scr\";");
    assert!(
        rendered.contains("a.scr:1:")
            && rendered.contains("b.scr:1:")
            && rendered.contains("main.scr:1:")
    );
    let helper = dir.path().join("trace.scr");
    std::fs::write(
        &helper,
        "<?scr\nfunction explode() {\n    Json::decode(\"invalid\");\n}\n",
    )
    .unwrap();
    let failure = error(
        &mut runtime,
        "require \"trace.scr\";\nexplode();",
        "JsonError",
    );
    let rendered = failure.render("wrong.scr", "wrong");
    assert!(rendered.contains("trace.scr:3:"), "{rendered}");
    assert!(rendered.contains("main.scr:2:"), "{rendered}");
    // The entry file participates in cycle detection before it can execute twice.
    std::fs::write(&main, "<?scr require \"main.scr\";").unwrap();
    error(
        &mut runtime,
        r#"require "main.scr";"#,
        "CircularIncludeError",
    );
}

#[cfg(unix)]
#[test]
fn require_once_resolves_symlink_aliases() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("original.scr"), "<?scr echo \"once\";").unwrap();
    std::os::unix::fs::symlink("original.scr", dir.path().join("alias.scr")).unwrap();
    let mut runtime = runtime(Some(&dir.path().join("main.scr")));
    assert_eq!(
        run(
            &mut runtime,
            r#"require_once "original.scr"; require_once "alias.scr";"#
        )
        .unwrap(),
        "once"
    );
}

#[test]
fn includes_respect_read_permission_and_filesystem_roots() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("helper.scr"), "<?scr echo \"forbidden\";").unwrap();
    let mut options = RuntimeOptions::default();
    options.context.script_path = Some(dir.path().join("main.scr"));
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.read");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    error(&mut denied, r#"require "helper.scr";"#, "PermissionError");
    let mut restricted = Runtime::new();
    scr_stdlib::register_with_options(
        &mut restricted,
        StdlibOptions {
            filesystem: FilesystemPolicy {
                roots: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    error(
        &mut restricted,
        r#"require "helper.scr";"#,
        "PermissionError",
    );
}

// No external network services: each fixture binds an ephemeral loopback port.
fn server(
    count: usize,
    handler: impl Fn(usize, &str) -> Vec<u8> + Send + 'static,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        for index in 0..count {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    other => panic!("fixture accept failed: {other:?}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let request = read_request(&mut stream);
            let reply = handler(index, &request);
            let _ = stream.write_all(&reply);
            tx.send(request).unwrap();
        }
    });
    (url, rx, worker)
}
fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}
fn reply(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut output = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    output.extend_from_slice(body);
    output
}
#[test]
fn http_response_methods_headers_post_and_error_statuses() {
    let (url, requests, worker) = server(3, |i, _| {
        reply(
            if i == 2 { "503 Unavailable" } else { "200 OK" },
            "X-Value: a\r\nX-Value: b\r\nContent-Type: application/json\r\n",
            br#"{"status":"up"}"#,
        )
    });
    let mut runtime = runtime(None);
    runtime.set_global("url", Value::String(url)).unwrap();
    assert_eq!(run(&mut runtime, r#"$r = Http::get($url, ["headers" => ["X-Test" => "hello"]]); $status = $r->status; $headers = $r->headers; $body = $r->body; $text = $r->text(); $ok = $r->ok(); $data = $r->json(); echo $data["status"];"#).unwrap(), "up");
    assert_eq!(runtime.get_global("status"), Some(&Value::Int(200)));
    assert_eq!(runtime.get_global("ok"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("body"), runtime.get_global("text"));
    let first = requests.recv().unwrap();
    assert!(first.to_ascii_lowercase().contains("x-test: hello"));
    assert_eq!(
        run(&mut runtime, r#"echo Json::encode($headers["x-value"]);"#).unwrap(),
        r#"["a","b"]"#
    );
    run(&mut runtime, r#"$r = Http::post($url, Json::encode(["message" => "hello"]), ["headers" => ["Content-Type" => "application/json"]]);"#).unwrap();
    let second = requests.recv().unwrap();
    assert!(second.starts_with("POST "));
    assert!(second.ends_with(r#"{"message":"hello"}"#));
    run(&mut runtime, r#"$r = Http::request("PATCH", $url, ["body" => "patch"]); $ok = $r->ok(); $status = $r->status;"#).unwrap();
    assert_eq!(runtime.get_global("ok"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("status"), Some(&Value::Int(503)));
    assert!(requests.recv().unwrap().ends_with("patch"));
    worker.join().unwrap();
    error(&mut runtime, "$r->ok(1);", "TypeError");
    error(&mut runtime, "Json::encode($r);", "JsonError");
    error(&mut runtime, "$r->status = 1;", "PropertyError");
}

#[test]
fn http_redirects_are_bounded_and_may_be_disabled() {
    let (url, requests, worker) = server(5, |i, _| {
        if i == 1 {
            reply("200 OK", "", b"done")
        } else {
            reply("302 Found", "Location: /again\r\n", b"")
        }
    });
    let mut runtime = runtime(None);
    runtime.set_global("url", Value::String(url)).unwrap();
    assert_eq!(
        run(&mut runtime, r#"$r = Http::get($url); echo $r->text();"#).unwrap(),
        "done"
    );
    error(
        &mut runtime,
        r#"Http::get($url, ["maxRedirects" => 1]);"#,
        "RedirectError",
    );
    run(
        &mut runtime,
        r#"$r = Http::get($url, ["maxRedirects" => 0]); $status = $r->status;"#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("status"), Some(&Value::Int(302)));
    worker.join().unwrap();
    assert_eq!(requests.iter().count(), 5);
}

#[test]
fn http_limits_timeouts_connection_encoding_and_json_failures_are_errors() {
    let (url, requests, worker) = server(4, |i, _| match i {
        0 => reply("200 OK", "", b"oversized"),
        1 => reply("404 Not Found", "", b"not JSON"),
        2 => reply("200 OK", "", &[0xff]),
        _ => {
            thread::sleep(Duration::from_millis(150));
            reply("200 OK", "", b"late")
        }
    });
    let mut runtime = runtime(None);
    runtime.set_global("url", Value::String(url)).unwrap();
    error(
        &mut runtime,
        r#"Http::get($url, ["maxBodyBytes" => 4]);"#,
        "ResponseLimitError",
    );
    run(&mut runtime, "$r = Http::get($url); $status = $r->status;").unwrap();
    assert_eq!(runtime.get_global("status"), Some(&Value::Int(404)));
    error(&mut runtime, "$r->json();", "JsonError");
    error(&mut runtime, "Http::get($url);", "EncodingError");
    error(
        &mut runtime,
        r#"Http::get($url, ["timeoutMs" => 30]);"#,
        "TimeoutError",
    );
    worker.join().unwrap();
    assert_eq!(requests.iter().count(), 4);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    runtime.set_global("url", Value::String(url)).unwrap();
    error(&mut runtime, "Http::get($url);", "ConnectionError");
    for source in [
        r#"Http::get("file:///etc/passwd");"#,
        r#"Http::get("http://user:pass@localhost/");"#,
        r#"Http::get($url, ["timeoutMs" => 0]);"#,
        r#"Http::get($url, ["headers" => ["X-Bad" => "bad\r\nheader"]]);"#,
        r#"Http::get($url, ["verifyTls" => false]);"#,
        r#"Http::get($url, ["maxBodyBytes" => 999999999]);"#,
    ] {
        error(&mut runtime, source, "TypeError");
    }
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.remove("http.request");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    error(
        &mut denied,
        r#"Http::get("http://localhost");"#,
        "PermissionError",
    );
}

#[test]
fn https_rejects_untrusted_certificates() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        cert.key_pair.serialize_der(),
    ));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok((stream, _)) = listener.accept() {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let connection =
                    rustls::ServerConnection::new(std::sync::Arc::new(config)).unwrap();
                let mut stream = rustls::StreamOwned::new(connection, stream);
                let _ = stream.read(&mut [0; 128]);
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
    });
    let mut runtime = runtime(None);
    runtime
        .set_global(
            "url",
            Value::String(format!("https://localhost:{}", address.port())),
        )
        .unwrap();
    error(
        &mut runtime,
        r#"Http::get($url, ["timeoutMs" => 2000]);"#,
        "TlsError",
    );
    worker.join().unwrap();
}

#[test]
fn deep_include_chains_are_bounded_and_runtime_state_recovers() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..70 {
        std::fs::write(
            dir.path().join(format!("part{index}.scr")),
            format!("<?scr require \"part{}.scr\";", index + 1),
        )
        .unwrap();
    }
    let mut runtime = runtime(Some(&dir.path().join("main.scr")));
    error(&mut runtime, r#"require "part0.scr";"#, "ModuleError");
    assert_eq!(
        run(&mut runtime, r#"echo "recovered";"#).unwrap(),
        "recovered"
    );
}

#[test]
fn json_preserves_internal_looking_object_keys_and_rejects_excessive_depth() {
    let mut runtime = runtime(None);
    runtime
        .set_global(
            "json",
            Value::String(r#"{"$serde_json::private::Number":"123","other":true}"#.into()),
        )
        .unwrap();
    run(
        &mut runtime,
        "$data = Json::decode($json); $encoded = Json::encode($data);",
    )
    .unwrap();
    assert!(matches!(runtime.get_global("data"), Some(Value::Map(entries)) if entries.len() == 2));
    runtime
        .set_global(
            "json",
            Value::String(format!("{}0{}", "[".repeat(140), "]".repeat(140))),
        )
        .unwrap();
    error(&mut runtime, "Json::decode($json);", "JsonError");
}

#[test]
fn response_limits_apply_to_chunked_bodies_and_exact_boundaries() {
    let (url, requests, worker) = server(3, |index, _| {
        match index {
        0 => b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n6\r\nabcdef\r\n0\r\n\r\n".to_vec(),
        1 => reply("200 OK", "", b"1234"),
        _ => reply("204 No Content", "", b""),
    }
    });
    let mut runtime = runtime(None);
    runtime.set_global("url", Value::String(url)).unwrap();
    error(
        &mut runtime,
        r#"Http::get($url, ["maxBodyBytes" => 4]);"#,
        "ResponseLimitError",
    );
    assert_eq!(
        run(
            &mut runtime,
            r#"$r = Http::get($url, ["maxBodyBytes" => 4]); echo $r->text();"#
        )
        .unwrap(),
        "1234"
    );
    assert_eq!(
        run(
            &mut runtime,
            r#"$r = Http::get($url, ["maxBodyBytes" => 0, "headers" => []]); echo $r->text();"#
        )
        .unwrap(),
        ""
    );
    worker.join().unwrap();
    assert_eq!(requests.iter().count(), 3);
}
