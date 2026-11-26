use mysql::prelude::Queryable;
use scr_ext_mysql::{Destination, MysqlOptions};
use scr_runtime::{MapKey, Runtime, Value};
use std::{
    env,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    dsn: String,
    user: String,
    password: String,
    ca: String,
    port: u16,
}
impl Fixture {
    fn load() -> Self {
        Self {
            dsn: env::var("SCR_MYSQL_FIXTURE_DSN").expect("use tests/support/mysql_fixture.py"),
            user: env::var("SCR_MYSQL_FIXTURE_USER").unwrap(),
            password: env::var("SCR_MYSQL_FIXTURE_PASSWORD").unwrap(),
            ca: env::var("SCR_MYSQL_FIXTURE_CA").unwrap(),
            port: env::var("SCR_MYSQL_FIXTURE_PORT").unwrap().parse().unwrap(),
        }
    }
    fn runtime(&self, policy: MysqlOptions, max_connections: usize) -> Runtime {
        let mut runtime = Runtime::new();
        scr_db::register_with_options(
            &mut runtime,
            vec![scr_ext_mysql::driver_with_options(policy).unwrap()],
            scr_db::DatabaseOptions {
                max_connections,
                ..Default::default()
            },
        )
        .unwrap();
        for (name, value) in [
            ("dsn", &self.dsn),
            ("user", &self.user),
            ("password", &self.password),
        ] {
            runtime
                .set_global(name, Value::String(value.clone()))
                .unwrap();
        }
        runtime
            .set_global(
                "options",
                Value::map(vec![(
                    MapKey::String("tlsCaFile".into()),
                    Value::String(self.ca.clone()),
                )]),
            )
            .unwrap();
        runtime
    }
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime.execute_source(source, &mut Vec::new()).unwrap();
}
fn fails(runtime: &mut Runtime, source: &str, kind: &str) -> String {
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with(kind), "{error:?}");
    assert!(
        !error
            .message
            .contains("fixture-password-not-for-production")
    );
    error.message
}
fn sessions(observer: &mut mysql::Conn) -> u64 {
    observer
        .query_first(
            "SELECT COUNT(*) FROM information_schema.processlist WHERE USER = 'scripta_fixture'",
        )
        .unwrap()
        .unwrap()
}
fn wait_sessions(observer: &mut mysql::Conn, expected: u64) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let count = sessions(observer);
        if count == expected {
            return;
        }
        assert!(
            Instant::now() < until,
            "expected {expected} sessions, got {count}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture; run tests/support/mysql_fixture.py"]
fn verified_tls_authentication_attributes_and_resource_lifecycle() {
    let fixture = Fixture::load();
    let mut observer = mysql::Conn::new(
        mysql::OptsBuilder::new()
            .ip_or_hostname(Some("localhost"))
            .tcp_port(fixture.port)
            .prefer_socket(false)
            .bind_address(Some("0.0.0.0:0".parse::<std::net::SocketAddr>().unwrap()))
            .tcp_connect_timeout(Some(Duration::from_secs(2)))
            .read_timeout(Some(Duration::from_secs(2)))
            .write_timeout(Some(Duration::from_secs(2)))
            .user(Some(&fixture.user))
            .pass(Some(&fixture.password))
            .db_name(Some("scripta_fixture"))
            .ssl_opts(
                mysql::SslOpts::default().with_root_cert_path(Some(PathBuf::from(&fixture.ca))),
            ),
    )
    .unwrap();
    wait_sessions(&mut observer, 1);
    let policy = MysqlOptions {
        destinations: Some(vec![Destination {
            host: "localhost".into(),
            port: fixture.port,
        }]),
        filesystem: scr_stdlib::FilesystemPolicy {
            roots: Some(vec![
                PathBuf::from(&fixture.ca).parent().unwrap().to_path_buf(),
            ]),
            ..Default::default()
        },
        max_ca_bytes: std::fs::metadata(&fixture.ca).unwrap().len() as usize,
        max_workers: 1,
        ..Default::default()
    };
    let mut runtime = fixture.runtime(policy.clone(), 1);
    run(
        &mut runtime,
        "$db=Db::connect($dsn,$user,$password,$options);$driver=Db::getAttribute($db,'driverName');Db::setAttribute($db,'operationTimeoutMs',100);$timeout=Db::getAttribute($db,'operationTimeoutMs');",
    );
    assert_eq!(
        runtime.get_global("driver"),
        Some(&Value::String("mysql".into()))
    );
    assert_eq!(runtime.get_global("timeout"), Some(&Value::Int(100)));
    wait_sessions(&mut observer, 2);
    fails(
        &mut runtime,
        "Db::connect($dsn,$user,$password,$options);",
        "DbLimitError",
    );
    fails(
        &mut runtime,
        "Db::setAttribute($db,'operationTimeoutMs',30001);",
        "DbLimitError",
    );
    fails(
        &mut runtime,
        "Db::setAttribute($db,'emulatePrepares',true);",
        "DbUnsupportedError",
    );
    fails(
        &mut runtime,
        "Db::setAttribute($db,'tlsMode','disabled');",
        "DbStateError",
    );
    fails(
        &mut runtime,
        "Db::setAttribute($db,'driverVersion','bad');",
        "DbUnsupportedError",
    );
    run(
        &mut runtime,
        "$timeout=Db::getAttribute($db,'operationTimeoutMs');Db::close($db);Db::close($db);$closed=$db->closed;",
    );
    assert_eq!(runtime.get_global("timeout"), Some(&Value::Int(100)));
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    wait_sessions(&mut observer, 1);
    fails(
        &mut runtime,
        "Db::getAttribute($db,'driverName');",
        "DbStateError",
    );
    run(
        &mut runtime,
        "$db=Db::connect($dsn,$user,$password,$options);",
    );
    let retained = runtime.get_global("db").unwrap().clone();
    drop(runtime);
    wait_sessions(&mut observer, 2);
    drop(retained);
    wait_sessions(&mut observer, 1);

    let mut runtime = fixture.runtime(policy.clone(), 1);
    fails(
        &mut runtime,
        "Db::connect($dsn,$user,'incorrect-fixture-password',$options);",
        "DbError",
    );
    fails(
        &mut runtime,
        "Db::connect($dsn,$user,$password);",
        "DbError",
    ); // untrusted CA
    let wrong_host = format!(
        "mysql:host=127.0.0.1;port={};dbname=scripta_fixture",
        fixture.port
    );
    let mut unrestricted = fixture.runtime(MysqlOptions::default(), 1);
    unrestricted
        .set_global("dsn", Value::String(wrong_host))
        .unwrap();
    fails(
        &mut unrestricted,
        "Db::connect($dsn,$user,$password,$options);",
        "DbError",
    ); // cert is DNS:localhost only
    wait_sessions(&mut observer, 1);

    let mut too_small = policy;
    too_small.max_ca_bytes -= 1;
    let mut runtime = fixture.runtime(too_small, 1);
    fails(
        &mut runtime,
        "Db::connect($dsn,$user,$password,$options);",
        "DbLimitError",
    );
    wait_sessions(&mut observer, 1);
}
