use scr_runtime::{
    MapKey, NativeFunction, NativeObject, NativeSignature, NativeType, Runtime, RuntimeOptions,
    Value,
};
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
const SIZE: usize = 256;
fn map(start: i64) -> Value {
    Value::Map(
        (0..SIZE)
            .map(|i| (MapKey::Int(start + i as i64), Value::Int(start + i as i64)))
            .collect(),
    )
}
fn run(runtime: &mut Runtime, source: &str) -> Result<u8, scr_runtime::RuntimeError> {
    runtime.execute_source(source, &mut Vec::new())
}
#[derive(Debug)]
struct Probe;
impl NativeObject for Probe {
    fn type_name(&self) -> &str {
        "Probe"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("unknown".into())
    }
}
#[test]
fn indexed_maps_preserve_order_typed_keys_nested_copies_and_handle_identity() {
    let mut runtime = Runtime::new();
    let Value::Map(mut entries) = map(0) else {
        unreachable!()
    };
    entries[0].1 = Value::list(vec![Value::list(vec![Value::Int(1)])]);
    entries[1].1 = Value::NativeObject(Rc::new(Probe));
    entries[2].1 = Value::native_function(
        "identity",
        NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
            Ok(Value::Null)
        }),
    );
    entries.insert(
        1,
        (MapKey::String("0".into()), Value::String("text".into())),
    );
    let order: Vec<_> = entries.iter().map(|(k, _)| k.clone()).collect();
    runtime.set_global("map", Value::Map(entries)).unwrap();
    run(
        &mut runtime,
        r#"
        $copy = $map; $warm = $map[255]; $text = $map["0"];
        $map[0][0][] = 2;
        $sameObject = $map[1] === $copy[1]; $sameCallable = $map[2] === $copy[2];
        $old = $copy[0][0]; $new = $map[0][0];
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("old"),
        Some(&Value::list(vec![Value::Int(1)]))
    );
    assert_eq!(
        runtime.get_global("new"),
        Some(&Value::list(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(
        runtime.get_global("text"),
        Some(&Value::String("text".into()))
    );
    for name in ["sameObject", "sameCallable"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Bool(true)));
    }
    let Some(Value::Map(entries)) = runtime.get_global("map") else {
        panic!()
    };
    assert_eq!(
        entries.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        order
    );
}
#[test]
fn replacements_destructuring_growth_and_reused_parameter_scopes_keep_current_values() {
    let mut runtime = Runtime::new();
    runtime.set_global("map", map(0)).unwrap();
    run(
        &mut runtime,
        "$warm = $map[0]; $map[256] = 900; $grown = $map[256];",
    )
    .unwrap();
    assert_eq!(runtime.get_global("grown"), Some(&Value::Int(900)));
    let Value::Map(mut replacement) = map(0) else {
        unreachable!()
    };
    replacement.reverse();
    replacement.last_mut().unwrap().1 = Value::Int(42);
    runtime.set_global("map", Value::Map(replacement)).unwrap();
    run(&mut runtime, "$host = $map[0];").unwrap();
    assert_eq!(runtime.get_global("host"), Some(&Value::Int(42)));
    runtime.set_global("replacement", map(1000)).unwrap();
    run(&mut runtime, "[$map] = [$replacement]; $destructured = $map[1000]; $map = $replacement; $assigned = $map[1001];").unwrap();
    assert_eq!(runtime.get_global("destructured"), Some(&Value::Int(1000)));
    assert_eq!(runtime.get_global("assigned"), Some(&Value::Int(1001)));
    runtime.set_global("first", map(0)).unwrap();
    runtime.set_global("second", map(1000)).unwrap();
    run(&mut runtime, "function lookup($map, $key) { return $map[$key]; } $a = lookup($first, 0); $b = lookup($second, 1000);").unwrap();
    assert_eq!(runtime.get_global("a"), Some(&Value::Int(0)));
    assert_eq!(runtime.get_global("b"), Some(&Value::Int(1000)));
    let error = run(&mut runtime, "lookup($second, 0);").unwrap_err();
    assert!(error.message.starts_with("IndexError: map key not found"));
    assert_eq!(error.trace.last().unwrap().function, "lookup");
}
#[test]
fn cached_map_updates_keep_exact_budgets_and_failed_destinations() {
    let slot = std::mem::size_of::<Value>();
    let key = MapKey::LOGICAL_BYTES;
    let exact = slot + SIZE * (slot + key);
    for limit in [exact, exact + slot + key] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        runtime.set_global("map", map(0)).unwrap();
        run(&mut runtime, "$warm = $map[0]; $map[0] += 1;").unwrap();
        let previous = runtime.get_global("map").cloned();
        let source = "$map[256] = 9;";
        let result = run(&mut runtime, source);
        if limit == exact {
            let error = result.unwrap_err();
            assert!(
                error
                    .message
                    .contains("execution value byte limit exceeded")
            );
            assert_eq!(&source[error.span.start..error.span.end], "$map[256]");
            assert_eq!(runtime.get_global("map"), previous.as_ref());
        } else {
            result.unwrap();
            run(&mut runtime, "$new = $map[256];").unwrap();
            assert_eq!(runtime.get_global("new"), Some(&Value::Int(9)));
        }
        for source in ["$map[true] = 1;", "$map[0][0] = 1;", "$map[999] += 1;"] {
            let previous = runtime.get_global("map").cloned();
            let error = run(&mut runtime, source).unwrap_err();
            assert!(
                error.message.starts_with("TypeError:") || error.message.starts_with("IndexError:")
            );
            assert_eq!(runtime.get_global("map"), previous.as_ref());
        }
    }
}
#[test]
fn warmed_indexes_preserve_cancellation_and_reject_malformed_host_maps() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    runtime.set_global("map", map(0)).unwrap();
    run(&mut runtime, "$warm = $map[0];").unwrap();
    let previous = runtime.get_global("map").cloned();
    flag.store(true, Ordering::Relaxed);
    assert!(
        run(&mut runtime, "$map[0] = 99;")
            .unwrap_err()
            .message
            .contains("execution interrupted")
    );
    assert_eq!(runtime.get_global("map"), previous.as_ref());
    flag.store(false, Ordering::Relaxed);
    let Value::Map(mut invalid) = map(0) else {
        unreachable!()
    };
    invalid[1].0 = MapKey::Int(0);
    assert!(
        runtime
            .set_global("map", Value::Map(invalid))
            .unwrap_err()
            .message
            .contains("duplicate host map key")
    );
    assert_eq!(runtime.get_global("map"), previous.as_ref());
    run(&mut runtime, "$map[0] = 42; $recovered = $map[0];").unwrap();
    assert_eq!(runtime.get_global("recovered"), Some(&Value::Int(42)));
}
#[test]
fn map_indexes_do_not_retain_native_resources_after_scope_or_runtime_cleanup() {
    let object = Rc::new(Probe);
    let weak = Rc::downgrade(&object);
    let Value::Map(mut entries) = map(0) else {
        unreachable!()
    };
    entries[0].1 = Value::NativeObject(object);
    let mut runtime = Runtime::new();
    runtime.set_global("map", Value::Map(entries)).unwrap();
    run(&mut runtime, "$warm = $map[255]; $map = null;").unwrap();
    runtime.collect_garbage();
    assert!(weak.upgrade().is_none());
    let mut other = Runtime::new();
    other.set_global("map", map(1000)).unwrap();
    run(&mut other, "$value = $map[1000];").unwrap();
    assert_eq!(other.get_global("value"), Some(&Value::Int(1000)));
}
