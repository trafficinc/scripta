use super::*;
use scr_runtime::{MapKey, RuntimeOptions};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Default)]
struct Observed {
    connects: Cell<usize>,
    prepares: Cell<usize>,
    statement_closes: Cell<usize>,
    closes: Cell<usize>,
    drops: Cell<usize>,
    fail_connect: Cell<bool>,
    fail_execute: Cell<bool>,
    fail_close: Cell<bool>,
    fail_statement_close: Cell<bool>,
    reuse_ids: Cell<bool>,
    executes: Cell<usize>,
    fetches: Cell<usize>,
    execs: Cell<usize>,
    transactions: Cell<usize>,
    break_transaction: Cell<bool>,
    break_execute: Cell<bool>,
    broken: Cell<bool>,
    deny_statement_close: Cell<bool>,
    interrupt_connect: Option<Arc<AtomicBool>>,
}
#[derive(Debug)]
struct TestDriver(Rc<Observed>);
impl Driver for TestDriver {
    fn info(&self) -> DriverInfo {
        DriverInfo {
            name: "fixture",
            version: "1.0.0",
            capabilities: &[],
        }
    }
    fn connect(
        &self,
        _: &mut RuntimeContext<'_>,
        _: ConnectRequest<'_>,
    ) -> NativeResult<Box<dyn Connection>> {
        self.0.connects.set(self.0.connects.get() + 1);
        if self.0.fail_connect.get() {
            return Err(NativeError::new("DbError", "fixture connection failed"));
        }
        if let Some(flag) = &self.0.interrupt_connect {
            flag.store(true, Ordering::Relaxed);
        }
        Ok(Box::new(TestConnection(self.0.clone())))
    }
}
#[derive(Debug)]
struct TestConnection(Rc<Observed>);
impl Connection for TestConnection {
    fn exec(&mut self, _: &mut RuntimeContext<'_>, _: &str) -> NativeResult<i64> {
        self.0.execs.set(self.0.execs.get() + 1);
        Ok(2)
    }
    fn row_count(&self, _: u64) -> NativeResult<i64> {
        Ok(7)
    }
    fn last_insert_id(&mut self, _: &mut RuntimeContext<'_>, _: Option<&str>) -> NativeResult {
        Ok(Value::String("123".into()))
    }
    fn transaction(
        &mut self,
        _: &mut RuntimeContext<'_>,
        operation: TransactionOperation,
    ) -> NativeResult<bool> {
        self.0.transactions.set(self.0.transactions.get() + 1);
        if self.0.break_transaction.get() {
            self.0.broken.set(true);
            return Err(NativeError::new(
                "DbError",
                "fixture transaction outcome unknown",
            ));
        }
        Ok(operation == TransactionOperation::Begin)
    }

    fn fetch_column(&mut self, _: &mut RuntimeContext<'_>, _: u64, column: i64) -> NativeResult {
        self.0.fetches.set(self.0.fetches.get() + 1);
        match column {
            0 => Ok(Value::Bool(false)),
            1 => Ok(Value::Null),
            _ => Err(NativeError::new("DbEndOfResultsError", "fixture exhausted")),
        }
    }
    fn is_broken(&self) -> bool {
        self.0.broken.get()
    }
    fn execute(&mut self, _: &mut RuntimeContext<'_>, _: u64, _: &Value) -> NativeResult<()> {
        self.0.executes.set(self.0.executes.get() + 1);
        if self.0.break_execute.get() {
            self.0.broken.set(true);
            return Err(NativeError::new(
                "DbTimeoutError",
                "fixture execution timed out",
            ));
        }
        if self.0.fail_execute.get() {
            return Err(NativeError::new("DbError", "fixture constraint failure"));
        }
        Ok(())
    }
    fn take_error_details(&mut self) -> Option<ErrorDetails> {
        self.0.fail_execute.get().then(|| ErrorDetails {
            category: ErrorCategory::Constraint,
            sql_state: Some("23000".into()),
            driver_code: Some("1062".into()),
        })
    }
    fn validate_close_statement(&self, _: u64) -> NativeResult<()> {
        if self.0.deny_statement_close.get() {
            return Err(NativeError::new("DbStateError", "fixture cursor busy"));
        }
        Ok(())
    }
    fn prepare(&mut self, _: &mut RuntimeContext<'_>, _: &str) -> NativeResult<u64> {
        self.0.prepares.set(self.0.prepares.get() + 1);
        Ok(if self.0.reuse_ids.get() {
            1
        } else {
            self.0.prepares.get() as u64
        })
    }
    fn close_statement(&mut self, _: &mut RuntimeContext<'_>, _: u64) -> NativeResult<()> {
        self.0
            .statement_closes
            .set(self.0.statement_closes.get() + 1);
        if self.0.fail_statement_close.get() {
            return Err(NativeError::new(
                "DbError",
                "fixture statement close failed",
            ));
        }
        Ok(())
    }
    fn close(&mut self, _: &mut RuntimeContext<'_>) -> NativeResult<()> {
        self.0.closes.set(self.0.closes.get() + 1);
        if self.0.fail_close.get() {
            return Err(NativeError::new("DbError", "fixture close failed"));
        }
        Ok(())
    }
}
impl Drop for TestConnection {
    fn drop(&mut self) {
        self.0.drops.set(self.0.drops.get() + 1);
    }
}

fn setup(options: DatabaseOptions) -> (Runtime, Rc<Observed>) {
    let observed = Rc::new(Observed::default());
    let mut runtime = Runtime::new();
    register_with_options(
        &mut runtime,
        vec![Rc::new(TestDriver(observed.clone()))],
        options,
    )
    .unwrap();
    (runtime, observed)
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime.execute_source(source, &mut Vec::new()).unwrap();
}
fn fails(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with(kind), "{error:?}");
    assert!(error.source.is_some());
    assert!(error.span.end > error.span.start);
}
const CONNECT: &str = "$db=Db::connect('fixture:database=test','user','secret');";

#[test]
fn validation_and_missing_drivers_have_no_backend_effects() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    assert_eq!(observed.connects.get(), 0);
    for source in [
        "Db::connect('fixture:database=test', 1, 'secret');",
        "Db::connect('fixture:database=test', 'user', 'secret', [1]);",
        "Db::connect('fixture:database=test', 'user', 'secret', [0 => 1]);",
        "Db::connect('fixture:database=test', 'user');",
    ] {
        fails(&mut runtime, source, "TypeError");
    }
    fails(
        &mut runtime,
        "Db::connect('mysql:host=unused','user','secret');",
        "DbUnsupportedError",
    );
    fails(
        &mut runtime,
        "Db::connect('no delimiter','user','secret');",
        "DbError",
    );
    assert_eq!(observed.connects.get(), 0);
    run(&mut runtime, CONNECT);
    assert_eq!(observed.connects.get(), 1);
}

#[test]
fn failed_connect_preserves_destination_and_does_not_consume_a_slot() {
    let (mut runtime, observed) = setup(DatabaseOptions {
        max_connections: 1,
        ..Default::default()
    });
    observed.fail_connect.set(true);
    run(&mut runtime, "$db='preserved';");
    fails(&mut runtime, CONNECT, "DbError");
    assert_eq!(
        runtime.get_global("db"),
        Some(&Value::String("preserved".into()))
    );
    observed.fail_connect.set(false);
    run(&mut runtime, CONNECT);
    assert_eq!(observed.connects.get(), 2);
    fails(
        &mut runtime,
        "Db::connect('fixture:test','user','secret');",
        "DbLimitError",
    );
    assert_eq!(observed.connects.get(), 2);
    run(&mut runtime, "Db::close($db);Db::close($db);");
    assert_eq!(observed.closes.get(), 1);
    assert_eq!(observed.drops.get(), 1);
    run(&mut runtime, CONNECT);
}

#[test]
fn connection_close_invalidates_statements_and_releases_resources_once() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$stmt=Db::prepare($db,'SELECT 1');Db::close($db);$closed=$stmt->closed;Db::closeStatement($stmt);Db::closeStatement($stmt);Db::close($db);",
    );
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(observed.closes.get(), 1);
    assert_eq!(observed.statement_closes.get(), 0);
    assert_eq!(observed.drops.get(), 1);
    fails(&mut runtime, "Db::prepare($db,'SELECT 1');", "DbStateError");
}

#[test]
fn statement_and_utf8_sql_limits_cover_exact_and_plus_one_without_leaking_slots() {
    let (mut runtime, observed) = setup(DatabaseOptions {
        max_statements_per_connection: 1,
        max_sql_bytes: 2,
        ..Default::default()
    });
    run(&mut runtime, CONNECT);
    run(&mut runtime, "$stmt=Db::prepare($db,'é');");
    fails(&mut runtime, "Db::prepare($db,'é');", "DbLimitError");
    run(
        &mut runtime,
        "Db::closeStatement($stmt);Db::closeStatement($stmt);",
    );
    fails(&mut runtime, "Db::prepare($db,'éx');", "DbLimitError");
    assert_eq!(observed.prepares.get(), 1);
    assert_eq!(observed.statement_closes.get(), 1);
    run(&mut runtime, "$other=Db::prepare($db,'é');");
}

#[test]
fn orphaned_statement_resources_remain_bounded_until_connection_close() {
    let (mut runtime, observed) = setup(DatabaseOptions {
        max_statements_per_connection: 1,
        ..Default::default()
    });
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$stmt=Db::prepare($db,'SELECT 1');$stmt=null;",
    );
    fails(&mut runtime, "Db::prepare($db,'SELECT 2');", "DbLimitError");
    assert_eq!(observed.prepares.get(), 1);
    run(&mut runtime, "Db::close($db);");
    assert_eq!(observed.drops.get(), 1);
}

#[test]
fn dropping_a_closed_handle_does_not_remove_a_reused_statement_id() {
    let (mut runtime, observed) = setup(DatabaseOptions {
        max_statements_per_connection: 1,
        ..Default::default()
    });
    observed.reuse_ids.set(true);
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$old=Db::prepare($db,'SELECT 1');Db::closeStatement($old);$new=Db::prepare($db,'SELECT 2');$old=null;",
    );
    fails(&mut runtime, "Db::prepare($db,'SELECT 3');", "DbLimitError");
    run(&mut runtime, "Db::close($db);$closed=$new->closed;");
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(observed.drops.get(), 1);
}

#[test]
fn backend_identifier_collision_disconnects_without_exposing_a_new_handle() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    observed.reuse_ids.set(true);
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$first=Db::prepare($db,'SELECT 1');$second='preserved';",
    );
    fails(
        &mut runtime,
        "$second=Db::prepare($db,'SELECT 2');",
        "DbStateError",
    );
    assert_eq!(
        runtime.get_global("second"),
        Some(&Value::String("preserved".into()))
    );
    run(&mut runtime, "$closed=$first->closed;");
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(observed.drops.get(), 1);
}

#[test]
fn native_resources_outlive_runtime_only_when_host_holds_a_handle() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(&mut runtime, "$stmt=Db::prepare($db,'SELECT 1');");
    let retained = runtime.get_global("stmt").unwrap().clone();
    drop(runtime);
    assert_eq!(observed.drops.get(), 0);
    drop(retained);
    assert_eq!(observed.drops.get(), 1);
}

#[derive(Debug)]
struct Spoof;
impl NativeObject for Spoof {
    fn type_name(&self) -> &str {
        "DbConnection"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Ok(Value::Bool(false))
    }
}

#[test]
fn foreign_and_spoofed_handles_cannot_operate_or_close_original_resources() {
    let (mut original, observed) = setup(DatabaseOptions::default());
    let (mut other, _) = setup(DatabaseOptions::default());
    run(&mut original, CONNECT);
    run(&mut original, "$stmt=Db::prepare($db,'SELECT 1');");
    other
        .set_global("foreign", original.get_global("db").unwrap().clone())
        .unwrap();
    other
        .set_global("foreignStmt", original.get_global("stmt").unwrap().clone())
        .unwrap();
    other
        .set_global("spoof", Value::NativeObject(Rc::new(Spoof)))
        .unwrap();
    fails(&mut other, "Db::close($foreign);", "TypeError");
    fails(&mut other, "Db::prepare($foreign,'SELECT 2');", "TypeError");
    fails(&mut other, "Db::closeStatement($foreignStmt);", "TypeError");
    fails(&mut other, "Db::close($spoof);", "TypeError");
    fails(&mut other, "Db::errorInfo($spoof);", "TypeError");
    assert_eq!(observed.closes.get(), 0);
    assert_eq!(observed.statement_closes.get(), 0);
    run(&mut original, "Db::closeStatement($stmt);Db::close($db);");
    assert_eq!(observed.drops.get(), 1);
}

#[test]
fn cleanup_failure_disconnects_and_never_reuses_a_leaked_statement_slot() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(&mut runtime, "$stmt=Db::prepare($db,'SELECT 1');");
    observed.fail_statement_close.set(true);
    fails(&mut runtime, "Db::closeStatement($stmt);", "DbError");
    run(
        &mut runtime,
        "$closed=$db->closed;Db::close($db);Db::closeStatement($stmt);",
    );
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(observed.drops.get(), 1);
    fails(&mut runtime, "Db::prepare($db,'SELECT 2');", "DbStateError");
    run(&mut runtime, CONNECT);
    observed.fail_close.set(true);
    fails(&mut runtime, "Db::close($db);", "DbError");
    run(&mut runtime, "Db::close($db);");
    assert_eq!(observed.drops.get(), 2);
}

#[test]
fn denied_network_blocks_connect_and_cancellation_drops_unpublished_connections() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    // A runtime with denied network access must never reach the driver.
    let mut denied = Runtime::with_options(RuntimeOptions {
        context: scr_runtime::ContextOptions {
            permissions: scr_runtime::PermissionConfig {
                allowed: BTreeSet::new(),
            },
            ..Default::default()
        },
        ..Default::default()
    });
    register(&mut denied, vec![Rc::new(TestDriver(observed.clone()))]).unwrap();
    fails(&mut denied, CONNECT, "PermissionError");
    assert_eq!(observed.connects.get(), 1);
    run(&mut runtime, "Db::close($db);");
    assert_eq!(observed.drops.get(), 1);

    let flag = Arc::new(AtomicBool::new(false));
    let observed = Rc::new(Observed {
        interrupt_connect: Some(flag.clone()),
        ..Default::default()
    });
    let mut interrupted = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    register(
        &mut interrupted,
        vec![Rc::new(TestDriver(observed.clone()))],
    )
    .unwrap();
    let error = interrupted
        .execute_source(CONNECT, &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("execution interrupted"));
    assert_eq!(observed.drops.get(), 1);
    assert!(interrupted.get_global("db").is_none());
    flag.store(false, Ordering::Relaxed);
}

#[test]
fn invalid_host_configuration_and_duplicate_registration_fail_before_connecting() {
    let observed = Rc::new(Observed::default());
    let mut runtime = Runtime::new();
    let driver: Rc<dyn Driver> = Rc::new(TestDriver(observed.clone()));
    let error = register(&mut runtime, vec![driver.clone(), driver.clone()]).unwrap_err();
    assert_eq!(error.kind, "RegistryError");
    let error = register_with_options(
        &mut runtime,
        vec![driver.clone()],
        DatabaseOptions {
            max_connections: 0,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, "TypeError");
    register(&mut runtime, vec![driver.clone()]).unwrap();
    assert_eq!(
        register(&mut runtime, vec![driver]).unwrap_err().kind,
        "RegistryError"
    );
    assert_eq!(observed.connects.get(), 0);
    // A nonempty string-keyed options map is accepted without coercion.
    let options = Value::map(vec![(MapKey::String("mode".into()), Value::Bool(false))]);
    runtime.set_global("options", options).unwrap();
    run(
        &mut runtime,
        "$db=Db::connect('fixture:test','user','secret',$options);",
    );
}

#[test]
fn execution_checks_signature_owner_and_closed_state_before_dispatch() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(&mut runtime, "$stmt=Db::prepare($db,'SELECT 1');");
    fails(&mut runtime, "Db::execute($stmt,1);", "TypeError");
    assert_eq!(observed.executes.get(), 0);
    let (mut other, _) = setup(DatabaseOptions::default());
    other
        .set_global("stmt", runtime.get_global("stmt").unwrap().clone())
        .unwrap();
    fails(&mut other, "Db::execute($stmt);", "TypeError");
    fails(&mut other, "Db::closeCursor($stmt);", "TypeError");
    assert_eq!(observed.executes.get(), 0);
    run(
        &mut runtime,
        "Db::execute($stmt);Db::closeStatement($stmt);",
    );
    fails(&mut runtime, "Db::execute($stmt);", "DbStateError");
    assert_eq!(observed.executes.get(), 1);
}

#[test]
fn fetching_preserves_false_and_null_and_checks_signatures_ownership_and_closed_state() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$stmt=Db::prepare($db,'SELECT 1');$value='preserved';",
    );
    for source in [
        "Db::fetch($db);",
        "Db::fetchAll($stmt,1);",
        "Db::fetchColumn($stmt,'0');",
    ] {
        fails(&mut runtime, source, "TypeError");
    }
    let (mut other, _) = setup(DatabaseOptions::default());
    other
        .set_global("stmt", runtime.get_global("stmt").unwrap().clone())
        .unwrap();
    for source in [
        "Db::fetch($stmt);",
        "Db::fetchAll($stmt);",
        "Db::fetchColumn($stmt);",
    ] {
        fails(&mut other, source, "TypeError");
    }
    assert_eq!(observed.fetches.get(), 0);
    run(
        &mut runtime,
        "$false=Db::fetchColumn($stmt);$null=Db::fetchColumn($stmt,1);",
    );
    assert_eq!(runtime.get_global("false"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("null"), Some(&Value::Null));
    fails(
        &mut runtime,
        "$value=Db::fetchColumn($stmt,2);",
        "DbEndOfResultsError",
    );
    assert_eq!(
        runtime.get_global("value"),
        Some(&Value::String("preserved".into()))
    );
    run(&mut runtime, "Db::closeStatement($stmt);");
    for source in [
        "Db::fetch($stmt);",
        "Db::fetchAll($stmt);",
        "Db::fetchColumn($stmt);",
    ] {
        fails(&mut runtime, source, "DbStateError");
    }
    assert_eq!(observed.fetches.get(), 3);
}

#[test]
fn execution_timeout_invalidates_all_handles_and_preserves_assignment_destination() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$a=Db::prepare($db,'SELECT 1');$b=Db::prepare($db,'SELECT 2');$result='preserved';",
    );
    observed.break_execute.set(true);
    fails(&mut runtime, "$result=Db::execute($a);", "DbTimeoutError");
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("preserved".into()))
    );
    run(
        &mut runtime,
        "$closed=$db->closed && $a->closed && $b->closed;Db::closeCursor($a);",
    );
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(observed.drops.get(), 1);
}

#[test]
fn statement_cleanup_preflight_failure_preserves_handles_and_slots() {
    let (mut runtime, observed) = setup(DatabaseOptions {
        max_statements_per_connection: 1,
        ..Default::default()
    });
    run(&mut runtime, CONNECT);
    run(&mut runtime, "$stmt=Db::prepare($db,'SELECT 1');");
    observed.deny_statement_close.set(true);
    fails(&mut runtime, "Db::closeStatement($stmt);", "DbStateError");
    run(&mut runtime, "$open=!$db->closed && !$stmt->closed;");
    assert_eq!(runtime.get_global("open"), Some(&Value::Bool(true)));
    assert_eq!(observed.statement_closes.get(), 0);
    fails(&mut runtime, "Db::prepare($db,'SELECT 2');", "DbLimitError");
    observed.deny_statement_close.set(false);
    run(
        &mut runtime,
        "Db::closeStatement($stmt);$second=Db::prepare($db,'SELECT 2');",
    );
}

#[test]
fn metadata_and_transaction_dispatch_validate_signatures_ownership_and_utf8_sql_limits() {
    let (mut runtime, observed) = setup(DatabaseOptions {
        max_sql_bytes: 2,
        ..Default::default()
    });
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$s=Db::prepare($db,'é');$n=Db::exec($db,'é');$count=Db::rowCount($s);$id=Db::lastInsertId($db);Db::beginTransaction($db);$active=Db::inTransaction($db);Db::commit($db);Db::rollBack($db);",
    );
    assert_eq!(runtime.get_global("n"), Some(&Value::Int(2)));
    assert_eq!(runtime.get_global("count"), Some(&Value::Int(7)));
    assert_eq!(runtime.get_global("id"), Some(&Value::String("123".into())));
    assert_eq!(observed.transactions.get(), 4);
    fails(&mut runtime, "Db::exec($db,'éa');", "DbLimitError");
    for source in [
        "Db::exec($db,1);",
        "Db::rowCount($db);",
        "Db::lastInsertId($db,1);",
        "Db::beginTransaction($s);",
        "Db::commit($db,1);",
        "Db::rollBack();",
        "Db::inTransaction($s);",
    ] {
        fails(&mut runtime, source, "TypeError");
    }
    let (mut other, _) = setup(DatabaseOptions::default());
    for name in ["db", "s"] {
        other
            .set_global(name, runtime.get_global(name).unwrap().clone())
            .unwrap();
    }
    for source in [
        "Db::exec($db,'é');",
        "Db::rowCount($s);",
        "Db::lastInsertId($db);",
        "Db::beginTransaction($db);",
        "Db::commit($db);",
        "Db::rollBack($db);",
        "Db::inTransaction($db);",
    ] {
        fails(&mut other, source, "TypeError");
    }
    assert_eq!(observed.execs.get(), 1);
    assert_eq!(observed.transactions.get(), 4);
    run(&mut runtime, "Db::close($db);");
    for source in [
        "Db::exec($db,'é');",
        "Db::rowCount($s);",
        "Db::lastInsertId($db);",
        "Db::beginTransaction($db);",
        "Db::commit($db);",
        "Db::rollBack($db);",
        "Db::inTransaction($db);",
    ] {
        fails(&mut runtime, source, "DbStateError");
    }
}

#[test]
fn uncertain_commit_invalidates_every_handle_preserves_destination_and_is_never_retried() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$s=Db::prepare($db,'SELECT 1');$destination='preserved';",
    );
    observed.break_transaction.set(true);
    fails(&mut runtime, "$destination=Db::commit($db);", "DbError");
    assert_eq!(
        runtime.get_global("destination"),
        Some(&Value::String("preserved".into()))
    );
    run(&mut runtime, "$closed=$db->closed && $s->closed;");
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    fails(&mut runtime, "Db::commit($db);", "DbStateError");
    assert_eq!(observed.transactions.get(), 1);
    assert_eq!(observed.drops.get(), 1);
}

fn info_field(runtime: &Runtime, name: &str, key: &str) -> Value {
    let Some(Value::Map(fields)) = runtime.get_global(name) else {
        panic!("expected info map")
    };
    fields
        .iter()
        .find(|(k, _)| *k == MapKey::String(key.into()))
        .unwrap()
        .1
        .clone()
}
#[test]
fn error_info_tracks_shared_failures_preserves_inspection_and_cleanup_and_survives_close() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$a=Db::prepare($db,'SELECT 1');$b=Db::prepare($db,'SELECT 2');$initial=Db::errorInfo($a);",
    );
    assert_eq!(runtime.get_global("initial"), Some(&Value::Null));
    observed.fail_execute.set(true);
    fails(&mut runtime, "Db::execute($a);", "DbError");
    run(
        &mut runtime,
        "$info=Db::errorInfo($a);$connection=Db::errorInfo($db);$other=Db::errorInfo($b);Db::lastInsertId($db);Db::rowCount($a);Db::inTransaction($db);Db::closeStatement($a);$closed=Db::errorInfo($a);",
    );
    assert_eq!(
        info_field(&runtime, "info", "category"),
        Value::String("constraint".into())
    );
    assert_eq!(
        info_field(&runtime, "info", "operation"),
        Value::String("execute".into())
    );
    assert_eq!(
        info_field(&runtime, "info", "sqlState"),
        Value::String("23000".into())
    );
    assert_eq!(runtime.get_global("info"), runtime.get_global("connection"));
    assert_eq!(runtime.get_global("info"), runtime.get_global("closed"));
    assert_eq!(runtime.get_global("other"), Some(&Value::Null));
    observed.fail_execute.set(false);
    run(
        &mut runtime,
        "Db::execute($b);$cleared=Db::errorInfo($db);$unchanged=Db::errorInfo($a);",
    );
    assert_eq!(runtime.get_global("cleared"), Some(&Value::Null));
    assert_eq!(runtime.get_global("unchanged"), runtime.get_global("info"));
    observed.break_execute.set(true);
    fails(&mut runtime, "Db::execute($b);", "DbTimeoutError");
    run(
        &mut runtime,
        "$timeout=Db::errorInfo($b);Db::close($db);$after=Db::errorInfo($db);",
    );
    assert_eq!(
        info_field(&runtime, "timeout", "category"),
        Value::String("timeout".into())
    );
    assert_eq!(runtime.get_global("timeout"), runtime.get_global("after"));
    let (mut other, _) = setup(DatabaseOptions::default());
    for handle in ["db", "a", "b"] {
        other
            .set_global("foreign", runtime.get_global(handle).unwrap().clone())
            .unwrap();
        fails(&mut other, "Db::errorInfo($foreign);", "TypeError");
    }
    fails(&mut runtime, "Db::errorInfo(1);", "TypeError");
}
#[test]
fn preflight_errors_and_scalar_exhaustion_preserve_failure_history_and_null_success_clears_it() {
    let (mut runtime, observed) = setup(DatabaseOptions::default());
    run(&mut runtime, CONNECT);
    run(&mut runtime, "$s=Db::prepare($db,'SELECT 1');");
    observed.fail_execute.set(true);
    fails(&mut runtime, "Db::execute($s);", "DbError");
    run(&mut runtime, "$before=Db::errorInfo($s);");
    fails(&mut runtime, "Db::execute($s,1);", "TypeError");
    fails(
        &mut runtime,
        "Db::fetchColumn($s,2);",
        "DbEndOfResultsError",
    );
    run(&mut runtime, "$after=Db::errorInfo($s);");
    assert_eq!(runtime.get_global("before"), runtime.get_global("after"));
    run(
        &mut runtime,
        "$value=Db::fetchColumn($s,1);$cleared=Db::errorInfo($s);",
    );
    assert_eq!(runtime.get_global("value"), Some(&Value::Null));
    assert_eq!(runtime.get_global("cleared"), Some(&Value::Null));
}

#[test]
fn error_info_output_limits_preserve_destination_and_shared_snapshots_are_counted_once() {
    let observed = Rc::new(Observed::default());
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(256),
        ..Default::default()
    });
    register(&mut runtime, vec![Rc::new(TestDriver(observed.clone()))]).unwrap();
    run(&mut runtime, CONNECT);
    run(
        &mut runtime,
        "$s=Db::prepare($db,'SELECT 1');$info='preserved';",
    );
    observed.fail_execute.set(true);
    fails(&mut runtime, "Db::execute($s);", "DbError");
    fails(&mut runtime, "$info=Db::errorInfo($s);", "DbLimitError");
    assert_eq!(
        runtime.get_global("info"),
        Some(&Value::String("preserved".into()))
    );
    observed.fail_execute.set(false);
    run(
        &mut runtime,
        "Db::execute($s);$info=Db::errorInfo($s);Db::close($db);",
    );
    assert_eq!(runtime.get_global("info"), Some(&Value::Null));
}
