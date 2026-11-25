//! Native tests run through tests/support/postgres_fixture.py, never public hosts.
use scr_ext_postgres::PostgresOptions;
use scr_runtime::{MapKey, Runtime, RuntimeOptions, Value};
use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
fn runtime(policy: PostgresOptions, options: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_db::register(
        &mut runtime,
        vec![scr_ext_postgres::driver_with_options(policy).unwrap()],
    )
    .unwrap();
    for (global, name) in [
        ("dsn", "DSN"),
        ("user", "USER"),
        ("password", "PASSWORD"),
        ("ca", "CA"),
    ] {
        runtime
            .set_global(
                global,
                Value::String(
                    env::var(format!("SCR_PG_FIXTURE_{name}")).expect("run postgres_fixture.py"),
                ),
            )
            .unwrap();
    }
    run(
        &mut runtime,
        "$options=['tlsCaFile'=>$ca];$db=Db::connect($dsn,$user,$password,$options);",
    );
    runtime
}
fn open() -> Runtime {
    runtime(Default::default(), Default::default())
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
fn integer_text(r: &Runtime, name: &str) -> String {
    let Some(Value::Int(n)) = r.get_global(name) else {
        panic!("expected int")
    };
    n.to_string()
}
fn value(runtime: &Runtime, name: &str, expected: Value) {
    assert_eq!(runtime.get_global(name), Some(&expected));
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn binding_fetching_and_cursor_contract() {
    let mut r = open();
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT :name::text AS name, :n::bigint AS n, :flag::boolean AS flag, :binary::bytea AS binary, :nil::text AS nil');",
    );
    // Bytes must remain distinct from text, including NUL and invalid UTF-8.
    r.set_global("binary", Value::Bytes(vec![0, 255, 1].into()))
        .unwrap();
    r.set_global("attack", Value::String("é' OR true --".into()))
        .unwrap();
    run(
        &mut r,
        "Db::execute($s,['name'=>$attack,'n'=>42,'flag'=>false,'binary'=>$binary,'nil'=>null]);",
    );
    fails(&mut r, "Db::prepare($db,'SELECT 1');", "DbStateError");
    fails(&mut r, "Db::fetchColumn($s,-1);", "IndexError");
    run(
        &mut r,
        "$row=Db::fetch($s);$end=Db::fetch($s);$all=Db::fetchAll($s);",
    );
    value(
        &r,
        "row",
        Value::Map(vec![
            (
                MapKey::String("name".into()),
                Value::String("é' OR true --".into()),
            ),
            (MapKey::String("n".into()), Value::Int(42)),
            (MapKey::String("flag".into()), Value::Bool(false)),
            (
                MapKey::String("binary".into()),
                Value::Bytes(vec![0, 255, 1].into()),
            ),
            (MapKey::String("nil".into()), Value::Null),
        ]),
    );
    value(&r, "end", Value::Null);
    value(&r, "all", Value::List(vec![]));
    fails(&mut r, "Db::fetchColumn($s);", "DbEndOfResultsError");
    run(
        &mut r,
        "$p=Db::prepare($db,'SELECT ?::int AS n UNION ALL SELECT 7');Db::execute($p,[3]);",
    );
    fails(&mut r, "Db::execute($p,[4]);", "DbStateError");
    run(
        &mut r,
        "$first=Db::fetchColumn($p);$rest=Db::fetchAll($p);Db::execute($p,[4]);Db::closeCursor($p);",
    );
    value(&r, "first", Value::Int(3));
    fails(&mut r, "Db::fetch($p);", "DbStateError");
    fails(&mut r, "Db::execute($p,['bad']);", "DbError");
    fails(&mut r, "Db::fetchAll($p);", "DbStateError");
    run(
        &mut r,
        "Db::execute($p,[9]);Db::closeStatement($p);Db::closeStatement($p);Db::close($db);Db::close($db);",
    );
    fails(&mut r, "Db::fetch($s);", "DbStateError");
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn dialect_literals_operators_and_explicit_types() {
    let mut r = open();
    run(
        &mut r,
        r#"$s=Db::prepare($db,"SELECT $$?:ignored$$ AS dollar, $tag$?:ignored$tag$ AS tagged, E'\\\\?' AS escaped, :n::bigint AS number /* :ignore /* ? */ */ --:ignored?\n");Db::execute($s,['n'=>4]);$row=Db::fetch($s);"#,
    );
    run(
        &mut r,
        r#"$s=Db::prepare($db,"SELECT '{\"a\":1}'::jsonb ?? :key::text AS has_key, '{\"a\":1}'::jsonb ?| ARRAY['a'] AS any_key, '{\"a\":1}'::jsonb ?& ARRAY['a'] AS all_keys, '{\"a\":1}'::jsonb @? '$.a' AS path");Db::execute($s,['key'=>'a']);$operators=Db::fetch($s);"#,
    );
    value(
        &r,
        "operators",
        Value::Map(
            ["has_key", "any_key", "all_keys", "path"]
                .into_iter()
                .map(|s| (MapKey::String(s.into()), Value::Bool(true)))
                .collect(),
        ),
    );
    run(
        &mut r,
        r#"$s=Db::prepare($db,"SELECT 12345678901234567890.123400::numeric AS n, '2024-02-29'::date AS d, '2024-02-29 12:34:56.123456'::timestamp AS t, '2024-02-29 12:34:56+02'::timestamptz AS z, '{\"n\":1}'::json AS j");Db::execute($s);$types=Db::fetch($s);"#,
    );
    value(
        &r,
        "types",
        Value::Map(vec![
            (
                MapKey::String("n".into()),
                Value::String("12345678901234567890.123400".into()),
            ),
            (
                MapKey::String("d".into()),
                Value::String("2024-02-29".into()),
            ),
            (
                MapKey::String("t".into()),
                Value::String("2024-02-29 12:34:56.123456".into()),
            ),
            (
                MapKey::String("z".into()),
                Value::String("2024-02-29 10:34:56+00".into()),
            ),
            (
                MapKey::String("j".into()),
                Value::String("{\"n\":1}".into()),
            ),
        ]),
    );
    for sql in [
        "SELECT 1; DELETE FROM sequence_items",
        "SET TimeZone='UTC'",
        "BEGIN",
        "COMMIT",
        "COPY sequence_items TO STDOUT",
        "CALL test()",
        "WITH x AS (SELECT 1) SELECT * FROM x",
    ] {
        r.set_global("sql", Value::String(sql.into())).unwrap();
        fails(&mut r, "Db::prepare($db,$sql);", "DbUnsupportedError");
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn returning_sequences_triggers_two_sessions_and_rollback() {
    let mut r = open();
    run(
        &mut r,
        "$other=Db::connect($dsn,$user,$password,$options);$a=Db::prepare($db,'INSERT INTO sequence_items(label) VALUES (?) RETURNING id');$b=Db::prepare($other,'INSERT INTO sequence_items(label) VALUES (?) RETURNING id');Db::execute($a,['session-a']);",
    );
    fails(&mut r, "Db::rowCount($a);", "DbStateError");
    run(
        &mut r,
        "$aId=Db::fetchColumn($a);$aCount=Db::rowCount($a);Db::execute($b,['session-b']);$bId=Db::fetchColumn($b);$seqA=Db::lastInsertId($db,'sequence_items_id_seq');$seqB=Db::lastInsertId($other,'sequence_items_id_seq');$audit=Db::lastInsertId($db,'trigger_sequence');",
    );
    let a = integer_text(&r, "aId");
    let b = integer_text(&r, "bId");
    assert_ne!(a, b);
    value(&r, "seqA", Value::String(a));
    value(&r, "seqB", Value::String(b));
    value(&r, "aCount", Value::Int(1));
    assert_ne!(r.get_global("seqA"), r.get_global("audit"));
    fails(&mut r, "Db::lastInsertId($db);", "DbUnsupportedError");
    run(
        &mut r,
        "Db::beginTransaction($db);Db::execute($a,['rolled-back']);$rolled=Db::fetchColumn($a);Db::rollBack($db);$allocated=Db::lastInsertId($db,'sequence_items_id_seq');$check=Db::prepare($db,'SELECT COUNT(*) FROM sequence_items WHERE label=?');Db::execute($check,['rolled-back']);$count=Db::fetchColumn($check);",
    );
    value(&r, "count", Value::Int(0));
    assert_eq!(
        r.get_global("allocated"),
        Some(&Value::String(integer_text(&r, "rolled")))
    );
    run(
        &mut r,
        "Db::beginTransaction($db);Db::execute($a,['discarded-returning']);Db::closeCursor($a);",
    );
    fails(&mut r, "Db::rowCount($a);", "DbStateError");
    run(
        &mut r,
        "Db::rollBack($db);Db::close($other);Db::close($db);",
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn aborted_transactions_error_info_and_ddl_are_postgres_specific() {
    let mut r = open();
    run(
        &mut r,
        "Db::exec($db,'CREATE TABLE pg_abort (id bigint PRIMARY KEY)');$s=Db::prepare($db,'INSERT INTO pg_abort VALUES (?)');Db::beginTransaction($db);Db::execute($s,[1]);",
    );
    fails(&mut r, "Db::execute($s,[1]);", "DbError");
    run(
        &mut r,
        "$info=Db::errorInfo($s);$state=$info['sqlState'];$code=$info['driverCode'];$active=Db::inTransaction($db);",
    );
    value(&r, "state", Value::String("23505".into()));
    value(&r, "code", Value::Null);
    value(&r, "active", Value::Bool(true));
    fails(&mut r, "Db::commit($db);", "DbStateError");
    fails(&mut r, "Db::execute($s,[2]);", "DbError");
    run(
        &mut r,
        "$info=Db::errorInfo($s);$state=$info['sqlState'];Db::rollBack($db);$q=Db::prepare($db,'SELECT COUNT(*) FROM pg_abort');Db::execute($q);$n=Db::fetchColumn($q);",
    );
    value(&r, "state", Value::String("25P02".into()));
    value(&r, "n", Value::Int(0));
    run(
        &mut r,
        "Db::beginTransaction($db);Db::exec($db,'CREATE TABLE pg_ddl_rollback (id bigint)');$active=Db::inTransaction($db);Db::rollBack($db);",
    );
    value(&r, "active", Value::Bool(true));
    fails(
        &mut r,
        "Db::prepare($db,'SELECT * FROM pg_ddl_rollback');",
        "DbError",
    );
    fails(&mut r, "Db::commit($db);", "DbStateError");
    run(&mut r, "Db::close($db);$saved=Db::errorInfo($db);");
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn decoding_failure_null_false_and_index_preflight() {
    let mut r = open();
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT NULL::boolean UNION ALL SELECT false');Db::execute($s);$nil=Db::fetchColumn($s);$no=Db::fetchColumn($s);",
    );
    value(&r, "nil", Value::Null);
    value(&r, "no", Value::Bool(false));
    fails(&mut r, "Db::fetchColumn($s);", "DbEndOfResultsError");
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT 1 AS n, 2 AS n');Db::execute($s);",
    );
    fails(&mut r, "Db::fetch($s);", "DbTypeError");
    run(&mut r, "Db::execute($s);$v=Db::fetchColumn($s,1);");
    value(&r, "v", Value::Int(2));
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT ARRAY[1,2]');Db::execute($s);",
    );
    fails(&mut r, "Db::fetchColumn($s);", "DbTypeError");
    run(
        &mut r,
        "$s=Db::prepare($db,\"SELECT 'NaN'::float8\");Db::execute($s);",
    );
    fails(&mut r, "Db::fetchColumn($s);", "DbTypeError");
    run(&mut r, "$s=Db::prepare($db,'SELECT ?::text');");
    r.set_global("binary", Value::Bytes(vec![255].into()))
        .unwrap();
    fails(&mut r, "Db::execute($s,[$binary]);", "DbTypeError");
    run(
        &mut r,
        "Db::execute($s,['valid']);$valid=Db::fetchColumn($s);Db::close($db);",
    );
    value(&r, "valid", Value::String("valid".into()));
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn exact_limits_and_limit_plus_one() {
    let mut r = runtime(
        PostgresOptions {
            max_rows: 2,
            ..Default::default()
        },
        Default::default(),
    );
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT generate_series(1,2) AS n');Db::execute($s);$rows=Db::fetchAll($s);",
    );
    let Value::List(rows) = r.get_global("rows").unwrap() else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    run(&mut r, "$s=Db::prepare($db,'SELECT generate_series(1,3)');");
    fails(&mut r, "Db::execute($s);", "DbLimitError");
    fails(&mut r, "Db::prepare($db,'SELECT 1');", "DbStateError");
    let mut r = runtime(
        PostgresOptions {
            max_cell_bytes: 4,
            ..Default::default()
        },
        Default::default(),
    );
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT ?::text');Db::execute($s,['1234']);$v=Db::fetchColumn($s);",
    );
    value(&r, "v", Value::String("1234".into()));
    fails(&mut r, "Db::execute($s,['12345']);", "DbLimitError");
    run(
        &mut r,
        "Db::execute($s,['ok']);$v=Db::fetchColumn($s);$s=Db::prepare($db,\"SELECT '12345'::text\");Db::execute($s);",
    );
    fails(&mut r, "Db::fetchColumn($s);", "DbLimitError");
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT 1');Db::execute($s);Db::fetchAll($s);Db::close($db);",
    );
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn deadlines_and_cancellation_break_sessions_without_replay() {
    let mut r = open();
    run(
        &mut r,
        "$s=Db::prepare($db,'SELECT 1 FROM pg_sleep(2)');Db::setAttribute($db,'operationTimeoutMs',40);",
    );
    let now = Instant::now();
    fails(&mut r, "Db::execute($s);", "DbTimeoutError");
    assert!(now.elapsed() < Duration::from_secs(1));
    fails(&mut r, "Db::execute($s);", "DbStateError");
    let flag = Arc::new(AtomicBool::new(false));
    let mut r = runtime(
        Default::default(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
    );
    run(&mut r, "$s=Db::prepare($db,'SELECT 1 FROM pg_sleep(2)');");
    let cancellation = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        flag.store(true, Ordering::Release);
    });
    let now = Instant::now();
    fails(&mut r, "Db::execute($s);", "Interrupted");
    assert!(now.elapsed() < Duration::from_secs(1));
    cancellation.join().unwrap();
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn lost_commit_acknowledgement_is_never_replayed() {
    let mut r = open();
    let dsn = env::var("SCR_PG_FIXTURE_DSN").unwrap();
    let backend = env::var("SCR_PG_FIXTURE_PORT").unwrap();
    let proxy = env::var("SCR_PG_FIXTURE_COMMIT_PROXY_PORT").unwrap();
    r.set_global(
        "proxyDsn",
        Value::String(dsn.replace(&format!("port={backend};"), &format!("port={proxy};"))),
    )
    .unwrap();
    run(
        &mut r,
        "Db::exec($db,'CREATE TABLE pg_uncertain (id bigint PRIMARY KEY)');$proxied=Db::connect($proxyDsn,$user,$password,$options);Db::beginTransaction($proxied);$write=Db::prepare($proxied,'INSERT INTO pg_uncertain VALUES (?)');Db::execute($write,[1]);$destination='preserved';",
    );
    fails(&mut r, "$destination=Db::commit($proxied);", "DbError");
    value(&r, "destination", Value::String("preserved".into()));
    run(
        &mut r,
        "$info=Db::errorInfo($proxied);$category=$info['category'];",
    );
    value(&r, "category", Value::String("connection".into()));
    fails(&mut r, "Db::execute($write,[2]);", "DbStateError");
    run(
        &mut r,
        "$check=Db::prepare($db,'SELECT COUNT(*) FROM pg_uncertain');Db::execute($check);$count=Db::fetchColumn($check);Db::close($db);",
    );
    value(&r, "count", Value::Int(1));
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn tls_verification_authentication_and_open_transaction_cleanup() {
    let mut r = open();
    fails(
        &mut r,
        "Db::connect($dsn,$user,'wrong-password',$options);",
        "DbError",
    );
    fails(&mut r, "Db::connect($dsn,$user,$password);", "DbError");
    let dsn = env::var("SCR_PG_FIXTURE_DSN")
        .unwrap()
        .replace("host=localhost;", "host=127.0.0.1;");
    r.set_global("mismatch", Value::String(dsn)).unwrap();
    fails(
        &mut r,
        "Db::connect($mismatch,$user,$password,$options);",
        "DbError",
    );
    run(
        &mut r,
        "Db::exec($db,'CREATE TABLE pg_cleanup (id bigint PRIMARY KEY)');$other=Db::connect($dsn,$user,$password,$options);Db::beginTransaction($other);Db::exec($other,'INSERT INTO pg_cleanup VALUES (1)');Db::close($other);$s=Db::prepare($db,'SELECT COUNT(*) FROM pg_cleanup');Db::execute($s);$n=Db::fetchColumn($s);Db::close($db);",
    );
    value(&r, "n", Value::Int(0));
}

#[test]
#[ignore = "requires disposable PostgreSQL SCRAM/TLS fixture"]
fn changing_sql_lexing_settings_quarantines_the_session() {
    let mut r = open();
    run(
        &mut r,
        "$s=Db::prepare($db,\"SELECT set_config('standard_conforming_strings', 'off', false)\");",
    );
    fails(&mut r, "Db::execute($s);", "DbUnsupportedError");
    fails(
        &mut r,
        "Db::exec($db,'DELETE FROM sequence_items');",
        "DbStateError",
    );
}
