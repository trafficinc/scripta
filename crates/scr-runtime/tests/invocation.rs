use scr_runtime::{
    ExecutionOutcome, NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value,
};

fn register_bridge(runtime: &mut Runtime) {
    runtime
        .register(
            "Host::invoke",
            NativeFunction::reentrant(
                NativeSignature::new(vec![NativeType::Callable], NativeType::Any),
                |context, args| context.invoke(args[0].clone(), &[]),
            ),
        )
        .unwrap();
}

#[test]
fn reentrant_callbacks_preserve_exception_kinds_values_and_runtime_recovery() {
    let mut runtime = Runtime::new();
    register_bridge(&mut runtime);
    let mut output = Vec::new();
    runtime.execute_source("try { Host::invoke(function () { return 1 / 0; }); } catch (ArithmeticError $e) { echo 'arithmetic'; } try { Host::invoke(function () { throw 7; }); } catch (int $e) { if ($e === 7) echo '7'; }", &mut output).unwrap();
    assert_eq!(output, b"arithmetic7");
    runtime
        .execute_source(
            "function fail() { return 1 + 'x'; } function value() { return 42; }",
            &mut output,
        )
        .unwrap();
    let error = runtime
        .invoke(runtime.get_callable("fail").unwrap(), &[], &mut output)
        .unwrap_err();
    assert_eq!(error.exception_kind(), Some("TypeError"));
    assert!(error.diagnostic().unwrap().source.is_some());
    assert_eq!(
        runtime
            .invoke(runtime.get_callable("value").unwrap(), &[], &mut output)
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn exits_are_distinct_from_completion_in_source_and_host_invocations() {
    let mut runtime = Runtime::new();
    let mut output = Vec::new();
    assert_eq!(
        runtime
            .execute_source_outcome("function stop() { exit 0; }", &mut output)
            .unwrap(),
        ExecutionOutcome::Completed
    );
    let error = runtime
        .invoke(runtime.get_callable("stop").unwrap(), &[], &mut output)
        .unwrap_err();
    assert_eq!(error.exit_code(), Some(0));
    assert_eq!(error.exception_kind(), None);
    assert_eq!(
        runtime
            .execute_source_outcome("exit 0;", &mut output)
            .unwrap(),
        ExecutionOutcome::Exited(0)
    );
    assert_eq!(runtime.execute_source("exit 7;", &mut output).unwrap(), 7);
}

#[test]
fn callback_budgets_foreign_owners_and_recursion_remain_enforced() {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(20),
        ..Default::default()
    });
    register_bridge(&mut runtime);
    runtime
        .execute_source(
            "function spin() { Host::invoke(function () { while (true) {} }); }",
            &mut Vec::new(),
        )
        .unwrap();
    let error = runtime
        .invoke(runtime.get_callable("spin").unwrap(), &[], &mut Vec::new())
        .unwrap_err();
    assert!(error.is_interrupted());
    assert_eq!(error.exception_kind(), None);
    let mut other = Runtime::new();
    other
        .execute_source("function f() { return 1; }", &mut Vec::new())
        .unwrap();
    let mut target = Runtime::new();
    let error = target
        .invoke(other.get_callable("f").unwrap(), &[], &mut Vec::new())
        .unwrap_err();
    assert_eq!(error.exception_kind(), None);
    assert!(
        error
            .diagnostic()
            .unwrap()
            .message
            .contains("different runtime")
    );
    let mut limited = Runtime::with_options(RuntimeOptions {
        recursion_limit: 1,
        ..Default::default()
    });
    limited
        .execute_source("function f() { return f(); }", &mut Vec::new())
        .unwrap();
    assert!(
        limited
            .invoke(limited.get_callable("f").unwrap(), &[], &mut Vec::new())
            .unwrap_err()
            .diagnostic()
            .unwrap()
            .message
            .contains("maximum call depth")
    );
}
