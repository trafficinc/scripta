//! Common checks for optimized-versus-reference execution. Suites own their
//! optimization switches, hit assertions, and runtime-specific identity checks.
use crate::{Runtime, RuntimeError, RuntimeOptions, Value};

type ExecutionResult = Result<u8, RuntimeError>;

pub(crate) fn runtimes(options: RuntimeOptions) -> (Runtime, Runtime) {
    (
        Runtime::with_options(options.clone()),
        Runtime::with_options(options),
    )
}

pub(crate) fn compare_execution(
    fast: &mut Runtime,
    reference: &mut Runtime,
    context: &str,
    execute: impl Fn(&mut Runtime, &mut Vec<u8>) -> ExecutionResult,
) -> ExecutionResult {
    let mut a = Vec::new();
    let mut b = Vec::new();
    let result = execute(fast, &mut a);
    // RuntimeError equality includes diagnostics, source spans and stack frames.
    assert_eq!(result, execute(reference, &mut b), "{context}");
    assert_eq!(a, b, "{context}: output");
    assert_eq!(fast.steps, reference.steps, "{context}: checkpoints");
    assert_eq!(
        fast.environments.live, reference.environments.live,
        "{context}: live scopes"
    );
    for runtime in [&*fast, &*reference] {
        assert_eq!(runtime.environments.current, 0, "{context}: active scope");
        assert!(runtime.stack.is_empty(), "{context}: active frames");
        assert!(
            runtime
                .prepared_stack
                .as_ref()
                .is_none_or(|stack| stack.is_empty()),
            "{context}: prepared operands"
        );
        assert!(
            runtime
                .compact_stack
                .as_ref()
                .is_none_or(|stack| stack.is_empty()),
            "{context}: compact operands"
        );
        assert!(
            runtime.environments.temporary_roots.is_empty(),
            "{context}: temporary roots"
        );
        assert!(
            runtime.argument_buffers.len() <= crate::MAX_ARGUMENT_BUFFERS,
            "{context}: argument buffer count"
        );
        assert!(
            runtime
                .argument_buffers
                .iter()
                .all(|buffer| buffer.is_empty()
                    && (1..=crate::MAX_ARGUMENT_CAPACITY).contains(&buffer.capacity())),
            "{context}: retained argument buffers"
        );
    }
    let globals = &fast.environments.scope(0).variables;
    let others = &reference.environments.scope(0).variables;
    assert_eq!(globals.len(), others.len(), "{context}: global names");
    for (name, value) in globals {
        let other = others
            .get(name)
            .unwrap_or_else(|| panic!("{context}: missing global {name}"));
        compare_value(value, other, &format!("{context}: ${name}"));
    }
    result
}

pub(crate) fn compare_source(
    fast: &mut Runtime,
    reference: &mut Runtime,
    source: &str,
) -> ExecutionResult {
    compare_execution(fast, reference, source, |runtime, output| {
        runtime.execute_source(source, output)
    })
}

pub(crate) fn compare_with_recovery(
    fast: &mut Runtime,
    reference: &mut Runtime,
    source: &str,
    recovery: &str,
) -> ExecutionResult {
    let first = compare_source(fast, reference, source);
    let _ = compare_source(fast, reference, recovery);
    first
}

fn compare_value(a: &Value, b: &Value, context: &str) {
    match (a, b) {
        // Independent runtimes create independent identities, including handles
        // nested in collections. Suites must check their behavior/ownership.
        (Value::Callable(_), Value::Callable(_))
        | (Value::NativeObject(_), Value::NativeObject(_)) => {}
        (Value::Float(a), Value::Float(b)) => assert_eq!(a.to_bits(), b.to_bits(), "{context}"),
        (Value::List(a), Value::List(b)) => {
            assert_eq!(a.len(), b.len(), "{context}: list length");
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                compare_value(a, b, &format!("{context}[{index}]"));
            }
        }
        (Value::Map(a), Value::Map(b)) => {
            assert_eq!(a.len(), b.len(), "{context}: map length");
            for ((key, a), (other, b)) in a.iter().zip(b) {
                assert_eq!(key, other, "{context}: map key/order");
                compare_value(a, b, &format!("{context}[{key:?}]"));
            }
        }
        _ => assert_eq!(a, b, "{context}"),
    }
}
