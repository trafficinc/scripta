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
fn affected_counts_and_insert_ids_follow_each_accepted_command() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "$ddl=Db::exec($db,'CREATE TABLE metadata_rows (id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,v INT UNIQUE) ENGINE=InnoDB');$initial=Db::lastInsertId($db);$insert=Db::prepare($db,'INSERT INTO metadata_rows(v) VALUES (?)');",
    );
    assert_eq!(f.runtime.get_global("ddl"), Some(&Value::Int(0)));
    assert_eq!(f.runtime.get_global("initial"), Some(&Value::Null));
    fails(&mut f.runtime, "Db::rowCount($insert);", "DbStateError");
    run(
        &mut f.runtime,
        "Db::execute($insert,[1]);$one=Db::rowCount($insert);$id=Db::lastInsertId($db);Db::closeCursor($insert);$still=Db::rowCount($insert);",
    );
    assert_eq!(f.runtime.get_global("one"), Some(&Value::Int(1)));
    assert_eq!(f.runtime.get_global("still"), Some(&Value::Int(1)));
    assert_eq!(f.runtime.get_global("id"), Some(&Value::String("1".into())));
    for source in [
        "Db::exec($db,'SELECT 1');",
        "Db::exec($db,'INSERT INTO metadata_rows(v) VALUES (?)');",
        "Db::exec($db,'SET autocommit=0');",
        "Db::exec($db,'BEGIN');",
        "Db::exec($db,'DELETE FROM metadata_rows; DROP TABLE metadata_rows');",
        "Db::execute($insert,[]);",
    ] {
        assert!(f.runtime.execute_source(source, &mut Vec::new()).is_err());
        run(&mut f.runtime, "$id=Db::lastInsertId($db);");
        assert_eq!(f.runtime.get_global("id"), Some(&Value::String("1".into())));
    }
    fails(
        &mut f.runtime,
        "Db::lastInsertId($db,'sequence');",
        "DbUnsupportedError",
    );
    fails(&mut f.runtime, "Db::execute($insert,[1]);", "DbError");
    run(&mut f.runtime, "$id=Db::lastInsertId($db);");
    assert_eq!(f.runtime.get_global("id"), Some(&Value::Null));
    fails(&mut f.runtime, "Db::rowCount($insert);", "DbStateError");
    run(
        &mut f.runtime,
        "Db::execute($insert,[2]);$rows=Db::exec($db,'INSERT INTO metadata_rows(v) VALUES (3),(4)');$id=Db::lastInsertId($db);$zero=Db::exec($db,'UPDATE metadata_rows SET v=v');$cleared=Db::lastInsertId($db);$updated=Db::exec($db,'UPDATE metadata_rows SET v=10 WHERE v=1');$deleted=Db::exec($db,'DELETE FROM metadata_rows WHERE v=10');$missing=Db::exec($db,'DELETE FROM metadata_rows WHERE v=999');",
    );
    for (name, n) in [
        ("rows", 2),
        ("zero", 0),
        ("updated", 1),
        ("deleted", 1),
        ("missing", 0),
    ] {
        assert_eq!(f.runtime.get_global(name), Some(&Value::Int(n)));
    }
    assert_eq!(f.runtime.get_global("id"), Some(&Value::String("4".into())));
    assert_eq!(f.runtime.get_global("cleared"), Some(&Value::Null));
    run(
        &mut f.runtime,
        "$replaced=Db::exec($db,'REPLACE INTO metadata_rows(id,v) VALUES (3,2)');$upserted=Db::exec($db,'INSERT INTO metadata_rows(id,v) VALUES (3,2) ON DUPLICATE KEY UPDATE v=20');",
    );
    assert_eq!(f.runtime.get_global("replaced"), Some(&Value::Int(2)));
    assert_eq!(f.runtime.get_global("upserted"), Some(&Value::Int(2)));
    fails(
        &mut f.runtime,
        "Db::exec($db,'INSERT INTO metadata_rows(v) VALUES (3)');",
        "DbError",
    );
    run(&mut f.runtime, "$cleared=Db::lastInsertId($db);");
    assert_eq!(f.runtime.get_global("cleared"), Some(&Value::Null));
    run(
        &mut f.runtime,
        "Db::exec($db,'ALTER TABLE metadata_rows AUTO_INCREMENT=9223372036854775808');Db::execute($insert,[99]);$large=Db::lastInsertId($db,null);$select=Db::prepare($db,'SELECT 1 AS n');Db::execute($select);$cleared=Db::lastInsertId($db);",
    );
    assert_eq!(
        f.runtime.get_global("large"),
        Some(&Value::String("9223372036854775808".into()))
    );
    assert_eq!(f.runtime.get_global("cleared"), Some(&Value::Null));
    fails(
        &mut f.runtime,
        "Db::rowCount($select);",
        "DbUnsupportedError",
    );
    run(&mut f.runtime, "Db::closeCursor($select);");
    fails(
        &mut f.runtime,
        "Db::rowCount($select);",
        "DbUnsupportedError",
    );
    run(&mut f.runtime, "Db::closeStatement($insert);");
    fails(&mut f.runtime, "Db::rowCount($insert);", "DbStateError");
    assert_eq!(count(&mut f.observer, "metadata_rows"), 4);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn mysql_found_rows_explicitly_selects_matched_rather_than_changed_updates() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE matched_rows (n INT) ENGINE=InnoDB');Db::exec($db,'INSERT INTO matched_rows VALUES (1),(2)');Db::close($db);",
    );
    let Some(Value::Map(mut options)) = f.runtime.get_global("options").cloned() else {
        panic!()
    };
    options.push((MapKey::String("mysqlFoundRows".into()), Value::Bool(true)));
    f.runtime
        .set_global("options", Value::Map(options))
        .unwrap();
    run(
        &mut f.runtime,
        "$db=Db::connect($dsn,$user,$password,$options);$s=Db::prepare($db,'UPDATE matched_rows SET n=n');Db::execute($s);$prepared=Db::rowCount($s);$direct=Db::exec($db,'UPDATE matched_rows SET n=n');",
    );
    assert_eq!(f.runtime.get_global("prepared"), Some(&Value::Int(2)));
    assert_eq!(f.runtime.get_global("direct"), Some(&Value::Int(2)));
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn commit_rollback_busy_cursors_and_statement_errors_preserve_real_state() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE transaction_rows (id INT AUTO_INCREMENT PRIMARY KEY,v INT UNIQUE) ENGINE=InnoDB');$idle=Db::inTransaction($db);",
    );
    assert_eq!(f.runtime.get_global("idle"), Some(&Value::Bool(false)));
    for op in ["Db::commit($db);", "Db::rollBack($db);"] {
        fails(&mut f.runtime, op, "DbStateError");
    }
    run(
        &mut f.runtime,
        "Db::beginTransaction($db);$active=Db::inTransaction($db);Db::exec($db,'INSERT INTO transaction_rows(v) VALUES (1)');$id=Db::lastInsertId($db);",
    );
    assert_eq!(f.runtime.get_global("active"), Some(&Value::Bool(true)));
    assert_eq!(count(&mut f.observer, "transaction_rows"), 0);
    fails(&mut f.runtime, "Db::beginTransaction($db);", "DbStateError");
    fails(
        &mut f.runtime,
        "Db::exec($db,'INSERT INTO transaction_rows(v) VALUES (1)');",
        "DbError",
    );
    run(
        &mut f.runtime,
        "$active=Db::inTransaction($db);Db::exec($db,'INSERT INTO transaction_rows(v) VALUES (2)');$id=Db::lastInsertId($db);Db::commit($db);$after=Db::lastInsertId($db);$idle=Db::inTransaction($db);",
    );
    assert_eq!(f.runtime.get_global("active"), Some(&Value::Bool(true)));
    assert_eq!(f.runtime.get_global("idle"), Some(&Value::Bool(false)));
    assert_eq!(f.runtime.get_global("id"), f.runtime.get_global("after"));
    assert_eq!(count(&mut f.observer, "transaction_rows"), 2);
    run(
        &mut f.runtime,
        "Db::beginTransaction($db);$s=Db::prepare($db,'INSERT INTO transaction_rows(v) VALUES (?)');Db::execute($s,[3]);Db::rollBack($db);$id=Db::lastInsertId($db);$select=Db::prepare($db,'SELECT 1 AS n UNION ALL SELECT 2');Db::execute($select);$idle=Db::inTransaction($db);",
    );
    assert_eq!(f.runtime.get_global("id"), Some(&Value::Null));
    assert_eq!(count(&mut f.observer, "transaction_rows"), 2);
    for op in [
        "Db::beginTransaction($db);",
        "Db::commit($db);",
        "Db::rollBack($db);",
        "Db::exec($db,'DELETE FROM transaction_rows');",
    ] {
        fails(&mut f.runtime, op, "DbStateError");
    }
    run(
        &mut f.runtime,
        "$rows=Db::fetchAll($select);Db::beginTransaction($db);Db::execute($s,[4]);Db::rollBack($db);",
    );
    assert_eq!(count(&mut f.observer, "transaction_rows"), 2);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn successful_and_failed_ddl_refresh_implicit_commit_state() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE implicit_rows (n INT) ENGINE=InnoDB');Db::beginTransaction($db);Db::exec($db,'INSERT INTO implicit_rows VALUES (1)');$ddl=Db::exec($db,'CREATE TABLE implicit_other (n INT) ENGINE=InnoDB');$active=Db::inTransaction($db);",
    );
    assert_eq!(f.runtime.get_global("active"), Some(&Value::Bool(false)));
    assert_eq!(count(&mut f.observer, "implicit_rows"), 1);
    fails(&mut f.runtime, "Db::rollBack($db);", "DbStateError");
    run(
        &mut f.runtime,
        "Db::beginTransaction($db);Db::exec($db,'INSERT INTO implicit_rows VALUES (2)');",
    );
    fails(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE implicit_other (n INT)');",
        "DbError",
    );
    run(&mut f.runtime, "$active=Db::inTransaction($db);");
    assert_eq!(f.runtime.get_global("active"), Some(&Value::Bool(false)));
    assert_eq!(count(&mut f.observer, "implicit_rows"), 2);
    fails(&mut f.runtime, "Db::commit($db);", "DbStateError");
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE UNIQUE INDEX implicit_n ON implicit_rows(n)');Db::exec($db,'DROP INDEX implicit_n ON implicit_rows');Db::exec($db,'DROP TABLE implicit_other');",
    );
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn explicit_close_and_runtime_drop_rollback_active_work() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE cleanup_transactions (n INT) ENGINE=InnoDB');Db::beginTransaction($db);Db::exec($db,'INSERT INTO cleanup_transactions VALUES (1)');Db::close($db);",
    );
    assert_eq!(count(&mut f.observer, "cleanup_transactions"), 0);
    run(
        &mut f.runtime,
        "$db=Db::connect($dsn,$user,$password,$options);Db::beginTransaction($db);$s=Db::prepare($db,'INSERT INTO cleanup_transactions VALUES (?)');Db::execute($s,[2]);",
    );
    let Fixture {
        runtime,
        mut observer,
    } = f;
    drop(runtime);
    wait_for_cleanup(&mut observer);
    assert_eq!(count(&mut observer, "cleanup_transactions"), 0);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn interrupted_transaction_disconnects_without_replaying_or_committing_writes() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut f = Fixture::open(
        MysqlOptions::default(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
        8,
    );
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE interrupted_transactions (n INT) ENGINE=InnoDB');Db::beginTransaction($db);Db::exec($db,'INSERT INTO interrupted_transactions VALUES (1)');$s=Db::prepare($db,'SELECT SLEEP(?)');$caught=false;",
    );
    let interrupt = flag.clone();
    let timer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        interrupt.store(true, Ordering::Release);
    });
    fails(
        &mut f.runtime,
        "try{Db::execute($s,[0.2]);}catch(Error $e){$caught=true;}",
        "Interrupted",
    );
    timer.join().unwrap();
    flag.store(false, Ordering::Release);
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(f.runtime.get_global("caught"), Some(&Value::Bool(false)));
    wait_for_cleanup(&mut f.observer);
    assert_eq!(count(&mut f.observer, "interrupted_transactions"), 0);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn disconnected_commit_never_reports_success_or_reuses_the_connection() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE disconnected_transactions (n INT) ENGINE=InnoDB');Db::beginTransaction($db);Db::exec($db,'INSERT INTO disconnected_transactions VALUES (1)');$s=Db::prepare($db,'SELECT CONNECTION_ID()');Db::execute($s);$connection=Db::fetchColumn($s);$destination='preserved';",
    );
    let Some(Value::Int(id)) = f.runtime.get_global("connection") else {
        panic!()
    };
    f.observer
        .query_drop(format!("KILL CONNECTION {id}"))
        .unwrap();
    fails(&mut f.runtime, "$destination=Db::commit($db);", "DbError");
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    fails(&mut f.runtime, "Db::commit($db);", "DbStateError");
    wait_for_cleanup(&mut f.observer);
    assert_eq!(count(&mut f.observer, "disconnected_transactions"), 0);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn deadlock_rollback_is_reported_by_server_state_rather_than_local_begin_flags() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    f.observer
        .query_drop("CREATE TABLE deadlock_transactions (id INT PRIMARY KEY,n INT) ENGINE=InnoDB")
        .unwrap();
    f.observer
        .exec_batch(
            "INSERT INTO deadlock_transactions VALUES (?,0)",
            (1..=101).map(|id| (id,)),
        )
        .unwrap();
    run(
        &mut f.runtime,
        "Db::beginTransaction($db);Db::exec($db,'UPDATE deadlock_transactions SET n=1 WHERE id=1');",
    );
    f.observer.query_drop("START TRANSACTION").unwrap();
    f.observer
        .query_drop("UPDATE deadlock_transactions SET n=2 WHERE id>1")
        .unwrap();
    let Fixture {
        mut runtime,
        mut observer,
    } = f;
    let contender = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        observer
            .query_drop("UPDATE deadlock_transactions SET n=2 WHERE id=1")
            .unwrap();
        observer.query_drop("COMMIT").unwrap();
        observer
    });
    // Our transaction has fewer modified rows and is selected as the victim.
    fails(
        &mut runtime,
        "Db::exec($db,'UPDATE deadlock_transactions SET n=1 WHERE id=2');",
        "DbError",
    );
    let mut observer = contender.join().unwrap();
    run(&mut runtime, "$active=Db::inTransaction($db);");
    assert_eq!(runtime.get_global("active"), Some(&Value::Bool(false)));
    fails(&mut runtime, "Db::commit($db);", "DbStateError");
    let n: Option<u64> = observer
        .query_first("SELECT COUNT(*) FROM deadlock_transactions WHERE n=2")
        .unwrap();
    assert_eq!(n, Some(101));
    run(
        &mut runtime,
        "Db::beginTransaction($db);Db::exec($db,'UPDATE deadlock_transactions SET n=3 WHERE id=1');Db::rollBack($db);",
    );
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn lost_successful_commit_acknowledgement_reports_unknown_outcome_without_replay() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE uncertain_transactions (n INT) ENGINE=InnoDB');Db::close($db);",
    );
    let port = env::var("SCR_MYSQL_FIXTURE_COMMIT_PROXY_PORT")
        .expect("fixture COMMIT acknowledgement proxy");
    f.runtime
        .set_global(
            "proxyDsn",
            Value::String(format!(
                "mysql:host=localhost;port={port};dbname=scripta_fixture"
            )),
        )
        .unwrap();
    run(
        &mut f.runtime,
        "$db=Db::connect($proxyDsn,$user,$password,$options);Db::beginTransaction($db);$s=Db::prepare($db,'INSERT INTO uncertain_transactions VALUES (?)');Db::execute($s,[1]);$destination='preserved';",
    );
    assert_eq!(count(&mut f.observer, "uncertain_transactions"), 0);
    let error = f
        .runtime
        .execute_source("$destination=Db::commit($db);", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.starts_with("DbError"), "{error:?}");
    assert!(error.message.contains("outcome is unknown"), "{error:?}");
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    fails(&mut f.runtime, "Db::commit($db);", "DbStateError");
    wait_for_cleanup(&mut f.observer);
    // The transaction committed once even though the caller saw an error.
    assert_eq!(count(&mut f.observer, "uncertain_transactions"), 1);
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn timed_out_raw_write_disconnects_and_rolls_back_the_active_transaction() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "Db::exec($db,'CREATE TABLE timeout_transactions (n INT) ENGINE=InnoDB');Db::beginTransaction($db);Db::exec($db,'INSERT INTO timeout_transactions VALUES (1)');$s=Db::prepare($db,'SELECT 1');Db::setAttribute($db,'operationTimeoutMs',30);",
    );
    let started = Instant::now();
    fails(
        &mut f.runtime,
        "Db::exec($db,'INSERT INTO timeout_transactions SELECT SLEEP(0.2)');",
        "DbTimeoutError",
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    run(&mut f.runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    wait_for_cleanup(&mut f.observer);
    assert_eq!(count(&mut f.observer, "timeout_transactions"), 0);
}
