use super::*;
use scr_runtime::{MapKey, Runtime, RuntimeOptions};
use std::{
    io::Write,
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

fn parse(dsn: &str, options: Value, policy: &MysqlOptions) -> NativeResult<settings::Config> {
    settings::Config::parse(
        ConnectRequest {
            dsn,
            user: "fixture-user",
            password: "private-fixture-password",
            options: &options,
        },
        policy,
    )
}
fn options(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(k, v)| (MapKey::String((*k).into()), v.clone()))
            .collect(),
    )
}
fn setup(policy: MysqlOptions, runtime_options: RuntimeOptions) -> (Runtime, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let driver = Rc::new(MysqlDriver {
        options: Arc::new(policy.prepare().unwrap()),
        workers: count.clone(),
    });
    let mut runtime = Runtime::with_options(runtime_options);
    scr_db::register(&mut runtime, vec![driver]).unwrap();
    runtime
        .set_global("user", Value::String("fixture-user".into()))
        .unwrap();
    runtime
        .set_global("password", Value::String("private-fixture-password".into()))
        .unwrap();
    (runtime, count)
}
fn fails(
    runtime: &mut Runtime,
    dsn: &str,
    options: Value,
    kind: &str,
) -> scr_runtime::RuntimeError {
    runtime
        .set_global("dsn", Value::String(dsn.into()))
        .unwrap();
    runtime.set_global("options", options).unwrap();
    let error = runtime
        .execute_source(
            "$db=Db::connect($dsn,$user,$password,$options);",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.starts_with(kind), "{error:?}");
    assert!(!error.message.contains("private-fixture-password"));
    assert!(!error.message.contains("fixture-user"));
    error
}
fn drained(count: &AtomicUsize) {
    let until = Instant::now() + Duration::from_secs(3);
    while count.load(Ordering::Acquire) != 0 && Instant::now() < until {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(count.load(Ordering::Acquire), 0);
}

#[test]
fn dsn_and_options_are_strict_and_do_not_echo_untrusted_input() {
    let policy = MysqlOptions::default();
    for dsn in [
        "mysql:",
        "mysql:host=localhost",
        "mysql:dbname=test",
        "mysql:host=localhost;dbname=x;host=other",
        "mysql:host=localhost;dbname=x;port=0",
        "mysql:host=localhost;dbname=x;port=65536",
        "mysql:host=localhost;dbname=x;port=+1",
        "mysql:host=user@host;dbname=x",
        "mysql:host=localhost;dbname=x;password=private-fixture-password",
        "mysql:host=localhost;dbname=x;",
        "mysql:host=localhost;dbname=x\0",
    ] {
        let error = parse(dsn, Value::List(vec![]), &policy).err().unwrap();
        assert_eq!(error.kind, "TypeError", "{dsn}");
        assert!(!error.message.contains("private-fixture-password"));
    }
    let config = parse(
        "mysql:host=LOCALHOST;dbname=test",
        Value::List(vec![]),
        &policy,
    )
    .unwrap();
    assert_eq!(config.host, "localhost");
    assert_eq!(config.port, 3306);
    assert_eq!(config.attributes.operation_timeout_ms, 30000);
    for (name, value, kind) in [
        ("emulatePrepares", Value::Bool(true), "DbUnsupportedError"),
        ("emulatePrepares", Value::Int(0), "TypeError"),
        (
            "tlsMode",
            Value::String("disabled".into()),
            "DbUnsupportedError",
        ),
        (
            "errorMode",
            Value::String("silent".into()),
            "DbUnsupportedError",
        ),
        (
            "fetchMode",
            Value::String("object".into()),
            "DbUnsupportedError",
        ),
        ("connectTimeoutMs", Value::Int(0), "TypeError"),
        ("connectTimeoutMs", Value::Int(5001), "DbLimitError"),
        ("maxRows", Value::Int(10001), "DbLimitError"),
        ("maxResultBytes", Value::Float(1.0), "TypeError"),
        (
            "private-fixture-password",
            Value::Bool(false),
            "DbUnsupportedError",
        ),
    ] {
        let error = parse(
            "mysql:host=localhost;dbname=test",
            options(&[(name, value)]),
            &policy,
        )
        .err()
        .unwrap();
        assert_eq!(error.kind, kind);
        assert!(!error.message.contains("private-fixture-password"));
    }
}

#[test]
fn destination_policy_and_host_ceilings_cannot_be_changed_by_scripts() {
    let policy = MysqlOptions {
        destinations: Some(vec![Destination {
            host: "LOCALHOST".into(),
            port: 3306,
        }]),
        max_rows: 20,
        ..Default::default()
    }
    .prepare()
    .unwrap();
    let config = parse(
        "mysql:host=localhost;dbname=test",
        Value::List(vec![]),
        &policy,
    )
    .unwrap();
    assert_eq!(config.attributes.max_rows, 20);
    for dsn in [
        "mysql:host=localhost;port=3307;dbname=test",
        "mysql:host=127.0.0.1;dbname=test",
    ] {
        assert_eq!(
            parse(dsn, Value::List(vec![]), &policy).err().unwrap().kind,
            "PermissionError"
        );
    }
    assert!(
        driver_with_options(MysqlOptions {
            max_workers: 0,
            ..Default::default()
        })
        .is_err()
    );
    let mut attributes = config.attributes;
    attributes
        .set("operationTimeoutMs", &Value::Int(20), &policy, false)
        .unwrap();
    assert_eq!(
        attributes.get("operationTimeoutMs").unwrap(),
        Value::Int(20)
    );
    for (name, value) in [
        ("operationTimeoutMs", Value::Int(30001)),
        ("mysqlFoundRows", Value::Bool(true)),
        ("driverVersion", Value::String("bad".into())),
    ] {
        assert!(attributes.set(name, &value, &policy, false).is_err());
    }
    assert_eq!(
        attributes.get("operationTimeoutMs").unwrap(),
        Value::Int(20)
    );
    assert_eq!(
        attributes.get("mysqlFoundRows").unwrap(),
        Value::Bool(false)
    );
}

struct Peer {
    port: u16,
    stop: Option<mpsc::Sender<()>>,
    task: Option<thread::JoinHandle<()>>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.join().unwrap();
        }
    }
}
fn peer(packet: Option<Vec<u8>>) -> Peer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = mpsc::channel();
    let task = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if stopped.try_recv().is_ok() {
                return;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(packet) = packet {
                        let _ = stream.write_all(&packet);
                    }
                    let _ = stopped.recv_timeout(Duration::from_secs(5));
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    Peer {
        port,
        stop: Some(stop),
        task: Some(task),
    }
}
fn dsn(port: u16) -> String {
    format!("mysql:host=127.0.0.1;port={port};dbname=fixture")
}

#[test]
fn deadline_bounds_a_server_that_never_sends_a_handshake_and_resources_are_reaped() {
    let peer = peer(None);
    let (mut runtime, count) = setup(
        MysqlOptions {
            max_connect_timeout_ms: 200,
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    let start = Instant::now();
    fails(
        &mut runtime,
        &dsn(peer.port),
        options(&[("connectTimeoutMs", Value::Int(40))]),
        "DbTimeoutError",
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(runtime.get_global("db").is_none());
    drop(peer);
    drained(&count);
}

#[test]
fn cancelled_connections_keep_worker_slots_until_the_blocking_work_finishes() {
    let peer = peer(None);
    let flag = Arc::new(AtomicBool::new(false));
    let (mut runtime, count) = setup(
        MysqlOptions {
            max_workers: 1,
            max_connect_timeout_ms: 1000,
            ..Default::default()
        },
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
    );
    let interrupt = flag.clone();
    let timer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        interrupt.store(true, Ordering::Release);
    });
    let error = fails(
        &mut runtime,
        &dsn(peer.port),
        Value::List(vec![]),
        "Interrupted",
    );
    assert!(error.message.contains("interrupted"));
    timer.join().unwrap();
    flag.store(false, Ordering::Release);
    assert_eq!(count.load(Ordering::Acquire), 1);
    fails(
        &mut runtime,
        &dsn(peer.port),
        Value::List(vec![]),
        "DbLimitError",
    );
    drop(peer);
    drained(&count);
}

#[test]
fn malformed_and_denied_requests_never_start_workers() {
    let (mut runtime, count) = setup(
        MysqlOptions {
            destinations: Some(vec![]),
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    fails(
        &mut runtime,
        "mysql:host=localhost;dbname=x",
        Value::List(vec![]),
        "PermissionError",
    );
    fails(
        &mut runtime,
        "mysql:host=localhost;dbname=x;port=0",
        Value::List(vec![]),
        "TypeError",
    );
    assert_eq!(count.load(Ordering::Acquire), 0);
}

#[test]
fn ca_reads_reuse_filesystem_policy_and_are_bounded_before_starting_workers() {
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("test-ca.crt");
    std::fs::write(&ca, b"abcde").unwrap();
    let path = Value::String(ca.to_str().unwrap().into());
    let (mut runtime, count) = setup(
        MysqlOptions {
            max_ca_bytes: 4,
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    fails(
        &mut runtime,
        "mysql:host=localhost;dbname=x",
        options(&[("tlsCaFile", path.clone())]),
        "DbLimitError",
    );
    assert_eq!(count.load(Ordering::Acquire), 0);
    let (mut runtime, count) = setup(
        MysqlOptions {
            filesystem: scr_stdlib::FilesystemPolicy {
                roots: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
        RuntimeOptions::default(),
    );
    fails(
        &mut runtime,
        "mysql:host=localhost;dbname=x",
        options(&[("tlsCaFile", path)]),
        "PermissionError",
    );
    assert_eq!(count.load(Ordering::Acquire), 0);
    assert_eq!(std::fs::read(&ca).unwrap(), b"abcde");
}

#[test]
fn denied_filesystem_capability_and_connection_payload_fail_before_worker_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("test-ca.crt");
    std::fs::write(&ca, b"fixture").unwrap();
    let mut runtime_options = RuntimeOptions::default();
    runtime_options
        .context
        .permissions
        .allowed
        .remove("filesystem.read");
    let (mut runtime, count) = setup(MysqlOptions::default(), runtime_options);
    fails(
        &mut runtime,
        "mysql:host=localhost;dbname=x",
        options(&[("tlsCaFile", Value::String(ca.to_str().unwrap().into()))]),
        "PermissionError",
    );
    assert_eq!(count.load(Ordering::Acquire), 0);

    let (mut runtime, count) = setup(
        MysqlOptions::default(),
        RuntimeOptions {
            max_value_bytes: Some(std::mem::size_of::<Value>() + 51),
            ..Default::default()
        },
    );
    // Payload is host(9) + database(7) + user(12) + password(24) = 52 bytes.
    fails(
        &mut runtime,
        "mysql:host=127.0.0.1;dbname=fixture",
        Value::List(vec![]),
        "DbLimitError",
    );
    assert_eq!(count.load(Ordering::Acquire), 0);
}

#[test]
fn refused_tcp_connections_report_safe_errors_and_release_worker_slots() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (mut runtime, count) = setup(MysqlOptions::default(), RuntimeOptions::default());
    fails(&mut runtime, &dsn(port), Value::List(vec![]), "DbError");
    drained(&count);
}

#[test]
fn untrusted_server_messages_cannot_echo_credentials_into_errors() {
    let mut payload = vec![0xff, 0x15, 0x04, b'#'];
    payload.extend_from_slice(b"28000private-fixture-password fixture-user");
    let mut packet = vec![payload.len() as u8, 0, 0, 0];
    packet.extend_from_slice(&payload);
    let peer = peer(Some(packet));
    let (mut runtime, count) = setup(MysqlOptions::default(), RuntimeOptions::default());
    let error = fails(
        &mut runtime,
        &dsn(peer.port),
        Value::List(vec![]),
        "DbError",
    );
    assert!(error.message.contains("authentication"));
    assert!(error.message.contains("1045"));
    drop(peer);
    drained(&count);
}
