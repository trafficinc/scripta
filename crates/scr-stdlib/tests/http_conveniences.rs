use scr_runtime::{Runtime, RuntimeOptions, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};
fn runtime() -> Runtime {
    let mut rt = Runtime::new();
    scr_stdlib::register(&mut rt).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) {
    rt.execute_source(source, &mut Vec::new()).unwrap();
}
fn fail(rt: &mut Runtime, source: &str, kind: &str) {
    rt.set_global("result", Value::Int(42)).unwrap();
    let e = rt.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(e.message.starts_with(&format!("{kind}:")), "{source}: {e}");
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
}
fn reply(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend(body);
    bytes
}
fn request(stream: &mut TcpStream) -> Vec<u8> {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut data = Vec::new();
    let mut byte = [0];
    while !data.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        data.push(byte[0]);
        assert!(data.len() < 65536);
    }
    let text = String::from_utf8_lossy(&data);
    let length = text
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let start = data.len();
    data.resize(start + length, 0);
    stream.read_exact(&mut data[start..]).unwrap();
    data
}
fn server(
    count: usize,
    mut handler: impl FnMut(usize, &[u8], &str) -> Vec<u8> + Send + 'static,
) -> (String, thread::JoinHandle<Vec<Vec<u8>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let url = base.clone();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen = Vec::new();
        while seen.len() < count {
            assert!(Instant::now() < deadline, "missing fixture request");
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let data = request(&mut stream);
                    let response = handler(seen.len(), &data, &base);
                    stream.write_all(&response).unwrap();
                    seen.push(data);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("{e}"),
            }
        }
        seen
    });
    (url, worker)
}
fn header(data: &[u8], name: &str) -> Option<String> {
    String::from_utf8_lossy(data)
        .split("\r\n\r\n")
        .next()
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim().to_owned())
}
fn set_url(rt: &mut Runtime, url: &str) {
    rt.set_global("url", Value::String(url.into())).unwrap();
}

#[test]
fn multipart_binary_fields_metadata_and_exact_wire_limits() {
    let mut rt = runtime();
    run(
        &mut rt,
        r#"$parts=[["name"=>"tag","data"=>"café"],["name"=>"tag","data"=>Hex::decode("00ff"),"filename"=>"data.bin"]]; $form=Http::multipart($parts); $size=len($form["body"]); $exact=Http::multipart($parts,["maxOutputBytes"=>$size]);"#,
    );
    assert_eq!(
        rt.get_global("size"),
        rt.get_global("exact")
            .and_then(|v| if let Value::Map(entries) = v {
                entries
                    .iter()
                    .find(|(k, _)| matches!(k,scr_runtime::MapKey::String(k) if k=="body"))
                    .map(|(_, v)| v)
                    .and_then(|v| {
                        if let Value::Bytes(b) = v {
                            Some(b.len())
                        } else {
                            None
                        }
                    })
            } else {
                None
            })
            .map(|n| Value::Int(n as i64))
            .as_ref()
    );
    fail(
        &mut rt,
        "$result=Http::multipart($parts,['maxOutputBytes'=>$size-1]);",
        "ResourceLimitError",
    );
    fail(
        &mut rt,
        "$result=Http::multipart($parts,['maxParts'=>1]);",
        "ResourceLimitError",
    );
    for part in [
        r#"["name"=>"bad\r\nX: y","data"=>""]"#,
        r#"["name"=>"x","data"=>"","filename"=>"../x"]"#,
        r#"["name"=>"x","data"=>"","filename"=>"x","contentType"=>"a/ b"]"#,
        r#"["name"=>"x","data"=>"","contentType"=>"text/plain"]"#,
        r#"["name"=>"x","data"=>1]"#,
    ] {
        fail(
            &mut rt,
            &format!("$result=Http::multipart([{part}]);"),
            "MultipartError",
        );
    }
    let (url, worker) = server(1, |_, data, _| {
        let content = header(data, "content-type").unwrap();
        assert!(content.starts_with("multipart/form-data; boundary=scripta-"));
        assert!(data.windows(4).any(|w| w == b"\x00\xff\r\n"));
        assert_eq!(
            String::from_utf8_lossy(data)
                .matches("name=\"tag\"")
                .count(),
            2
        );
        reply("200 OK", "", b"ok")
    });
    set_url(&mut rt, &url);
    run(
        &mut rt,
        "$response=Http::post($url,$form['body'],['headers'=>['Content-Type'=>$form['contentType']]]);",
    );
    worker.join().unwrap();
}

#[test]
fn jars_follow_same_origin_redirects_and_preserve_cookie_path_order() {
    let (url, worker) = server(3, |i, data, _| match i {
        0 => reply(
            "302 Found",
            "Location: /deep/page\r\nSet-Cookie: id=root; Path=/\r\nSet-Cookie: id=deep; Path=/deep\r\nSet-Cookie: no=domain; Domain=127.0.0.1\r\n",
            b"",
        ),
        1 => {
            assert_eq!(header(data, "cookie").as_deref(), Some("id=deep; id=root"));
            assert!(header(data, "authorization").is_none());
            reply(
                "200 OK",
                "Set-Cookie: id=gone; Path=/deep; Max-Age=0\r\n",
                b"ok",
            )
        }
        _ => {
            assert_eq!(header(data, "cookie").as_deref(), Some("id=root"));
            reply("200 OK", "", b"ok")
        }
    });
    let mut rt = runtime();
    set_url(&mut rt, &url);
    run(
        &mut rt,
        "$jar=Http::cookieJar(); Http::get($url,['cookieJar'=>$jar,'headers'=>['Authorization'=>'secret']]); Http::get($url . '/deep/page',['cookieJar'=>$jar]); Http::closeCookieJar($jar); Http::closeCookieJar($jar); $closed=$jar->closed;",
    );
    assert_eq!(rt.get_global("closed"), Some(&Value::Bool(true)));
    fail(
        &mut rt,
        "$result=Http::get($url,['cookieJar'=>$jar]);",
        "CookieError",
    );
    worker.join().unwrap();
}

#[test]
fn redirect_credentials_custom_headers_and_cookies_do_not_cross_origins() {
    let (other, second) = server(1, |_, data, _| {
        for name in ["cookie", "authorization", "x-secret"] {
            assert!(header(data, name).is_none(), "leaked {name}");
        }
        reply("200 OK", "", b"ok")
    });
    let (url, first) = server(1, move |_, _, _| {
        reply(
            "302 Found",
            &format!("Location: {other}/\r\nSet-Cookie: session=secret; Path=/\r\n"),
            b"",
        )
    });
    let mut rt = runtime();
    set_url(&mut rt, &url);
    run(
        &mut rt,
        "$jar=Http::cookieJar(); Http::get($url,['cookieJar'=>$jar,'headers'=>['Authorization'=>'secret','X-Secret'=>'secret']]);",
    );
    first.join().unwrap();
    second.join().unwrap();
    let (url, worker) = server(1, |_, _, _| {
        reply(
            "307 Temporary Redirect",
            "Location: http://elsewhere.invalid/\r\n",
            b"",
        )
    });
    set_url(&mut rt, &url);
    fail(
        &mut rt,
        "$result=Http::post($url,'private',['cookieJar'=>$jar]);",
        "RedirectError",
    );
    worker.join().unwrap();
}

#[test]
fn retries_are_opt_in_safe_bounded_and_keep_one_transfer_duration() {
    let (url, worker) = server(3, |i, data, _| {
        assert!(data.starts_with(b"GET "));
        if i < 2 {
            reply("503 Service Unavailable", "Retry-After: 0\r\n", b"busy")
        } else {
            reply("200 OK", "", b"done")
        }
    });
    let mut rt = runtime();
    set_url(&mut rt, &url);
    run(
        &mut rt,
        "$response=Http::get($url,['retry'=>['maxRetries'=>2,'backoffMs'=>10]]); $status=$response->status; $duration=$response->durationMs;",
    );
    assert_eq!(rt.get_global("status"), Some(&Value::Int(200)));
    assert!(matches!(rt.get_global("duration"),Some(Value::Float(ms)) if *ms>=25.0));
    worker.join().unwrap();
    for options in [
        "['retry'=>['maxRetries'=>6]]",
        "['retry'=>['backoffMs'=>-1]]",
        "['retry'=>['maxBackoffMs'=>0]]",
        "['retry'=>['oops'=>1]]",
        "['retry'=>[]]",
    ] {
        fail(
            &mut rt,
            &format!("$result=Http::get('http://127.0.0.1:1',{options});"),
            "TypeError",
        );
    }
    fail(
        &mut rt,
        "$result=Http::post('http://127.0.0.1:1','data',['retry'=>['maxRetries'=>1]]);",
        "TypeError",
    );
    let (url, worker) = server(1, |_, _, _| {
        reply(
            "429 Too Many Requests",
            "Retry-After: 9999999999999999999999999\r\n",
            b"later",
        )
    });
    set_url(&mut rt, &url);
    run(
        &mut rt,
        "$response=Http::get($url,['retry'=>['maxRetries'=>5]]); $status=$response->status;",
    );
    assert_eq!(rt.get_global("status"), Some(&Value::Int(429)));
    worker.join().unwrap();
}

#[test]
fn retries_respect_body_limits_total_deadline_and_cancellation() {
    let (url, worker) = server(1, |_, _, _| reply("503 Service Unavailable", "", b"large"));
    let mut rt = runtime();
    set_url(&mut rt, &url);
    fail(
        &mut rt,
        "$result=Http::get($url,['maxBodyBytes'=>4,'retry'=>['maxRetries'=>2]]);",
        "ResponseLimitError",
    );
    worker.join().unwrap();
    let reserved = TcpListener::bind("127.0.0.1:0").unwrap();
    let unavailable = format!("http://{}", reserved.local_addr().unwrap());
    drop(reserved);
    set_url(&mut rt, &unavailable);
    fail(
        &mut rt,
        "$result=Http::get($url,['timeoutMs'=>60,'retry'=>['maxRetries'=>5,'backoffMs'=>10]]);",
        "TimeoutError",
    );
    // A long retry delay is cooperatively cancellable and cannot be caught.
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut rt = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    scr_stdlib::register(&mut rt).unwrap();
    let (ready, received) = std::sync::mpsc::channel();
    let (url, worker) = server(1, move |_, _, _| {
        ready.send(()).unwrap();
        reply("503 Service Unavailable", "", b"busy")
    });
    set_url(&mut rt, &url);
    let cancel = thread::spawn(move || {
        received.recv_timeout(Duration::from_secs(3)).unwrap();
        thread::sleep(Duration::from_millis(30));
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let start = Instant::now();
    assert!(rt.execute_source("try {Http::get($url,['retry'=>['maxRetries'=>2,'backoffMs'=>1000]]);} catch (Error $e) {}",&mut Vec::new()).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    cancel.join().unwrap();
    worker.join().unwrap();
}

#[test]
fn jar_updates_are_atomic_at_limits_but_survive_later_body_failure() {
    let (url, worker) = server(3, |i, data, _| match i {
        0 => reply("200 OK", "Set-Cookie: good=one; Path=/\r\n", &[0xff]),
        1 => {
            assert_eq!(header(data, "cookie").as_deref(), Some("good=one"));
            reply(
                "200 OK",
                &format!(
                    "Set-Cookie: added=two; Path=/\r\nSet-Cookie: huge={}\r\n",
                    "x".repeat(4092)
                ),
                b"ok",
            )
        }
        _ => {
            assert_eq!(header(data, "cookie").as_deref(), Some("good=one"));
            reply("200 OK", "", b"ok")
        }
    });
    let mut rt = runtime();
    set_url(&mut rt, &url);
    run(&mut rt, "$jar=Http::cookieJar();");
    fail(
        &mut rt,
        "$result=Http::get($url,['cookieJar'=>$jar]);",
        "EncodingError",
    );
    fail(
        &mut rt,
        "$result=Http::get($url,['cookieJar'=>$jar]);",
        "CookieLimitError",
    );
    run(&mut rt, "Http::get($url,['cookieJar'=>$jar]);");
    worker.join().unwrap();
}

#[test]
fn jar_ownership_limits_closed_state_and_registration_cleanup_are_explicit() {
    let mut owner = runtime();
    run(
        &mut owner,
        "$jar=Http::cookieJar(); $all=[]; for($i=0;$i<63;$i=$i+1){ $all[]=Http::cookieJar(); }",
    );
    fail(&mut owner, "$result=Http::cookieJar();", "CookieLimitError");
    run(
        &mut owner,
        "Http::closeCookieJar($jar); $fresh=Http::cookieJar();",
    );
    let value = owner.get_global("fresh").unwrap().clone();
    let mut other = runtime();
    other.set_global("jar", value.clone()).unwrap();
    fail(
        &mut other,
        "$result=Http::get('http://127.0.0.1:1',['cookieJar'=>$jar]);",
        "TypeError",
    );
    fail(
        &mut other,
        "$result=Http::closeCookieJar($jar);",
        "TypeError",
    );
    #[derive(Debug)]
    struct Fake;
    impl scr_runtime::NativeObject for Fake {
        fn type_name(&self) -> &str {
            "HttpCookieJar"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("forged".into())
        }
    }
    other
        .set_global("jar", Value::NativeObject(std::rc::Rc::new(Fake)))
        .unwrap();
    fail(
        &mut other,
        "$result=Http::get('http://127.0.0.1:1',['cookieJar'=>$jar]);",
        "TypeError",
    );
    fail(
        &mut other,
        "$result=Http::closeCookieJar($jar);",
        "TypeError",
    );
    run(&mut other, "$jar=Http::cookieJar();");
    fail(
        &mut other,
        "$result=Http::get('http://127.0.0.1:1',['cookieJar'=>$jar,'headers'=>['Cookie'=>'manual=x']]);",
        "TypeError",
    );
    drop(owner);
    let Value::NativeObject(jar) = value else {
        panic!()
    };
    assert_eq!(jar.get_property("closed").unwrap(), Value::Bool(true));
}

#[test]
fn downloads_retry_before_transfer_and_keep_failed_destinations_absent() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("data.bin");
    let mut rt = runtime();
    rt.set_global("dest", Value::String(destination.to_str().unwrap().into()))
        .unwrap();
    let (url, worker) = server(2, |i, _, _| {
        if i == 0 {
            reply("503 Service Unavailable", "", b"no")
        } else {
            reply("200 OK", "", &[0, 255, 1, 2])
        }
    });
    set_url(&mut rt, &url);
    fail(
        &mut rt,
        "$result=Http::download($url,$dest,['maxBodyBytes'=>3,'retry'=>['maxRetries'=>1,'backoffMs'=>0]]);",
        "ResponseLimitError",
    );
    worker.join().unwrap();
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let (url, worker) = server(2, |i, _, _| {
        if i == 0 {
            reply("503 Service Unavailable", "", b"no")
        } else {
            reply("200 OK", "", &[0, 255, 1, 2])
        }
    });
    set_url(&mut rt, &url);
    run(
        &mut rt,
        "Http::download($url,$dest,['retry'=>['maxRetries'=>1,'backoffMs'=>0]]);",
    );
    worker.join().unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), [0, 255, 1, 2]);
    fail(
        &mut rt,
        "$result=Http::download('http://127.0.0.1:1',$dest,['retry'=>['maxRetries'=>1]]);",
        "AlreadyExistsError",
    );
    assert_eq!(std::fs::read(&destination).unwrap(), [0, 255, 1, 2]);
}

#[test]
fn enhanced_tls_still_verifies_certificates_and_pure_helpers_need_no_capabilities() {
    use std::sync::Arc;
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = generated.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der());
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut tls = rustls::StreamOwned::new(connection, stream);
        let mut buffer = [0; 1024];
        let _ = tls.read(&mut buffer);
    });
    let mut rt = runtime();
    set_url(&mut rt, &format!("https://localhost:{}", address.port()));
    run(&mut rt, "$jar=Http::cookieJar();");
    fail(
        &mut rt,
        "$result=Http::get($url,['cookieJar'=>$jar,'retry'=>['maxRetries'=>2]]);",
        "TlsError",
    );
    worker.join().unwrap();
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    run(
        &mut denied,
        "$jar=Http::cookieJar(); $form=Http::multipart([]); Http::closeCookieJar($jar);",
    );
    fail(
        &mut denied,
        "$result=Http::get('http://127.0.0.1:1',['retry'=>['maxRetries'=>1]]);",
        "PermissionError",
    );
}

#[test]
fn retries_cannot_reset_the_host_bounded_redirect_budget() {
    let (url, worker) = server(3, |i, _, _| {
        if i == 1 {
            reply("503 Service Unavailable", "", b"retry")
        } else {
            reply("302 Found", "Location: /next\r\n", b"")
        }
    });
    let mut rt = runtime();
    set_url(&mut rt, &url);
    fail(
        &mut rt,
        "$result=Http::get($url,['maxRedirects'=>1,'retry'=>['maxRetries'=>1,'backoffMs'=>0]]);",
        "RedirectError",
    );
    worker.join().unwrap();
}
