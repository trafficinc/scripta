use mysql::prelude::Queryable;
use scr_ext_mysql::MysqlOptions;
use scr_runtime::{MapKey, Runtime, RuntimeOptions, Value};
use std::{
    env,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

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
fn count(observer: &mut mysql::Conn, table: &str) -> u64 {
    observer
        .query_first(format!("SELECT COUNT(*) FROM {table}"))
        .unwrap()
        .unwrap()
}

fn wait_for_cleanup(observer: &mut mysql::Conn) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let count: u64 = observer
            .query_first(
                "SELECT COUNT(*) FROM information_schema.processlist WHERE USER='scripta_fixture'",
            )
            .unwrap()
            .unwrap();
        if count == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "abandoned MySQL session was not cleaned up"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn real_binding_statement_reuse_and_cursor_preflight_preserve_database_state() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 256);
    f.observer.query_drop("CREATE TABLE binding_rows (id BIGINT PRIMARY KEY, name LONGTEXT, raw LONGBLOB, flag BOOLEAN, n BIGINT, score DOUBLE, nullable TEXT) ENGINE=InnoDB").unwrap();
    let hostile = "O'Reilly'); DROP TABLE binding_rows; -- 🦀\0";
    let binary = vec![0, 255, 128, b'\'', b';'];
    f.runtime
        .set_global("minimum", Value::Int(i64::MIN))
        .unwrap();
    f.runtime
        .set_global("text", Value::String(hostile.into()))
        .unwrap();
    f.runtime
        .set_global("raw", Value::Bytes(binary.clone().into()))
        .unwrap();
    run(
        &mut f.runtime,
        "$insert=Db::prepare($db,'INSERT INTO binding_rows VALUES (?,?,?,?,?,?,?)');",
    );
    assert_eq!(count(&mut f.observer, "binding_rows"), 0); // prepare does not execute
    run(
        &mut f.runtime,
        "Db::execute($insert,[1,$text,$raw,true,$minimum,1.25,null]);",
    );
    let row: (String, Vec<u8>, i64, i64, f64, Option<String>) = f
        .observer
        .query_first("SELECT name,raw,flag,n,score,nullable FROM binding_rows WHERE id=1")
        .unwrap()
        .unwrap();
    assert_eq!(row, (hostile.into(), binary, 1, i64::MIN, 1.25, None));
    run(
        &mut f.runtime,
        "Db::execute($insert,[2,'fresh',null,false,9,2.5,'set']);",
    );
    fails(&mut f.runtime, "Db::execute($insert);", "TypeError");
    fails(
        &mut f.runtime,
        "Db::execute($insert,[3,[],null,false,9,2.5,'bad']);",
        "TypeError",
    );
    run(&mut f.runtime, "$destination='preserved';");
    fails(
        &mut f.runtime,
        "$destination=Db::execute($insert,[2,'duplicate',null,false,9,2.5,null]);",
        "DbError",
    );
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    assert_eq!(count(&mut f.observer, "binding_rows"), 2);
    run(
        &mut f.runtime,
        "Db::execute($insert,[3,'recovered',null,false,9,2.5,null]);",
    );
    run(
        &mut f.runtime,
        "$update=Db::prepare($db,'UPDATE binding_rows SET name=:Name WHERE id=:ID');Db::execute($update,['ID'=>1,'Name'=>'renamed']);",
    );
    for source in [
        "Db::execute($update,['ID'=>1]);",
        "Db::execute($update,['ID'=>1,'Other'=>'bad']);",
        "Db::execute($update,['ID'=>1,'Name'=>'bad','Extra'=>0]);",
        "Db::execute($update,[1,'bad']);",
    ] {
        fails(&mut f.runtime, source, "TypeError");
    }
    assert_eq!(
        f.observer
            .query_first::<String, _>("SELECT name FROM binding_rows WHERE id=1")
            .unwrap()
            .unwrap(),
        "renamed"
    );
    for sql in [
        "DELETE FROM binding_rows; DROP TABLE binding_rows",
        "DELETE FROM binding_rows /*! ; DROP TABLE binding_rows */",
        "CALL destructive()",
        "SET autocommit=0",
        "SELECT 1 INTO OUTFILE '/tmp/scripta-forbidden'",
    ] {
        f.runtime
            .set_global("sql", Value::String(sql.into()))
            .unwrap();
        fails(
            &mut f.runtime,
            "Db::prepare($db,$sql);",
            "DbUnsupportedError",
        );
    }
    assert_eq!(count(&mut f.observer, "binding_rows"), 3);
    run(
        &mut f.runtime,
        "$select=Db::prepare($db,'SELECT id,name FROM binding_rows WHERE id=:ID');Db::execute($select,['ID'=>1]);",
    );
    for source in [
        "Db::prepare($db,'SELECT 2');",
        "Db::execute($select,['ID'=>2]);",
        "Db::setAttribute($db,'maxRows',2);",
        "Db::closeStatement($update);",
    ] {
        fails(&mut f.runtime, source, "DbStateError");
    }
    run(
        &mut f.runtime,
        "$stillOpen=!$update->closed;Db::closeCursor($select);Db::closeCursor($select);Db::execute($select,['ID'=>2]);Db::closeStatement($select);",
    );
    assert_eq!(f.runtime.get_global("stillOpen"), Some(&Value::Bool(true)));
    fails(
        &mut f.runtime,
        "Db::execute($select,['ID'=>1]);",
        "DbStateError",
    );
    run(
        &mut f.runtime,
        "Db::closeStatement($insert);Db::closeStatement($update);Db::close($db);",
    );
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn real_parameter_cell_and_statement_limits_cover_exact_and_plus_one() {
    let mut f = Fixture::open(
        MysqlOptions {
            max_parameters: 2,
            max_cell_bytes: 4,
            ..Default::default()
        },
        RuntimeOptions::default(),
        1,
    );
    f.observer
        .query_drop("CREATE TABLE bounded_rows (id INT PRIMARY KEY,v LONGBLOB)")
        .unwrap();
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'INSERT INTO bounded_rows VALUES (?,?)');Db::execute($s,[1,'abcd']);",
    );
    fails(
        &mut f.runtime,
        "Db::execute($s,[2,'abcde']);",
        "DbLimitError",
    );
    fails(
        &mut f.runtime,
        "Db::prepare($db,'SELECT ?');",
        "DbLimitError",
    );
    run(&mut f.runtime, "Db::closeStatement($s);");
    fails(
        &mut f.runtime,
        "Db::prepare($db,'SELECT ?,?,?');",
        "DbLimitError",
    );
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT ?,?');Db::execute($s,[1,2]);Db::closeCursor($s);Db::closeStatement($s);Db::close($db);",
    );
    assert_eq!(count(&mut f.observer, "bounded_rows"), 1);
    drop(f);
    let mut f = Fixture::open(
        MysqlOptions {
            max_parameter_bytes: std::mem::size_of::<Value>() * 3 + 4,
            ..Default::default()
        },
        RuntimeOptions::default(),
        2,
    );
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'INSERT INTO bounded_rows VALUES (?,?)');Db::execute($s,[3,'abcd']);",
    );
    fails(
        &mut f.runtime,
        "Db::execute($s,[4,'abcde']);",
        "DbLimitError",
    );
    assert_eq!(count(&mut f.observer, "bounded_rows"), 2);
    run(&mut f.runtime, "Db::close($db);");
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn result_limits_and_deadlines_break_the_session_and_invalidate_handles() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 4);
    run(
        &mut f.runtime,
        "Db::setAttribute($db,'maxRows',1);$s=Db::prepare($db,'SELECT 1 AS n UNION ALL SELECT 2');",
    );
    fails(&mut f.runtime, "Db::execute($s);", "DbLimitError");
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    fails(&mut f.runtime, "Db::execute($s);", "DbStateError");
    drop(f);
    let mut f = Fixture::open(
        MysqlOptions {
            max_workers: 1,
            ..Default::default()
        },
        RuntimeOptions::default(),
        2,
    );
    f.observer
        .query_drop(
            "CREATE TABLE deadline_rows (id INT AUTO_INCREMENT PRIMARY KEY,n INT) ENGINE=InnoDB",
        )
        .unwrap();
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'INSERT INTO deadline_rows(n) SELECT SLEEP(?)');Db::setAttribute($db,'operationTimeoutMs',30);",
    );
    let started = Instant::now();
    fails(&mut f.runtime, "Db::execute($s,[0.2]);", "DbTimeoutError");
    assert!(started.elapsed() < Duration::from_secs(2));
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    wait_for_cleanup(&mut f.observer);
    // A local timeout is not server-side cancellation. The quarantined operation
    // may commit once; the connector must never automatically replay it.
    assert_eq!(count(&mut f.observer, "deadline_rows"), 1);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn sql_cancellation_bypasses_catches_and_releases_dependent_handles() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut f = Fixture::open(
        MysqlOptions::default(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
        2,
    );
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT SLEEP(?)');$caught=false;",
    );
    let interrupt = flag.clone();
    let timer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        interrupt.store(true, Ordering::Release);
    });
    fails(
        &mut f.runtime,
        "try{Db::execute($s,[0.2]);}catch(Error $error){$caught=true;}",
        "Interrupted",
    );
    timer.join().unwrap();
    flag.store(false, Ordering::Release);
    assert_eq!(f.runtime.get_global("caught"), Some(&Value::Bool(false)));
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    wait_for_cleanup(&mut f.observer);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn default_protocol_limits_accept_one_mib_sql_and_cells_and_1024_parameters() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 4);
    f.observer
        .query_drop("CREATE TABLE byte_rows (id INT PRIMARY KEY,v LONGBLOB)")
        .unwrap();
    let mut sql = "INSERT INTO byte_rows VALUES (?,?)".to_owned();
    sql.extend(std::iter::repeat_n(' ', 1024 * 1024 - sql.len()));
    f.runtime
        .set_global("sql", Value::String(sql.clone()))
        .unwrap();
    f.runtime
        .set_global("data", Value::Bytes(vec![0xff; 1024 * 1024].into()))
        .unwrap();
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,$sql);Db::execute($s,[1,$data]);",
    );
    assert_eq!(
        f.observer
            .query_first::<u64, _>("SELECT OCTET_LENGTH(v) FROM byte_rows WHERE id=1")
            .unwrap()
            .unwrap(),
        1024 * 1024
    );
    sql.push(' ');
    f.runtime.set_global("sql", Value::String(sql)).unwrap();
    fails(&mut f.runtime, "Db::prepare($db,$sql);", "DbLimitError");
    f.runtime
        .set_global("data", Value::Bytes(vec![0xff; 1024 * 1024 + 1].into()))
        .unwrap();
    fails(&mut f.runtime, "Db::execute($s,[2,$data]);", "DbLimitError");
    assert_eq!(count(&mut f.observer, "byte_rows"), 1);
    run(&mut f.runtime, "Db::closeStatement($s);");
    let fields = (0..1024).map(|i| format!("? AS c{i}")).collect::<Vec<_>>();
    f.runtime
        .set_global("sql", Value::String(format!("SELECT {}", fields.join(","))))
        .unwrap();
    f.runtime
        .set_global("params", Value::List(vec![Value::Int(1); 1024]))
        .unwrap();
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,$sql);Db::execute($s,$params);Db::closeCursor($s);Db::closeStatement($s);",
    );
    f.runtime
        .set_global(
            "sql",
            Value::String(format!("SELECT {},? AS extra", fields.join(","))),
        )
        .unwrap();
    fails(&mut f.runtime, "Db::prepare($db,$sql);", "DbLimitError");
    run(&mut f.runtime, "Db::close($db);");
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn statement_scanning_uses_actual_server_sql_modes() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 4);
    let mode: String = f
        .observer
        .query_first("SELECT @@SESSION.sql_mode")
        .unwrap()
        .unwrap();
    if mode.split(',').any(|m| m == "NO_BACKSLASH_ESCAPES") {
        f.runtime
            .set_global("sql", Value::String("SELECT 'a\\', :Name AS marker".into()))
            .unwrap();
        run(
            &mut f.runtime,
            "$s=Db::prepare($db,$sql);Db::execute($s,['Name'=>42]);Db::closeStatement($s);",
        );
        f.runtime
            .set_global("sql", Value::String("SELECT 'a\\', ':literal'".into()))
            .unwrap();
        fails(
            &mut f.runtime,
            "Db::prepare($db,$sql);",
            "DbUnsupportedError",
        );
    } else {
        f.runtime
            .set_global(
                "sql",
                Value::String("SELECT 'a\\\'b', :Name AS marker".into()),
            )
            .unwrap();
        run(
            &mut f.runtime,
            "$s=Db::prepare($db,$sql);Db::execute($s,['Name'=>42]);Db::closeStatement($s);",
        );
    }
    if mode.split(',').any(|m| m == "ANSI_QUOTES") {
        f.runtime
            .set_global("sql", Value::String("SELECT :Name AS \"a\\\"".into()))
            .unwrap();
        run(
            &mut f.runtime,
            "$s=Db::prepare($db,$sql);Db::execute($s,['Name'=>42]);Db::closeStatement($s);",
        );
    }
    run(&mut f.runtime, "Db::close($db);");
}
