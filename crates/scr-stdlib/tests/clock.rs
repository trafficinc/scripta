use scr_runtime::{Runtime, RuntimeOptions, Value};
use std::{
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::Duration,
};

fn runtime(options: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime
        .execute_source(&format!("<?scripta {source}"), &mut Vec::new())
        .unwrap();
}
fn timestamp(runtime: &mut Runtime) -> f64 {
    run(runtime, "$time = Clock::monotonicMs();");
    let Some(Value::Float(time)) = runtime.get_global("time") else {
        panic!("expected a float timestamp")
    };
    assert!(time.is_finite() && *time >= 0.0);
    *time
}

#[test]
fn clock_is_nondecreasing_and_uses_one_origin_across_runtimes_without_permissions() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut first = runtime(options);
    let start = timestamp(&mut first);
    let mut previous = start;
    for _ in 0..100 {
        let current = timestamp(&mut first);
        assert!(current >= previous);
        previous = current;
    }
    thread::sleep(Duration::from_millis(20));
    let mut second = runtime(RuntimeOptions::default());
    let later = timestamp(&mut second);
    assert!(
        later - start >= 10.0,
        "runtime creation reset the clock origin"
    );
    assert!(timestamp(&mut first) >= later);
}

#[test]
fn clock_validates_arity_preserves_failed_assignment_and_obeys_scalar_budgets() {
    let mut clock = runtime(RuntimeOptions::default());
    run(&mut clock, "$result = 42;");
    let error = clock
        .execute_source(
            "<?scripta $result = Clock::monotonicMs(1);",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.starts_with("TypeError:"));
    assert_eq!(clock.get_global("result"), Some(&Value::Int(42)));
    let mut exact = runtime(RuntimeOptions {
        max_value_bytes: Some(size_of::<Value>()),
        ..Default::default()
    });
    timestamp(&mut exact);
    let mut small = runtime(RuntimeOptions {
        max_value_bytes: Some(size_of::<Value>() - 1),
        ..Default::default()
    });
    let error = small
        .execute_source("<?scripta $result = Clock::monotonicMs();", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("value byte limit"), "{error}");
    assert!(small.get_global("result").is_none());
}

#[test]
fn clock_does_not_bypass_cancellation() {
    let mut runtime = runtime(RuntimeOptions {
        interrupt_flag: Some(Arc::new(AtomicBool::new(true))),
        ..Default::default()
    });
    runtime.set_global("result", Value::Int(42)).unwrap();
    let error = runtime
        .execute_source("<?scripta $result = Clock::monotonicMs();", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("execution interrupted"), "{error}");
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
}
