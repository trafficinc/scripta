use scr_runtime::{NativeObject, Runtime, RuntimeOptions, Value};
use scr_stdlib::{StdlibOptions, streams::StreamConfig};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    thread,
    time::{Duration, Instant},
};
fn runtime(options: StdlibOptions, limits: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(limits);
    scr_stdlib::register_with_options(&mut runtime, options).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), scr_runtime::RuntimeError> {
    runtime
        .execute_source(&format!("<?scripta {source}"), &mut vec![])
        .map(|_| ())
}
fn val(runtime: &Runtime, key: &str) -> Value {
    runtime.get_global(key).unwrap().clone()
}
fn bound(runtime: &mut Runtime, host: &str) -> std::net::SocketAddr {
    run(runtime,&format!("$server=Socket::listen(\"{host}\",0,[\"blocking\"=>false]);$local=Socket::localAddress($server);$port=$local[\"port\"];")).unwrap();
    let Value::Int(port) = val(runtime, "port") else {
        panic!()
    };
    format!(
        "{}:{port}",
        if host.contains(':') {
            format!("[{host}]")
        } else {
            host.into()
        }
    )
    .parse()
    .unwrap()
}
#[test]
fn tcp_listen_accept_poll_partial_io_shutdown_and_shared_stream_handles() {
    let mut runtime = runtime(StdlibOptions::default(), RuntimeOptions::default());
    let address = bound(&mut runtime, "127.0.0.1");
    assert!(
        run(&mut runtime, "Socket::accept($server);")
            .unwrap_err()
            .message
            .contains("WouldBlockError")
    );
    let mut peer = TcpStream::connect(address).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    peer.write_all(b"hi").unwrap();
    run(&mut runtime,r#"$ready=Socket::poll([$server],[],1000);$count=len($ready["readable"]);$client=Socket::accept($server);$data=Socket::read($client,10);Socket::setOption($client,"noDelay",true);$nodelay=Socket::getOption($client,"noDelay");Socket::write($client,Bytes::encodeUtf8("ok"));Socket::shutdown($client,"write");"#).unwrap();
    assert_eq!(val(&runtime, "count"), Value::Int(1));
    assert_eq!(val(&runtime, "data"), Value::Bytes(b"hi".to_vec().into()));
    assert_eq!(val(&runtime, "nodelay"), Value::Bool(true));
    let mut received = vec![];
    peer.read_to_end(&mut received).unwrap();
    assert_eq!(received, b"ok");
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    run(&mut runtime,r#"$r=Socket::poll([$client],[],1000);$end=Stream::read($client,1);$alias=$server;Socket::close($server);Socket::close($alias);Socket::close($client);$closed=$alias->closed;"#).unwrap();
    assert_eq!(val(&runtime, "end"), Value::Null);
    assert_eq!(val(&runtime, "closed"), Value::Bool(true));
}
#[test]
fn would_block_is_distinct_from_eof_and_poll_preserves_input_order_without_duplicates() {
    let mut runtime = runtime(StdlibOptions::default(), RuntimeOptions::default());
    let address = bound(&mut runtime, "127.0.0.1");
    let first = TcpStream::connect(address).unwrap();
    let second = TcpStream::connect(address).unwrap();
    run(
        &mut runtime,
        "$a=Socket::accept($server);$b=Socket::accept($server);",
    )
    .unwrap();
    assert!(
        run(&mut runtime, "Socket::read($a,1);")
            .unwrap_err()
            .message
            .contains("WouldBlockError")
    );
    run(&mut runtime,r#"$closed=$a->closed;$p=Socket::poll([$a],[$b,$a,$b],0);$n=len($p["writable"]);$order=$p["writable"][0] === $b;"#).unwrap();
    assert_eq!(val(&runtime, "closed"), Value::Bool(false));
    assert_eq!(val(&runtime, "n"), Value::Int(2));
    assert_eq!(val(&runtime, "order"), Value::Bool(true));
    drop((first, second));
}
#[test]
fn bound_udp_receives_empty_and_truncated_datagrams_and_replies_to_sender() {
    let mut runtime = runtime(StdlibOptions::default(), RuntimeOptions::default());
    run(&mut runtime,r#"$udp=Socket::bindDatagram("127.0.0.1",0,["blocking"=>false]);$address=Socket::localAddress($udp);$port=$address["port"];"#).unwrap();
    let Value::Int(port) = val(&runtime, "port") else {
        panic!()
    };
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert!(
        run(&mut runtime, "Socket::receive($udp,3);")
            .unwrap_err()
            .message
            .contains("WouldBlockError")
    );
    peer.send_to(b"", ("127.0.0.1", port as u16)).unwrap();
    run(
        &mut runtime,
        r#"Socket::poll([$udp],[],1000);$empty=Socket::receive($udp,3);"#,
    )
    .unwrap();
    let Value::Map(empty) = val(&runtime, "empty") else {
        panic!()
    };
    assert_eq!(empty[0].1, Value::Bytes(Vec::<u8>::new().into()));
    peer.send_to(b"abcdef", ("127.0.0.1", port as u16)).unwrap();
    run(&mut runtime,r#"Socket::poll([$udp],[],1000);$packet=Socket::receive($udp,3);Socket::sendTo($udp,$packet["data"],$packet["address"],$packet["port"]);$truncated=$packet["truncated"];"#).unwrap();
    let mut buffer = [0; 8];
    assert_eq!(peer.recv_from(&mut buffer).unwrap().0, 3);
    assert_eq!(&buffer[..3], b"abc");
    assert_eq!(val(&runtime, "truncated"), Value::Bool(true));
}
#[test]
fn socket_capabilities_quotas_conflicts_and_foreign_handles_fail_without_consuming_accepts() {
    let mut runtime = runtime(
        StdlibOptions {
            streams: StreamConfig {
                max_handles: 1,
                ..Default::default()
            },
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    let address = bound(&mut runtime, "127.0.0.1");
    let _peer = TcpStream::connect(address).unwrap();
    assert!(
        run(&mut runtime, "Socket::accept($server);")
            .unwrap_err()
            .message
            .contains("handle limit")
    );
    let mut denied = RuntimeOptions::default();
    denied.context.permissions.allowed.remove("network.listen");
    let mut denied_runtime = super_runtime(denied);
    assert!(
        run(&mut denied_runtime, "Socket::listen(\"127.0.0.1\",0);")
            .unwrap_err()
            .message
            .contains("PermissionError")
    );
    let mut other = super_runtime(RuntimeOptions::default());
    other
        .set_global("foreign", val(&runtime, "server"))
        .unwrap();
    assert!(
        run(&mut other, "Socket::close($foreign);")
            .unwrap_err()
            .message
            .contains("another runtime")
    );
    assert!(
        run(
            &mut other,
            &format!("Socket::listen(\"127.0.0.1\",{});", address.port())
        )
        .is_err()
    );
    #[derive(Debug)]
    struct Fake;
    impl NativeObject for Fake {
        fn type_name(&self) -> &str {
            "SocketListener"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("fake".into())
        }
    }
    other
        .set_global("fake", Value::NativeObject(std::rc::Rc::new(Fake)))
        .unwrap();
    assert!(
        run(&mut other, "Socket::accept($fake);")
            .unwrap_err()
            .message
            .contains("real stream")
    );
}
fn super_runtime(limits: RuntimeOptions) -> Runtime {
    runtime(StdlibOptions::default(), limits)
}
#[test]
fn connect_options_timeouts_and_cancellable_poll_are_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut runtime = super_runtime(RuntimeOptions::default());
    run(&mut runtime,&format!("$s=Socket::connect(\"127.0.0.1\",{port},[\"blocking\"=>false]);Socket::setTimeout($s,50);Socket::setOption($s,\"keepAlive\",true);$alive=Socket::getOption($s,\"keepAlive\");")).unwrap();
    let _peer = listener.accept().unwrap();
    assert_eq!(val(&runtime, "alive"), Value::Bool(true));
    let started = Instant::now();
    run(
        &mut runtime,
        "$p=Socket::poll([$s],[],30);$n=len($p[\"readable\"]);",
    )
    .unwrap();
    assert_eq!(val(&runtime, "n"), Value::Int(0));
    assert!(started.elapsed() < Duration::from_secs(2));
    for bad in [
        "Socket::setTimeout($s,0);",
        "Socket::getOption($s,\"unsupported\");",
        "Socket::setOption($s,\"noDelay\",1);",
        "Socket::poll([],[],0);",
        "Socket::poll([],[$s],-1);",
    ] {
        assert!(run(&mut runtime, bad).is_err());
    }
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut controlled = super_runtime(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    bound(&mut controlled, "127.0.0.1");
    let interrupt = flag.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        interrupt.store(true, Ordering::Release)
    });
    assert!(
        run(&mut controlled, "Socket::poll([$server],[],5000);")
            .unwrap_err()
            .message
            .contains("interrupted")
    );
    worker.join().unwrap();
}
#[test]
fn ipv6_loopback_is_supported_when_available() {
    let probe = match TcpListener::bind("[::1]:0") {
        Ok(socket) => socket,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::Unsupported
            ) =>
        {
            return;
        }
        Err(e) => panic!("IPv6 fixture failed: {e}"),
    };
    drop(probe);
    let mut runtime = super_runtime(RuntimeOptions::default());
    let address = bound(&mut runtime, "::1");
    let mut peer = TcpStream::connect(address).unwrap();
    peer.write_all(b"6").unwrap();
    run(&mut runtime,"Socket::poll([$server],[],1000);$c=Socket::accept($server);Socket::poll([$c],[],1000);$data=Socket::read($c,1);").unwrap();
    assert_eq!(val(&runtime, "data"), Value::Bytes(b"6".to_vec().into()));
}
#[test]
fn refused_connections_release_quota_and_full_quota_does_not_dequeue_a_client() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("dummy"), b"x").unwrap();
    let mut runtime = runtime(
        StdlibOptions {
            streams: StreamConfig {
                max_handles: 2,
                ..Default::default()
            },
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    let address = bound(&mut runtime, "127.0.0.1");
    runtime
        .set_global(
            "path",
            Value::String(directory.path().join("dummy").to_str().unwrap().into()),
        )
        .unwrap();
    run(&mut runtime, "$dummy=Stream::open($path);").unwrap();
    let mut peer = TcpStream::connect(address).unwrap();
    peer.write_all(b"q").unwrap();
    assert!(
        run(&mut runtime, "Socket::accept($server);")
            .unwrap_err()
            .message
            .contains("handle limit")
    );
    run(&mut runtime,"Stream::close($dummy);$client=Socket::accept($server);Socket::poll([$client],[],1000);$data=Socket::read($client,1);").unwrap();
    assert_eq!(val(&runtime, "data"), Value::Bytes(b"q".to_vec().into()));
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    run(&mut runtime, "Socket::close($client);").unwrap();
    assert!(
        run(
            &mut runtime,
            &format!("Socket::connect(\"127.0.0.1\",{port},[\"timeoutMs\"=>100]);")
        )
        .is_err()
    );
    run(
        &mut runtime,
        "$u=Socket::bindDatagram(\"127.0.0.1\",0);Socket::close($u);",
    )
    .unwrap();
}
