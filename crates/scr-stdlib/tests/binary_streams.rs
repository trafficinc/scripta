use scr_runtime::{ContextOptions, NativeObject, Runtime, RuntimeOptions, Value};
use scr_stdlib::{FilesystemPolicy, StdlibOptions, streams::StreamConfig};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, UdpSocket},
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn runtime(directory: &Path, options: StdlibOptions, limits: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: directory.into(),
            ..limits.context
        },
        ..limits
    });
    scr_stdlib::register_with_options(&mut runtime, options).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), scr_runtime::RuntimeError> {
    runtime
        .execute_source(&format!("<?scripta {source}"), &mut vec![])
        .map(|_| ())
}
fn assert_global(runtime: &Runtime, name: &str, value: Value) {
    assert_eq!(runtime.get_global(name), Some(&value), "{name}")
}
#[test]
fn binary_file_modes_seek_copy_eof_and_alias_close_preserve_bytes() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("in"), b"\0\xffabc").unwrap();
    let mut runtime = runtime(
        directory.path(),
        StdlibOptions::default(),
        RuntimeOptions::default(),
    );
    run(&mut runtime,r#"$s=Stream::open("in");$out=Stream::open("out","x+");$n=Stream::copy($s,$out,5);$eof=Stream::read($s,1);$at=Stream::tell($out);Stream::seek($out,0);$data=Stream::read($out,5);$alias=$out;Stream::close($out);$closed=$alias->closed;Stream::close($alias);Stream::close($s);"#).unwrap();
    assert_eq!(
        fs::read(directory.path().join("out")).unwrap(),
        b"\0\xffabc"
    );
    assert_global(&runtime, "n", Value::Int(5));
    assert_global(&runtime, "at", Value::Int(5));
    assert_global(&runtime, "eof", Value::Null);
    assert_global(&runtime, "closed", Value::Bool(true));
    assert_global(&runtime, "data", Value::Bytes(b"\0\xffabc".to_vec().into()));
    assert!(
        run(&mut runtime, "Stream::read($alias,1);")
            .unwrap_err()
            .message
            .contains("closed")
    );
    run(&mut runtime,r#"$a=Stream::open("out","a+");Stream::seek($a,0);Stream::write($a,Bytes::encodeUtf8("tail"));Stream::close($a);"#).unwrap();
    assert_eq!(
        fs::read(directory.path().join("out")).unwrap(),
        b"\0\xffabctail"
    );
    run(&mut runtime,r#"$w=Stream::open("out","w+");$zero=Stream::read($w,0);Stream::write($w,Bytes::encodeUtf8("new"));Stream::flush($w);Stream::close($w);"#).unwrap();
    assert_eq!(fs::read(directory.path().join("out")).unwrap(), b"new");
    assert_global(&runtime, "zero", Value::Bytes(Vec::<u8>::new().into()));
}
#[test]
fn modes_permissions_roots_symlinks_and_result_budgets_fail_before_mutation() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("original"), b"keep").unwrap();
    for policy in [
        FilesystemPolicy {
            write: false,
            ..Default::default()
        },
        FilesystemPolicy {
            roots: Some(vec![]),
            ..Default::default()
        },
    ] {
        let mut runtime = runtime(
            directory.path(),
            StdlibOptions {
                filesystem: policy,
                ..Default::default()
            },
            RuntimeOptions::default(),
        );
        assert!(run(&mut runtime, "Stream::open(\"original\",\"w\");").is_err());
        assert_eq!(
            fs::read(directory.path().join("original")).unwrap(),
            b"keep"
        );
    }
    let mut runtime = runtime(
        directory.path(),
        StdlibOptions::default(),
        RuntimeOptions::default(),
    );
    for source in [
        "Stream::open(\"original\",\"x\");",
        "Stream::open(\"original\",\"bad\");",
        "Stream::open(\"original\",\"w\",[\"bogus\"=>1]);",
        "Stream::open(\"unknown://file\",\"w\");",
    ] {
        assert!(run(&mut runtime, source).is_err());
        assert_eq!(
            fs::read(directory.path().join("original")).unwrap(),
            b"keep"
        )
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            directory.path().join("original"),
            directory.path().join("link"),
        )
        .unwrap();
        assert!(run(&mut runtime, "Stream::open(\"link\",\"w\");").is_err());
        assert_eq!(
            fs::read(directory.path().join("original")).unwrap(),
            b"keep"
        );
    }
    let original_permissions = fs::metadata(directory.path().join("original"))
        .unwrap()
        .permissions();
    let mut permissions = original_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(directory.path().join("original"), permissions).unwrap();
    assert!(
        run(&mut runtime, "Stream::open(\"original\",\"w\");")
            .unwrap_err()
            .message
            .contains("read-only")
    );
    fs::set_permissions(directory.path().join("original"), original_permissions).unwrap();
    let mut small = super_runtime(
        directory.path(),
        RuntimeOptions {
            max_value_bytes: Some(size_of::<Value>()),
            ..Default::default()
        },
    );
    assert!(run(&mut small, "Stream::open(\"original\",\"w\");").is_err());
    assert_eq!(
        fs::read(directory.path().join("original")).unwrap(),
        b"keep"
    );
}
fn super_runtime(directory: &Path, limits: RuntimeOptions) -> Runtime {
    runtime(directory, StdlibOptions::default(), limits)
}
#[test]
fn exact_transfer_limits_handle_quotas_and_foreign_runtime_checks() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("four"), b"1234").unwrap();
    fs::write(directory.path().join("five"), b"12345").unwrap();
    let mut runtime = runtime(
        directory.path(),
        StdlibOptions {
            streams: StreamConfig {
                max_handles: 1,
                max_transfer_bytes: 4,
                max_chunk_bytes: 4,
                ..Default::default()
            },
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    run(&mut runtime,r#"$f=Stream::open("four");$data=Stream::read($f,4);$eof=Stream::read($f,1);$seen=Stream::eof($f);"#).unwrap();
    assert_global(&runtime, "eof", Value::Null);
    assert_global(&runtime, "seen", Value::Bool(true));
    assert!(
        run(&mut runtime, "Stream::open(\"new\",\"w\");")
            .unwrap_err()
            .message
            .contains("handle limit")
    );
    assert!(!directory.path().join("new").exists());
    run(
        &mut runtime,
        r#"Stream::close($f);$f=Stream::open("five");$first=Stream::read($f,4);$value=42;"#,
    )
    .unwrap();
    assert!(
        run(&mut runtime, "$value=Stream::read($f,1);")
            .unwrap_err()
            .message
            .contains("StreamLimitError")
    );
    assert_global(&runtime, "value", Value::Int(42));
    run(&mut runtime, "$closed=$f->closed;").unwrap();
    assert_global(&runtime, "closed", Value::Bool(true));
    run(&mut runtime, "$f=Stream::open(\"four\");").unwrap();
    let handle = runtime.get_global("f").unwrap().clone();
    let mut other = super_runtime(directory.path(), RuntimeOptions::default());
    other.set_global("f", handle).unwrap();
    assert!(
        run(&mut other, "Stream::read($f,1);")
            .unwrap_err()
            .message
            .contains("another runtime")
    );
    #[derive(Debug)]
    struct Fake;
    impl NativeObject for Fake {
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("no properties".into())
        }
        fn type_name(&self) -> &str {
            "ByteStream"
        }
    }
    other
        .set_global("fake", Value::NativeObject(std::rc::Rc::new(Fake)))
        .unwrap();
    assert!(
        run(&mut other, "Stream::read($fake,1);")
            .unwrap_err()
            .message
            .contains("real stream")
    );
}
#[test]
fn local_file_wrapper_decodes_explicit_paths_and_read_budgets_do_not_advance_cursor() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("a b"), vec![7; 200]).unwrap();
    let uri = format!(
        "file://{}{}/a%20b",
        if cfg!(windows) { "/" } else { "" },
        directory.path().to_str().unwrap().replace('\\', "/")
    );
    let mut runtime = super_runtime(
        directory.path(),
        RuntimeOptions {
            max_value_bytes: Some(1024),
            ..Default::default()
        },
    );
    runtime.set_global("uri", Value::String(uri)).unwrap();
    run(&mut runtime, "$s=Stream::open($uri);").unwrap();
    assert!(run(&mut runtime, "Stream::read($s,2048);").is_err());
    run(&mut runtime, "$pos=Stream::tell($s);").unwrap();
    assert_global(&runtime, "pos", Value::Int(0));
}
fn accept(listener: &TcpListener) -> std::net::TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                return socket;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "local fixture accept timed out");
                thread::sleep(Duration::from_millis(2))
            }
            Err(e) => panic!("{e}"),
        }
    }
}
fn request(reader: &mut BufReader<std::net::TcpStream>) -> (String, Vec<u8>) {
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty());
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line)
    }
    let mut body = vec![];
    if headers
        .to_lowercase()
        .contains("transfer-encoding: chunked")
    {
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let size = usize::from_str_radix(line.trim(), 16).unwrap();
            if size == 0 {
                reader.read_line(&mut String::new()).unwrap();
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            reader.read_exact(&mut body[start..]).unwrap();
            reader.read_exact(&mut [0; 2]).unwrap();
        }
    } else if let Some(length) = headers.lines().find_map(|l| {
        l.to_ascii_lowercase()
            .strip_prefix("content-length:")
            .map(|v| v.trim().parse::<usize>().unwrap())
    }) {
        body.resize(length, 0);
        reader.read_exact(&mut body).unwrap();
    }
    (headers, body)
}
#[test]
fn http_response_reads_are_incremental_binary_and_cannot_seek_or_write() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let socket = accept(&listener);
        let mut socket = BufReader::new(socket);
        request(&mut socket);
        socket.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\nX-Test: yes\r\n\r\n\xff\0").unwrap();
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        socket.get_mut().write_all(b"ab").unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global("url", Value::String(format!("http://{address}/")))
        .unwrap();
    run(
        &mut runtime,
        r#"$s=Stream::open($url);$status=$s->status;$first=Stream::read($s,2);"#,
    )
    .unwrap();
    assert_global(&runtime, "status", Value::Int(200));
    assert_global(&runtime, "first", Value::Bytes(b"\xff\0".to_vec().into()));
    assert!(run(&mut runtime, "Stream::seek($s,0);").is_err());
    assert!(run(&mut runtime, "Stream::write($s,Bytes::encodeUtf8(\"x\"));").is_err());
    release.send(()).unwrap();
    run(
        &mut runtime,
        "$second=Stream::read($s,2);$eof=Stream::read($s,1);Stream::close($s);",
    )
    .unwrap();
    assert_global(&runtime, "second", Value::Bytes(b"ab".to_vec().into()));
    assert_global(&runtime, "eof", Value::Null);
    worker.join().unwrap();
}
#[test]
fn http_upload_streams_a_source_and_does_not_follow_body_redirects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        for status in [200, 307] {
            let socket = accept(&listener);
            let mut reader = BufReader::new(socket);
            let (headers, body) = request(&mut reader);
            assert!(headers.starts_with("POST /"));
            assert_eq!(body, b"\xff\0abc");
            reader.get_mut().write_all(format!("HTTP/1.1 {status} OK\r\nLocation: /elsewhere\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").as_bytes()).unwrap();
        }
    });
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"\xff\0abc").unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global("url", Value::String(format!("http://{address}/")))
        .unwrap();
    run(&mut runtime,r#"$source=Stream::open("source");$response=Stream::request("POST",$url,$source);$status=$response->status;$body=Stream::read($response,2);Stream::close($response);Stream::seek($source,0);$r=Stream::request("POST",$url,$source);$redirect=$r->status;Stream::close($r);Stream::close($source);"#).unwrap();
    assert_global(&runtime, "body", Value::Bytes(b"ok".to_vec().into()));
    assert_global(&runtime, "redirect", Value::Int(307));
    worker.join().unwrap();
}
#[test]
fn http_stream_redirects_and_response_limits_match_the_buffered_client() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        for index in 0..3 {
            let socket = accept(&listener);
            let mut reader = BufReader::new(socket);
            request(&mut reader);
            let response = if index == 0 {
                b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345".as_slice()
            };
            reader.get_mut().write_all(response).unwrap();
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global("url", Value::String(format!("http://{address}/")))
        .unwrap();
    run(
        &mut runtime,
        "$s=Stream::open($url);$status=$s->status;$data=Stream::read($s,5);Stream::close($s);",
    )
    .unwrap();
    assert_global(&runtime, "status", Value::Int(200));
    run(
        &mut runtime,
        "$s=Stream::open($url,\"r\",[\"maxBodyBytes\"=>4]);",
    )
    .unwrap();
    assert!(
        run(&mut runtime, "Stream::read($s,4);Stream::read($s,1);")
            .unwrap_err()
            .message
            .contains("LimitError")
    );
    worker.join().unwrap();
}
#[test]
fn tcp_streams_preserve_partial_io_eof_and_enforce_timeout_and_permission() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut socket = accept(&listener);
        let mut data = [0; 2];
        socket.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"hi");
        socket.write_all(b"\xff\0").unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global("url", Value::String(format!("tcp://{address}")))
        .unwrap();
    run(&mut runtime,"$s=Stream::open($url);$n=Stream::write($s,Bytes::encodeUtf8(\"hi\"));$data=Stream::read($s,8);$eof=Stream::read($s,1);Stream::close($s);").unwrap();
    assert_global(&runtime, "n", Value::Int(2));
    assert_global(&runtime, "data", Value::Bytes(b"\xff\0".to_vec().into()));
    assert_global(&runtime, "eof", Value::Null);
    worker.join().unwrap();
    let mut limits = RuntimeOptions::default();
    limits.context.permissions.allowed.remove("network.connect");
    let mut denied = super_runtime(directory.path(), limits);
    assert!(
        run(&mut denied, "Stream::open(\"tcp://127.0.0.1:9\");")
            .unwrap_err()
            .message
            .contains("PermissionError")
    );
    assert!(
        run(&mut runtime, "Stream::open(\"tcp://localhost:80\");")
            .unwrap_err()
            .message
            .contains("literal IP")
    );
}
#[test]
fn udp_keeps_empty_packets_binary_payloads_and_truncation_boundaries() {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let address = socket.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let mut buffer = [0; 64];
        for _ in 0..2 {
            let (n, peer) = socket.recv_from(&mut buffer).unwrap();
            socket.send_to(&buffer[..n], peer).unwrap();
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global("url", Value::String(format!("udp://{address}")))
        .unwrap();
    run(&mut runtime,r#"$s=Stream::open($url,"rw",["timeoutMs"=>100]);Stream::send($s,Bytes::encodeUtf8(""));$empty=Stream::receive($s,4);Stream::send($s,Bytes::encodeUtf8("abcdef"));$short=Stream::receive($s,3);"#).unwrap();
    let Some(Value::Map(empty)) = runtime.get_global("empty") else {
        panic!()
    };
    assert_eq!(empty[0].1, Value::Bytes(Vec::<u8>::new().into()));
    let Some(Value::Map(short)) = runtime.get_global("short") else {
        panic!()
    };
    assert_eq!(short[0].1, Value::Bytes(b"abc".to_vec().into()));
    assert_eq!(short[3].1, Value::Bool(true));
    assert!(
        run(&mut runtime, "Stream::receive($s,3);")
            .unwrap_err()
            .message
            .contains("TimeoutError")
    );
    worker.join().unwrap();
}
#[test]
fn blocked_tcp_reads_observe_cancellation_and_close_the_handle() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let _socket = accept(&listener);
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    let flag = Arc::new(AtomicBool::new(false));
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(
        directory.path(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
    );
    runtime
        .set_global("url", Value::String(format!("tcp://{address}")))
        .unwrap();
    run(&mut runtime, "$s=Stream::open($url);").unwrap();
    let interrupt = flag.clone();
    let interrupter = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        interrupt.store(true, Ordering::Release)
    });
    let started = Instant::now();
    assert!(
        run(&mut runtime, "Stream::read($s,1);")
            .unwrap_err()
            .message
            .contains("interrupted")
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    interrupter.join().unwrap();
    flag.store(false, Ordering::Release);
    run(&mut runtime, "$closed=$s->closed;").unwrap();
    assert_global(&runtime, "closed", Value::Bool(true));
    release.send(()).unwrap();
    worker.join().unwrap();
}
#[test]
fn https_stream_rejects_untrusted_certificates_and_cannot_disable_verification() {
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        certificate.key_pair.serialize_der(),
    ));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.cert.der().clone()], key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let socket = accept(&listener);
        let connection = rustls::ServerConnection::new(std::sync::Arc::new(config)).unwrap();
        let mut socket = rustls::StreamOwned::new(connection, socket);
        let _ = socket.read(&mut [0; 128]);
    });
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global(
            "url",
            Value::String(format!("https://localhost:{}/", address.port())),
        )
        .unwrap();
    assert!(
        run(
            &mut runtime,
            "Stream::open($url,\"r\",[\"timeoutMs\"=>1000]);"
        )
        .unwrap_err()
        .message
        .contains("TlsError")
    );
    worker.join().unwrap();
    assert!(
        run(
            &mut runtime,
            "Stream::open($url,\"r\",[\"verifyTls\"=>false]);"
        )
        .unwrap_err()
        .message
        .contains("unknown stream option")
    );
}
#[test]
fn file_copy_validation_is_atomic_and_native_teardown_releases_handle_quota() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"12345").unwrap();
    let mut runtime = runtime(
        directory.path(),
        StdlibOptions {
            streams: StreamConfig {
                max_handles: 2,
                ..Default::default()
            },
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    run(
        &mut runtime,
        r#"$source=Stream::open("source");$out=Stream::open("out","x",["maxBytes"=>4]);"#,
    )
    .unwrap();
    assert!(
        run(&mut runtime, "Stream::copy($source,$out,5);")
            .unwrap_err()
            .message
            .contains("StreamLimitError")
    );
    assert!(fs::read(directory.path().join("out")).unwrap().is_empty());
    run(&mut runtime, "$position=Stream::tell($source);").unwrap();
    assert_global(&runtime, "position", Value::Int(0));
    run(&mut runtime,r#"Stream::close($source);Stream::close($out);for ($i=0;$i<20;$i+=1) {$f=Stream::open("source");Stream::close($f);}"#).unwrap();
    drop(runtime);
    fs::remove_file(directory.path().join("source")).unwrap();
}
#[test]
fn tcp_timeout_is_bounded_and_http_upload_rejects_network_sources_before_requesting() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut runtime = super_runtime(directory.path(), RuntimeOptions::default());
    runtime
        .set_global("url", Value::String(format!("tcp://{address}")))
        .unwrap();
    run(
        &mut runtime,
        "$s=Stream::open($url,\"rw\",[\"timeoutMs\"=>30]);",
    )
    .unwrap();
    let _socket = accept(&listener);
    assert!(
        run(
            &mut runtime,
            "Stream::request(\"POST\",\"http://127.0.0.1:9/\",$s);"
        )
        .unwrap_err()
        .message
        .contains("readable file stream")
    );
    let started = Instant::now();
    assert!(
        run(&mut runtime, "Stream::read($s,1);")
            .unwrap_err()
            .message
            .contains("TimeoutError")
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    run(&mut runtime, "$closed=$s->closed;").unwrap();
    assert_global(&runtime, "closed", Value::Bool(true));
}
#[test]
fn large_copy_uses_bounded_working_memory_and_reads_preflight_exact_result_limits() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("large"), vec![0xa5; 1024 * 1024]).unwrap();
    let mut runtime = super_runtime(
        directory.path(),
        RuntimeOptions {
            max_value_bytes: Some(1024),
            ..Default::default()
        },
    );
    run(&mut runtime,r#"$r=Stream::open("large");$w=Stream::open("copy","x");$n=Stream::copy($r,$w,1048576);Stream::close($r);Stream::close($w);"#).unwrap();
    assert_global(&runtime, "n", Value::Int(1048576));
    assert_eq!(
        fs::read(directory.path().join("copy")).unwrap(),
        vec![0xa5; 1024 * 1024]
    );
    for succeeds in [true, false] {
        let mut runtime = super_runtime(
            directory.path(),
            RuntimeOptions {
                max_value_bytes: Some(size_of::<Value>() + 128 - usize::from(!succeeds)),
                ..Default::default()
            },
        );
        run(&mut runtime, "$s=Stream::open(\"large\");$result=42;").unwrap();
        let result = run(&mut runtime, "$result=Stream::read($s,128);");
        assert_eq!(result.is_ok(), succeeds);
        if !succeeds {
            assert_global(&runtime, "result", Value::Int(42));
            run(&mut runtime, "$at=Stream::tell($s);").unwrap();
            assert_global(&runtime, "at", Value::Int(0));
        }
    }
}
