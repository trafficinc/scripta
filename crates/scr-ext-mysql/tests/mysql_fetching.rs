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
fn mixed_fetches_preserve_order_consume_remaining_rows_and_distinguish_exhaustion() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT 1 AS z,11 AS a UNION ALL SELECT 2,22 UNION ALL SELECT 3,33');",
    );
    fails(&mut f.runtime, "Db::fetch($s);", "DbStateError");
    run(&mut f.runtime, "Db::execute($s);");
    fails(&mut f.runtime, "Db::fetchColumn($s,-1);", "IndexError");
    fails(&mut f.runtime, "Db::fetchColumn($s,2);", "IndexError");
    fails(&mut f.runtime, "Db::fetchColumn($s,0.0);", "TypeError");
    run(
        &mut f.runtime,
        "$first=Db::fetch($s);$rest=Db::fetchAll($s);$end=Db::fetch($s);$again=Db::fetchAll($s);",
    );
    assert_eq!(field(&f.runtime, "first", "z"), &Value::Int(1));
    let Some(Value::Map(entries)) = f.runtime.get_global("first") else {
        panic!()
    };
    assert_eq!(
        entries.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        vec![MapKey::String("z".into()), MapKey::String("a".into())]
    );
    assert_eq!(
        f.runtime.get_global("rest"),
        Some(&Value::list(vec![
            Value::Map(vec![
                (MapKey::String("z".into()), Value::Int(2)),
                (MapKey::String("a".into()), Value::Int(22))
            ]),
            Value::Map(vec![
                (MapKey::String("z".into()), Value::Int(3)),
                (MapKey::String("a".into()), Value::Int(33))
            ])
        ]))
    );
    assert_eq!(f.runtime.get_global("end"), Some(&Value::Null));
    assert_eq!(f.runtime.get_global("again"), Some(&Value::list(vec![])));
    run(&mut f.runtime, "$destination='preserved';");
    fails(
        &mut f.runtime,
        "$destination=Db::fetchColumn($s);",
        "DbEndOfResultsError",
    );
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    run(
        &mut f.runtime,
        "$other=Db::prepare($db,'SELECT 99 AS n');Db::execute($other);$oldEnd=Db::fetch($s);$new=Db::fetchColumn($other);",
    );
    assert_eq!(f.runtime.get_global("oldEnd"), Some(&Value::Null));
    assert_eq!(f.runtime.get_global("new"), Some(&Value::Int(99)));
    run(
        &mut f.runtime,
        "Db::execute($s);$second=Db::fetchColumn($s,1);$tail=Db::fetchAll($s);",
    );
    assert_eq!(f.runtime.get_global("second"), Some(&Value::Int(11)));
    run(&mut f.runtime, "Db::closeCursor($s);");
    fails(&mut f.runtime, "Db::fetchAll($s);", "DbStateError");
    run(&mut f.runtime, "Db::execute($s);Db::closeStatement($s);");
    fails(&mut f.runtime, "Db::fetch($s);", "DbStateError");
    run(&mut f.runtime, "Db::close($db);");
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn null_zero_empty_results_and_no_result_commands_have_distinct_states() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT NULL AS v UNION ALL SELECT 0');Db::execute($s);$null=Db::fetchColumn($s);$zero=Db::fetchColumn($s);",
    );
    assert_eq!(f.runtime.get_global("null"), Some(&Value::Null));
    assert_eq!(f.runtime.get_global("zero"), Some(&Value::Int(0)));
    fails(
        &mut f.runtime,
        "Db::fetchColumn($s);",
        "DbEndOfResultsError",
    );
    fails(
        &mut f.runtime,
        "Db::fetchColumn($s);",
        "DbEndOfResultsError",
    );
    run(
        &mut f.runtime,
        "$empty=Db::prepare($db,'SELECT 1 AS v WHERE 1=0');Db::execute($empty);$none=Db::fetch($empty);$all=Db::fetchAll($empty);",
    );
    assert_eq!(f.runtime.get_global("none"), Some(&Value::Null));
    assert_eq!(f.runtime.get_global("all"), Some(&Value::list(vec![])));
    fails(
        &mut f.runtime,
        "Db::fetchColumn($empty);",
        "DbEndOfResultsError",
    );
    f.observer
        .query_drop("CREATE TABLE fetching_commands (id INT PRIMARY KEY)")
        .unwrap();
    run(
        &mut f.runtime,
        "$insert=Db::prepare($db,'INSERT INTO fetching_commands VALUES (?)');Db::execute($insert,[1]);",
    );
    fails(&mut f.runtime, "Db::fetch($insert);", "DbStateError");
    fails(&mut f.runtime, "Db::execute($insert,[1]);", "DbError");
    fails(&mut f.runtime, "Db::fetchAll($insert);", "DbStateError");
    run(
        &mut f.runtime,
        "$count=Db::prepare($db,'SELECT COUNT(*) AS n FROM fetching_commands');Db::execute($count);$n=Db::fetchColumn($count);Db::close($db);",
    );
    assert_eq!(f.runtime.get_global("n"), Some(&Value::Int(1)));
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn database_values_decode_strictly_without_decimal_integer_binary_or_temporal_loss() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    f.observer.query_drop("CREATE TABLE fetching_types (i BIGINT,u BIGINT UNSIGNED,d DECIMAL(30,5),f DOUBLE,t LONGTEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,b LONGBLOB,j JSON,dt DATETIME(6),ts TIMESTAMP(3) NULL,tm TIME(3),day_value DATE,year_value YEAR,bool_value BOOLEAN,bits_value BIT(9)) ENGINE=InnoDB").unwrap();
    use mysql::Value as M;
    let decimal = "12345678901234567890.00100";
    let text = "é🦀\0";
    let bytes = vec![0, 255, 128];
    f.observer
        .exec_drop(
            "INSERT INTO fetching_types VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            mysql::Params::Positional(vec![
                M::Int(i64::MIN),
                M::UInt(u64::MAX),
                M::Bytes(decimal.as_bytes().to_vec()),
                M::Double(1.25),
                M::Bytes(text.as_bytes().to_vec()),
                M::Bytes(bytes.clone()),
                M::Bytes(br#"{"n":1}"#.to_vec()),
                M::Bytes(b"2026-11-24 12:34:56.123456".to_vec()),
                M::Bytes(b"2026-11-24 12:34:56.789".to_vec()),
                M::Bytes(b"-49:02:03.123".to_vec()),
                M::Bytes(b"2026-11-24".to_vec()),
                M::Int(2026),
                M::Int(0),
                M::UInt(511),
            ]),
        )
        .unwrap();
    let json: String = f
        .observer
        .query_first("SELECT CAST(j AS CHAR) FROM fetching_types")
        .unwrap()
        .unwrap();
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT *,NULL AS nil FROM fetching_types');Db::execute($s);$row=Db::fetch($s);Db::close($db);",
    );
    for (key, value) in [
        ("i", Value::Int(i64::MIN)),
        ("u", Value::String(u64::MAX.to_string())),
        ("d", Value::String(decimal.into())),
        ("f", Value::Float(1.25)),
        ("t", Value::String(text.into())),
        ("b", Value::Bytes(bytes.into())),
        ("j", Value::String(json)),
        ("dt", Value::String("2026-11-24 12:34:56.123456".into())),
        ("ts", Value::String("2026-11-24 12:34:56.789".into())),
        ("tm", Value::String("-49:02:03.123".into())),
        ("day_value", Value::String("2026-11-24".into())),
        ("year_value", Value::Int(2026)),
        ("bool_value", Value::Int(0)),
        ("bits_value", Value::Bytes(vec![1, 255].into())),
        ("nil", Value::Null),
    ] {
        assert_eq!(field(&f.runtime, "row", key), &value, "{key}");
    }
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn conversion_errors_invalidate_only_the_result_preserve_destinations_and_allow_reexecution() {
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 16);
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,'SELECT 1 AS same,2 AS same UNION ALL SELECT 3,4');Db::execute($s);$destination='preserved';",
    );
    fails(&mut f.runtime, "$destination=Db::fetch($s);", "DbTypeError");
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    fails(&mut f.runtime, "Db::fetchAll($s);", "DbStateError");
    run(
        &mut f.runtime,
        "Db::execute($s);$scalar=Db::fetchColumn($s,1);Db::closeCursor($s);Db::closeStatement($s);",
    );
    assert_eq!(f.runtime.get_global("scalar"), Some(&Value::Int(2)));
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,\"SELECT ST_GeomFromText('POINT(1 2)') AS geo\");Db::execute($s);",
    );
    fails(
        &mut f.runtime,
        "$destination=Db::fetchAll($s);",
        "DbTypeError",
    );
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    run(&mut f.runtime, "Db::execute($s);");
    fails(&mut f.runtime, "Db::fetchColumn($s);", "DbTypeError");
    run(
        &mut f.runtime,
        "Db::closeStatement($s);$good=Db::prepare($db,'SELECT 42 AS n');Db::execute($good);$recovered=Db::fetchColumn($good);Db::close($db);",
    );
    assert_eq!(f.runtime.get_global("recovered"), Some(&Value::Int(42)));
}

#[test]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn decoded_cell_limits_and_full_size_result_boundaries_preserve_prior_outputs() {
    let mut f = Fixture::open(
        MysqlOptions {
            max_cell_bytes: 20,
            ..Default::default()
        },
        RuntimeOptions::default(),
        8,
    );
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,\"SELECT CAST('18446744073709551615' AS UNSIGNED) AS n\");Db::execute($s);$large=Db::fetchColumn($s);Db::close($db);",
    );
    assert_eq!(
        f.runtime.get_global("large"),
        Some(&Value::String(u64::MAX.to_string()))
    );
    drop(f);
    let mut f = Fixture::open(
        MysqlOptions {
            max_cell_bytes: 19,
            ..Default::default()
        },
        RuntimeOptions::default(),
        8,
    );
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,\"SELECT CAST('18446744073709551615' AS UNSIGNED) AS n\");Db::execute($s);$destination='preserved';",
    );
    fails(
        &mut f.runtime,
        "$destination=Db::fetchColumn($s);",
        "DbLimitError",
    );
    assert_eq!(
        f.runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    run(
        &mut f.runtime,
        "$good=Db::prepare($db,'SELECT 7 AS n');Db::execute($good);$n=Db::fetchColumn($good);Db::close($db);",
    );
    assert_eq!(f.runtime.get_global("n"), Some(&Value::Int(7)));
    drop(f);
    let mut f = Fixture::open(MysqlOptions::default(), RuntimeOptions::default(), 8);
    run(
        &mut f.runtime,
        "$s=Db::prepare($db,\"SELECT REPEAT('x',?) AS v\");Db::execute($s,[1048576]);$rows=Db::fetchAll($s);",
    );
    let Some(Value::List(rows)) = f.runtime.get_global("rows") else {
        panic!()
    };
    let Value::Map(entries) = &rows[0] else {
        panic!()
    };
    let Value::String(text) = &entries[0].1 else {
        panic!()
    };
    assert_eq!(text.len(), 1024 * 1024);
    assert!(text.as_bytes().iter().all(|b| *b == b'x'));
    fails(&mut f.runtime, "Db::execute($s,[1048577]);", "DbLimitError");
    let Some(Value::List(rows)) = f.runtime.get_global("rows") else {
        panic!()
    };
    assert_eq!(rows.len(), 1);
}
