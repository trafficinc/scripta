use mysql::prelude::Queryable;
use scr_ext_mysql::MysqlOptions;
use scr_runtime::{MapKey, Runtime, RuntimeOptions, Value};
use std::{env, path::PathBuf, time::Duration};

struct Fixture {
    runtime: Runtime,
    observer: mysql::Conn,
}
impl Fixture {
    fn open(policy: MysqlOptions, runtime_options: RuntimeOptions, max_statements: usize) -> Self {
        let ca = env::var("SCR_MYSQL_FIXTURE_CA").expect("run tests/support/mysql_fixture.py");
        let user = env::var("SCR_MYSQL_FIXTURE_USER").unwrap();
        let password = env::var("SCR_MYSQL_FIXTURE_PASSWORD").unwrap();
        let port = env::var("SCR_MYSQL_FIXTURE_PORT")
            .unwrap()
            .parse::<u16>()
            .unwrap();
        let observer = mysql::Conn::new(
            mysql::OptsBuilder::new()
                .ip_or_hostname(Some("localhost"))
                .tcp_port(port)
                .prefer_socket(false)
                .bind_address(Some("0.0.0.0:0".parse::<std::net::SocketAddr>().unwrap()))
                .tcp_connect_timeout(Some(Duration::from_secs(2)))
                .read_timeout(Some(Duration::from_secs(3)))
                .write_timeout(Some(Duration::from_secs(3)))
                .user(Some(&user))
                .pass(Some(&password))
                .db_name(Some("scripta_fixture"))
                .ssl_opts(mysql::SslOpts::default().with_root_cert_path(Some(PathBuf::from(&ca)))),
        )
        .unwrap();
        let mut runtime = Runtime::with_options(runtime_options);
        scr_db::register_with_options(
            &mut runtime,
            vec![scr_ext_mysql::driver_with_options(policy).unwrap()],
            scr_db::DatabaseOptions {
                max_statements_per_connection: max_statements,
                ..Default::default()
            },
        )
        .unwrap();
        for (name, value) in [
            ("dsn", env::var("SCR_MYSQL_FIXTURE_DSN").unwrap()),
            ("user", user),
            ("password", password),
        ] {
            runtime.set_global(name, Value::String(value)).unwrap();
        }
        runtime
            .set_global(
                "options",
                Value::Map(vec![(
                    MapKey::String("tlsCaFile".into()),
                    Value::String(ca),
                )]),
            )
            .unwrap();
        run(
            &mut runtime,
            "$db=Db::connect($dsn,$user,$password,$options);",
        );
        Self { runtime, observer }
    }
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime.execute_source(source, &mut Vec::new()).unwrap();
}
fn fails(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with(kind), "{error:?}");
    assert!(
        !error
            .message
            .contains("fixture-password-not-for-production")
    );
    assert!(error.source.is_some());
}
fn field<'a>(runtime: &'a Runtime, row: &str, name: &str) -> &'a Value {
    let Some(Value::Map(entries)) = runtime.get_global(row) else {
        panic!("expected row map")
    };
    &entries
        .iter()
        .find(|(key, _)| *key == MapKey::String(name.into()))
        .expect("missing result field")
        .1
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn structured_errors_preserve_safe_codes_and_statement_connection_reset_rules() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE error_rows (n INT PRIMARY KEY) ENGINE=InnoDB');$s=Db::prepare($db,'INSERT INTO error_rows VALUES (?)');$initial=Db::errorInfo($db);Db::execute($s,[1]);",
    );
    assert_eq!(f.runtime.get_global("initial"), Some(&Value::Null));
    fails(&mut f.runtime, "Db::execute($s,[1]);", "DbError");
    run(
        &mut f.runtime,
        "$info=Db::errorInfo($s);$connection=Db::errorInfo($db);Db::getAttribute($db,'driverName');Db::lastInsertId($db);Db::inTransaction($db);$inspected=Db::errorInfo($db);",
    );
    for (key, expected) in [
        ("operation", "execute"),
        ("category", "constraint"),
        ("sqlState", "23000"),
        ("driverCode", "1062"),
    ] {
        assert_eq!(
            field(&f.runtime, "info", key),
            &Value::String(expected.into())
        );
    }
    assert_eq!(
        f.runtime.get_global("info"),
        f.runtime.get_global("connection")
    );
    assert_eq!(
        f.runtime.get_global("info"),
        f.runtime.get_global("inspected")
    );
    fails(
        &mut f.runtime,
        "Db::execute($s,[1.0/0.0]);",
        "ArithmeticError",
    );
    fails(&mut f.runtime, "Db::execute($s,[[]]);", "TypeError");
    run(
        &mut f.runtime,
        "Db::closeCursor($s);$preserved=Db::errorInfo($s);",
    );
    assert_eq!(
        f.runtime.get_global("preserved"),
        f.runtime.get_global("info")
    );
    run(
        &mut f.runtime,
        "Db::execute($s,[2]);$cleared=Db::errorInfo($s);$connectionCleared=Db::errorInfo($db);",
    );
    assert_eq!(f.runtime.get_global("cleared"), Some(&Value::Null));
    assert_eq!(
        f.runtime.get_global("connectionCleared"),
        Some(&Value::Null)
    );
    fails(
        &mut f.runtime,
        "Db::exec($db,'INSERT INTO error_rows VALUES (2)');",
        "DbError",
    );
    run(
        &mut f.runtime,
        "$raw=Db::errorInfo($db);$statement=Db::errorInfo($s);Db::closeStatement($s);Db::close($db);$closed=Db::errorInfo($db);",
    );
    assert_eq!(
        field(&f.runtime, "raw", "operation"),
        &Value::String("exec".into())
    );
    assert_eq!(f.runtime.get_global("statement"), Some(&Value::Null));
    assert_eq!(f.runtime.get_global("raw"), f.runtime.get_global("closed"));
    let count: Option<u64> = f
        .observer
        .query_first("SELECT COUNT(*) FROM error_rows")
        .unwrap();
    assert_eq!(count, Some(2));
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn invalid_index_normal_exhaustion_and_cleanup_preserve_history_without_stale_codes() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT 1 AS n');Db::execute($s);",
    );
    fails(&mut f.runtime, "Db::beginTransaction($db);", "DbStateError");
    run(&mut f.runtime, "$before=Db::errorInfo($db);");
    fails(&mut f.runtime, "Db::fetchColumn($s,-1);", "IndexError");
    run(
        &mut f.runtime,
        "$after=Db::errorInfo($db);$n=Db::fetchColumn($s);$clear=Db::errorInfo($db);",
    );
    assert_eq!(
        f.runtime.get_global("before"),
        f.runtime.get_global("after")
    );
    assert_eq!(f.runtime.get_global("clear"), Some(&Value::Null));
    fails(&mut f.runtime, "Db::commit($db);", "DbStateError");
    run(
        &mut f.runtime,
        "$idle=Db::errorInfo($db);$end=Db::fetch($s);$empty=Db::fetchAll($s);",
    );
    fails(
        &mut f.runtime,
        "Db::fetchColumn($s);",
        "DbEndOfResultsError",
    );
    run(
        &mut f.runtime,
        "Db::closeCursor($s);$eof=Db::errorInfo($db);",
    );
    assert_eq!(f.runtime.get_global("idle"), f.runtime.get_global("eof"));
    assert_eq!(field(&f.runtime, "eof", "sqlState"), &Value::Null);
    // A prior server code must not be reused for a later local failure.
    fails(
        &mut f.runtime,
        "Db::exec($db,'DELETE FROM table_does_not_exist');",
        "DbError",
    );
    run(&mut f.runtime, "$server=Db::errorInfo($db);");
    assert_eq!(
        field(&f.runtime, "server", "driverCode"),
        &Value::String("1146".into())
    );
    fails(
        &mut f.runtime,
        "Db::exec($db,'SELECT 1');",
        "DbUnsupportedError",
    );
    run(
        &mut f.runtime,
        "$unsupported=Db::errorInfo($db);Db::close($db);$closed=Db::errorInfo($db);",
    );
    assert_eq!(
        field(&f.runtime, "unsupported", "category"),
        &Value::String("unsupported".into())
    );
    assert_eq!(field(&f.runtime, "unsupported", "driverCode"), &Value::Null);
    assert_eq!(
        f.runtime.get_global("unsupported"),
        f.runtime.get_global("closed")
    );
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn timeout_diagnostics_survive_connection_invalidation_without_leaking_credentials() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT SLEEP(?)');Db::setAttribute($db,'operationTimeoutMs',30);",
    );
    fails(&mut f.runtime, "Db::execute($s,[0.2]);", "DbTimeoutError");
    run(
        &mut f.runtime,
        "$info=Db::errorInfo($s);$connection=Db::errorInfo($db);Db::close($db);",
    );
    assert_eq!(
        field(&f.runtime, "info", "category"),
        &Value::String("timeout".into())
    );
    assert_eq!(
        f.runtime.get_global("info"),
        f.runtime.get_global("connection")
    );
    let Value::String(message) = field(&f.runtime, "info", "message") else {
        panic!()
    };
    assert!(!message.contains("fixture-password-not-for-production"));
    assert!(!message.contains("SELECT"));
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn simultaneous_connections_have_independent_cursors_transactions_and_failure_records() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE independent_rows (n INT PRIMARY KEY) ENGINE=InnoDB');$other=Db::connect($dsn,$user,$password,$options);Db::beginTransaction($db);Db::exec($db,'INSERT INTO independent_rows VALUES (1)');$a=Db::prepare($db,'SELECT n FROM independent_rows');Db::execute($a);$b=Db::prepare($other,'SELECT COUNT(*) AS n FROM independent_rows');Db::execute($b);$uncommitted=Db::fetchColumn($a);$outside=Db::fetchColumn($b);",
    );
    assert_eq!(f.runtime.get_global("uncommitted"), Some(&Value::Int(1)));
    assert_eq!(f.runtime.get_global("outside"), Some(&Value::Int(0)));
    fails(
        &mut f.runtime,
        "Db::exec($db,'INSERT INTO independent_rows VALUES (1)');",
        "DbError",
    );
    run(
        &mut f.runtime,
        "$error=Db::errorInfo($db);$unaffected=Db::errorInfo($other);Db::commit($db);Db::execute($b);$committed=Db::fetchColumn($b);Db::close($db);Db::execute($b);$retained=Db::fetchColumn($b);Db::close($other);",
    );
    assert_eq!(f.runtime.get_global("unaffected"), Some(&Value::Null));
    assert_eq!(f.runtime.get_global("committed"), Some(&Value::Int(1)));
    assert_eq!(f.runtime.get_global("retained"), Some(&Value::Int(1)));
}
