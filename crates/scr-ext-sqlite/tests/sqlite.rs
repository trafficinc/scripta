use scr_ext_sqlite::{SqliteOptions, register, register_with_options};
use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn runtime(path: &Path) -> Runtime {
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: path.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    register(&mut rt).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) -> Result<String, RuntimeError> {
    let mut out = Vec::new();
    rt.execute_source(&format!("<?scr {source}"), &mut out)?;
    Ok(String::from_utf8(out).unwrap())
}
fn fails(rt: &mut Runtime, source: &str, kind: &str) {
    let e = run(rt, source).unwrap_err();
    assert!(e.message.starts_with(&format!("{kind}:")), "{source}: {e}");
}
fn memory() -> Runtime {
    let mut rt = runtime(Path::new("."));
    run(&mut rt, "$db = Sqlite::open(':memory:');").unwrap();
    rt
}

#[test]
fn binds_scalars_unicode_null_and_quotes_without_sql_interpolation() {
    let mut rt = memory();
    run(&mut rt, "Sqlite::execute($db, 'CREATE TABLE sample (id INTEGER, name TEXT, enabled INTEGER, weight REAL, extra TEXT)');
        $count = Sqlite::execute($db, 'INSERT INTO sample VALUES (?, ?, ?, ?, ?)', [7, \"世界'); DROP TABLE sample; --\", true, 1.25, null]);
        $rows = Sqlite::query($db, 'SELECT * FROM sample WHERE id = ?1', [7]);
        $id = $rows[0]['id']; $name = $rows[0]['name']; $enabled = $rows[0]['enabled'];
        $weight = $rows[0]['weight']; $extra = $rows[0]['extra'];").unwrap();
    assert_eq!(rt.get_global("count"), Some(&Value::Int(1)));
    assert_eq!(rt.get_global("id"), Some(&Value::Int(7)));
    assert_eq!(
        rt.get_global("name"),
        Some(&Value::String("世界'); DROP TABLE sample; --".into()))
    );
    assert_eq!(rt.get_global("enabled"), Some(&Value::Int(1)));
    assert_eq!(rt.get_global("weight"), Some(&Value::Float(1.25)));
    assert_eq!(rt.get_global("extra"), Some(&Value::Null));
    assert_eq!(
        run(
            &mut rt,
            "echo Sqlite::query($db, 'SELECT CAST(count(*) AS TEXT) AS n FROM sample')[0]['n'];"
        )
        .unwrap(),
        "1"
    );
}

#[test]
fn transactions_counts_constraints_and_failed_call_recovery() {
    let mut rt = memory();
    run(&mut rt, "Sqlite::execute($db, 'CREATE TABLE t (id INTEGER PRIMARY KEY)');
        Sqlite::execute($db, 'BEGIN'); Sqlite::execute($db, 'INSERT INTO t VALUES (1)');
        Sqlite::execute($db, 'ROLLBACK'); $empty = Sqlite::query($db, 'SELECT * FROM t');
        Sqlite::execute($db, 'BEGIN'); Sqlite::execute($db, 'INSERT INTO t VALUES (2)');
        $commit = Sqlite::execute($db, 'COMMIT'); $ddl = Sqlite::execute($db, 'CREATE TABLE x (id)');").unwrap();
    assert_eq!(rt.get_global("empty"), Some(&Value::List(vec![])));
    assert_eq!(rt.get_global("commit"), Some(&Value::Int(0)));
    assert_eq!(rt.get_global("ddl"), Some(&Value::Int(0)));
    fails(
        &mut rt,
        "Sqlite::execute($db, 'INSERT INTO t VALUES (2)');",
        "SqliteConstraintError",
    );
    assert_eq!(run(&mut rt, "try { Sqlite::execute($db, 'INSERT INTO t VALUES (2)'); } catch (SqliteConstraintError $e) { echo 'caught'; }").unwrap(), "caught");
    assert_eq!(
        run(
            &mut rt,
            "echo Sqlite::query($db, 'SELECT CAST(id AS TEXT) AS id FROM t')[0]['id'];"
        )
        .unwrap(),
        "2"
    );
    run(
        &mut rt,
        "Sqlite::execute($db, 'CREATE TABLE child (id REFERENCES t(id))');",
    )
    .unwrap();
    fails(
        &mut rt,
        "Sqlite::execute($db, 'INSERT INTO child VALUES (99)');",
        "SqliteConstraintError",
    );
}

#[test]
fn rejects_wrong_signatures_parameters_and_unsupported_results() {
    let mut rt = memory();
    for source in [
        "Sqlite::open(1);",
        "Sqlite::query($db);",
        "Sqlite::query($db, 1);",
        "Sqlite::close('bad');",
        "Sqlite::query($db, 'SELECT ?', [[$db]]);",
        "Sqlite::query($db, 'SELECT 1', 'bad');",
    ] {
        fails(&mut rt, source, "TypeError");
    }
    for source in [
        "Sqlite::query($db, 'SELECT ?', []);",
        "Sqlite::query($db, 'SELECT 1', [1]);",
        "Sqlite::query($db, 'SELECT :name', [1]);",
        "Sqlite::query($db, 'SELECT 1 AS x, 2 AS x');",
        "Sqlite::query($db, 'SELECT 1; SELECT 2');",
        "Sqlite::query($db, '');",
        "Sqlite::execute($db, 'SELECT 1');",
        "Sqlite::query($db, 'CREATE TABLE no (a)');",
        "Sqlite::execute($db, 'CREATE TABLE no (a); SELECT 1');",
    ] {
        fails(&mut rt, source, "SqliteError");
    }
    assert_eq!(run(&mut rt, "echo Sqlite::query($db, \"SELECT CAST(count(*) AS TEXT) AS n FROM sqlite_schema WHERE name = 'no'\")[0]['n'];").unwrap(), "0");
    for source in [
        "Sqlite::query($db, \"SELECT x'ff' AS data\");",
        "Sqlite::query($db, \"SELECT CAST(x'ff' AS TEXT) AS data\");",
        "Sqlite::query($db, 'SELECT 1e999 AS data');",
    ] {
        fails(&mut rt, source, "SqliteTypeError");
    }
    assert_eq!(
        run(
            &mut rt,
            "echo Sqlite::query($db, \"SELECT hex(x'ff') AS data\")[0]['data'];"
        )
        .unwrap(),
        "FF"
    );
}

#[test]
fn file_persistence_read_only_and_resource_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path());
    run(&mut rt, "$db = Sqlite::open('test.db'); Sqlite::execute($db, 'CREATE TABLE t (x)'); Sqlite::execute($db, 'INSERT INTO t VALUES (5)');
        Sqlite::close($db); Sqlite::close($db); $closed = $db->closed;").unwrap();
    assert_eq!(rt.get_global("closed"), Some(&Value::Bool(true)));
    fails(&mut rt, "Sqlite::query($db, 'SELECT 1');", "SqliteError");
    run(
        &mut rt,
        "$db = Sqlite::open('test.db', true); $readonly = $db->readOnly;",
    )
    .unwrap();
    assert_eq!(rt.get_global("readonly"), Some(&Value::Bool(true)));
    assert_eq!(
        run(
            &mut rt,
            "echo Sqlite::query($db, 'SELECT CAST(x AS TEXT) AS x FROM t')[0]['x'];"
        )
        .unwrap(),
        "5"
    );
    fails(
        &mut rt,
        "Sqlite::execute($db, 'INSERT INTO t VALUES (6)');",
        "PermissionError",
    );
    fails(&mut rt, "Sqlite::open('missing.db', true);", "SqliteError");
    run(&mut rt, "Sqlite::close($db); $db = Sqlite::open('test.db'); Sqlite::execute($db, 'BEGIN EXCLUSIVE'); Sqlite::execute($db, 'INSERT INTO t VALUES (6)');").unwrap();
    drop(rt); // Dropping the connection rolls back the transaction and releases locks.
    let mut next = runtime(dir.path());
    assert_eq!(run(&mut next, "$db = Sqlite::open('test.db'); echo Sqlite::query($db, 'SELECT CAST(count(*) AS TEXT) AS n FROM t')[0]['n'];").unwrap(), "1");
}

#[test]
fn permissions_and_roots_apply_to_file_databases_not_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mut creator = runtime(dir.path());
    run(&mut creator, "$db = Sqlite::open('test.db'); Sqlite::execute($db, 'CREATE TABLE t (x)'); Sqlite::close($db);").unwrap();
    let mut context = ContextOptions {
        working_directory: dir.path().into(),
        ..Default::default()
    };
    context.permissions.allowed.remove("filesystem.write");
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: context.clone(),
        ..Default::default()
    });
    register(&mut rt).unwrap();
    fails(&mut rt, "Sqlite::open('new.db');", "PermissionError");
    assert!(!dir.path().join("new.db").exists());
    run(
        &mut rt,
        "$db = Sqlite::open('test.db', true); Sqlite::query($db, 'SELECT * FROM t');",
    )
    .unwrap();
    context.permissions.allowed.remove("filesystem.read");
    let mut rt = Runtime::with_options(RuntimeOptions {
        context,
        ..Default::default()
    });
    register(&mut rt).unwrap();
    fails(&mut rt, "Sqlite::open('test.db', true);", "PermissionError");
    run(
        &mut rt,
        "$db = Sqlite::open(':memory:'); Sqlite::execute($db, 'CREATE TABLE t (x)');",
    )
    .unwrap();
    let mut rt = Runtime::new();
    register_with_options(
        &mut rt,
        SqliteOptions {
            roots: Some(vec![dir.path().into()]),
            ..Default::default()
        },
    )
    .unwrap();
    rt.set_global(
        "inside",
        Value::String(dir.path().join("inside.db").to_string_lossy().into()),
    )
    .unwrap();
    run(&mut rt, "$db = Sqlite::open($inside); Sqlite::close($db);").unwrap();
    let outside = tempfile::tempdir().unwrap();
    rt.set_global(
        "outside",
        Value::String(outside.path().join("outside.db").to_string_lossy().into()),
    )
    .unwrap();
    fails(&mut rt, "Sqlite::open($outside);", "PermissionError");
    assert!(!outside.path().join("outside.db").exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
        rt.set_global(
            "escape",
            Value::String(dir.path().join("escape/test.db").to_string_lossy().into()),
        )
        .unwrap();
        fails(&mut rt, "Sqlite::open($escape);", "PermissionError");
        std::os::unix::fs::symlink(
            outside.path().join("dangling.db"),
            dir.path().join("dangling.db"),
        )
        .unwrap();
        rt.set_global(
            "escape",
            Value::String(dir.path().join("dangling.db").to_string_lossy().into()),
        )
        .unwrap();
        fails(&mut rt, "Sqlite::open($escape);", "SqliteError");
        assert!(!outside.path().join("dangling.db").exists());
    }
}

#[test]
fn blocks_secondary_files_pragmas_extensions_and_uris() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path());
    run(&mut rt, "$db = Sqlite::open(':memory:');").unwrap();
    for source in [
        "Sqlite::execute($db, \"ATTACH DATABASE 'outside.db' AS other\");",
        "Sqlite::execute($db, \"VACUUM INTO 'outside.db'\");",
        "Sqlite::query($db, 'PRAGMA database_list');",
        "Sqlite::execute($db, 'PRAGMA temp_store_directory = \"/tmp\"');",
        "Sqlite::query($db, \"SELECT load_extension('missing')\");",
        "Sqlite::execute($db, 'CREATE VIRTUAL TABLE v USING fts5(content)');",
    ] {
        fails(&mut rt, source, "PermissionError");
    }
    assert!(!dir.path().join("outside.db").exists());
    for path in ["", "file:test.db?mode=ro", "https://example.com/db"] {
        rt.set_global("path", Value::String(path.into())).unwrap();
        fails(&mut rt, "Sqlite::open($path);", "SqliteError");
    }
    fails(
        &mut rt,
        "Sqlite::open(':memory:', false, -1);",
        "RangeError",
    );
    fails(
        &mut rt,
        "Sqlite::open(':memory:', false, 5001);",
        "RangeError",
    );
    run(&mut rt, "$ro = Sqlite::open(':memory:', true);").unwrap();
    fails(
        &mut rt,
        "Sqlite::execute($ro, 'CREATE TABLE t(x)');",
        "PermissionError",
    );
}

#[test]
fn busy_errors_are_bounded_and_connection_can_recover() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path());
    run(&mut rt, "$a = Sqlite::open('locked.db'); Sqlite::execute($a, 'CREATE TABLE t (x)'); $b = Sqlite::open('locked.db', false, 0); Sqlite::execute($a, 'BEGIN EXCLUSIVE');").unwrap();
    fails(
        &mut rt,
        "Sqlite::query($b, 'SELECT * FROM t');",
        "SqliteBusyError",
    );
    run(
        &mut rt,
        "Sqlite::execute($a, 'ROLLBACK'); Sqlite::query($b, 'SELECT * FROM t');",
    )
    .unwrap();
}

#[test]
fn limits_rows_bytes_steps_connections_and_allow_recovery() {
    let mut rt = Runtime::new();
    register_with_options(
        &mut rt,
        SqliteOptions {
            max_rows: 2,
            max_result_bytes: 256,
            max_steps: 1000,
            max_connections: 1,
            ..Default::default()
        },
    )
    .unwrap();
    run(&mut rt, "$db = Sqlite::open(':memory:');").unwrap();
    fails(&mut rt, "Sqlite::open(':memory:');", "SqliteLimitError");
    fails(
        &mut rt,
        "Sqlite::query($db, 'SELECT CAST(1 AS TEXT) AS n UNION ALL SELECT 2 UNION ALL SELECT 3');",
        "SqliteLimitError",
    );
    fails(
        &mut rt,
        "Sqlite::query($db, \"SELECT printf('%0300d', 1) AS n\");",
        "SqliteLimitError",
    );
    fails(
        &mut rt,
        "Sqlite::query($db, 'WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n');",
        "SqliteLimitError",
    );
    assert_eq!(run(&mut rt, "echo Sqlite::query($db, 'SELECT CAST(1 AS TEXT) AS n')[0]['n']; Sqlite::close($db); $db = Sqlite::open(':memory:');").unwrap(), "1");
}

#[test]
fn runtime_value_budget_and_statement_deadline_are_enforced() {
    let mut rt = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(256),
        ..Default::default()
    });
    register(&mut rt).unwrap();
    run(&mut rt, "$db = Sqlite::open(':memory:');").unwrap();
    fails(
        &mut rt,
        "Sqlite::query($db, \"SELECT printf('%0200d', 1) AS n\");",
        "SqliteLimitError",
    );
    let mut rt = Runtime::new();
    register_with_options(
        &mut rt,
        SqliteOptions {
            statement_timeout: Duration::from_nanos(1),
            ..Default::default()
        },
    )
    .unwrap();
    run(&mut rt, "$db = Sqlite::open(':memory:');").unwrap();
    fails(
        &mut rt,
        "Sqlite::query($db, 'WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n');",
        "SqliteLimitError",
    );
}

#[test]
fn running_sql_observes_cancellation_and_bypasses_script_catches() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut rt = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    register_with_options(
        &mut rt,
        SqliteOptions {
            max_steps: usize::MAX,
            statement_timeout: Duration::from_secs(30),
            ..Default::default()
        },
    )
    .unwrap();
    run(&mut rt, "$db = Sqlite::open(':memory:');").unwrap();
    let cancel = flag.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        cancel.store(true, Ordering::Relaxed);
    });
    let error = run(&mut rt, "try { Sqlite::query($db, 'WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n'); } catch ($e) { echo 'caught'; }").unwrap_err();
    worker.join().unwrap();
    assert!(error.message.contains("interrupted"), "{error}");
    flag.store(false, Ordering::Relaxed);
    assert_eq!(
        run(
            &mut rt,
            "echo Sqlite::query($db, 'SELECT CAST(1 AS TEXT) AS n')[0]['n'];"
        )
        .unwrap(),
        "1"
    );
}

#[test]
fn handles_cannot_cross_registrations_even_for_close() {
    let mut first = memory();
    let handle = first.get_global("db").unwrap().clone();
    let mut second = runtime(Path::new("."));
    second.set_global("foreign", handle).unwrap();
    fails(
        &mut second,
        "Sqlite::query($foreign, 'SELECT 1');",
        "TypeError",
    );
    fails(&mut second, "Sqlite::close($foreign);", "TypeError");
    assert_eq!(
        run(
            &mut first,
            "echo Sqlite::query($db, 'SELECT CAST(1 AS TEXT) AS n')[0]['n'];"
        )
        .unwrap(),
        "1"
    );
    assert_eq!(register(&mut first).unwrap_err().kind, "RegistryError");
}
