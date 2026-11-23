use super::*;
use crate::RuntimeOptions;
fn prepared(disabled: bool, definition: &str, warm: &str) -> Runtime {
    let mut runtime = Runtime::new();
    runtime.disable_numeric_parameter_scopes = disabled;
    runtime.execute_source(definition, &mut Vec::new()).unwrap();
    runtime.execute_source(warm, &mut Vec::new()).unwrap();
    assert_eq!(runtime.numeric_parameter_scopes_reused, 0);
    runtime
}
fn compare(options: RuntimeOptions, source: &str) -> Runtime {
    let mut fast = prepared(false, "function f($x:int):int{return$x+1;}", "f(0);f(1);");
    let mut old = prepared(true, "function f($x:int):int{return$x+1;}", "f(0);f(1);");
    fast.options = options.clone();
    old.options = options;
    let _ =
        crate::test_support::compare_with_recovery(&mut fast, &mut old, source, "$recovered=1;");
    fast
}
#[test]
fn cached_parameter_scopes_preserve_exact_limits_errors_and_recovery() {
    let source = "$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}";
    for scopes in 0..8 {
        for steps in 0..60 {
            compare(
                RuntimeOptions {
                    max_scopes: Some(scopes),
                    instruction_limit: Some(steps),
                    ..RuntimeOptions::default()
                },
                source,
            );
        }
    }
    for steps in 500..660 {
        compare(
            RuntimeOptions {
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            },
            source,
        );
    }
    for depth in 0..5 {
        compare(
            RuntimeOptions {
                recursion_limit: depth,
                ..RuntimeOptions::default()
            },
            source,
        );
    }
    for bytes in [0, 1, 32, 1024] {
        assert_eq!(
            compare(
                RuntimeOptions {
                    max_value_bytes: Some(bytes),
                    ..RuntimeOptions::default()
                },
                source
            )
            .numeric_parameter_scopes_reused,
            0
        );
    }
    for source in [
        "$seen=f(4);",
        "$seen=f(1.0);",
        "$seen=f(9223372036854775807);",
        "$seen=f();",
        "$seen=f(1,2);",
    ] {
        compare(RuntimeOptions::default(), source);
    }
    assert!(compare(RuntimeOptions::default(), source).numeric_parameter_scopes_reused > 0);
}
#[test]
fn cold_nonnumeric_and_unsupported_calls_cannot_enter_an_empty_parameter_scope() {
    let mut runtime = prepared(false, "function f($x){return$x;}", "f(0);f(1);");
    runtime
        .execute_source("$s=f('text');$list=f([1,2]);$b=f(true);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.numeric_parameter_scopes_reused, 0);
    runtime
        .execute_source("$i=f(3);$float=f(-0.0);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.numeric_parameter_scopes_reused, 2);
    assert_eq!(runtime.get_global("s"), Some(&Value::String("text".into())));
    for (source, warm) in [
        ("function f($x=1){return$x+1;}", "f();f();f();"),
        ("function f(...$x){return 1;}", "f(1);f(2);f(3);"),
        ("function f($x){$x+=1;return$x;}", "f(1);f(2);f(3);"),
    ] {
        let mut runtime = Runtime::new();
        runtime.execute_source(source, &mut Vec::new()).unwrap();
        runtime.execute_source(warm, &mut Vec::new()).unwrap();
        assert_eq!(runtime.numeric_parameter_scopes_reused, 0);
    }
    let mut runtime = Runtime::new();
    runtime
        .execute_source("function f($x){return$x;}f(1);f(2);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.numeric_parameter_scopes_reused, 0);
    runtime.execute_source("f(3);", &mut Vec::new()).unwrap();
    assert_eq!(runtime.numeric_parameter_scopes_reused, 1);
}
#[test]
fn numeric_parameter_records_restore_boundary_flags_after_success_and_type_failure() {
    for disabled in [false, true] {
        for (definition, warm, call) in [
            (
                "function f($x:int):int{return$x+1;}",
                "f(0);f(1);",
                "f(1.0);",
            ),
            (
                "function f($x:int):float{return$x;}",
                "try{f(0);}catch(TypeError $e){}try{f(1);}catch(TypeError $e){}",
                "f(2);",
            ),
        ] {
            let mut runtime = prepared(disabled, definition, warm);
            assert!(runtime.execute_source(call, &mut Vec::new()).is_err());
            assert_eq!(runtime.environments.live, 1);
            assert!(runtime.stack.is_empty());
            let previous = runtime.enter_empty_scope(0, Span::default()).ok().unwrap();
            let scope = runtime.environments.scope(runtime.environments.current);
            assert!(!scope.function_boundary);
            assert!(!scope.captured);
            assert!(scope.variables.is_empty());
            runtime.environments.leave_empty(previous);
            runtime
                .execute_source("$global=7;$seen=0;{$seen=$global;}", &mut Vec::new())
                .unwrap();
            assert_eq!(runtime.get_global("seen"), Some(&Value::Int(7)));
        }
    }
}
#[test]
fn cancellation_at_a_warm_return_preserves_diagnostics_and_releases_both_scopes() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let mut runtime = prepared(disabled, "function f($x){return$x+1;}", "f(0);f(1);");
        let flag = Arc::new(AtomicBool::new(true));
        runtime.options.interrupt_flag = Some(flag.clone());
        let Value::Callable(f) = runtime.function("f", Span::default()).ok().unwrap() else {
            panic!()
        };
        let Err(crate::Signal::Interrupted(error)) =
            runtime.call_callable(f, vec![Value::Int(4)], Span::default(), &mut Vec::new())
        else {
            panic!()
        };
        errors.push(error);
        assert_eq!(runtime.environments.live, 1);
        assert_eq!(runtime.environments.current, 0);
        assert!(runtime.stack.is_empty());
        assert_eq!(
            runtime.numeric_parameter_scopes_reused,
            usize::from(!disabled)
        );
        flag.store(false, Ordering::Relaxed);
        runtime
            .execute_source("$seen=f(5);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(6)));
    }
    assert_eq!(errors[0], errors[1]);
}
#[test]
fn warm_parameter_cache_does_not_keep_captured_resources_or_unused_objects() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Probe(Rc<Cell<usize>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl NativeObject for Probe {
        fn type_name(&self) -> &str {
            "Probe"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("none".into())
        }
    }
    for disabled in [false, true] {
        let drops = Rc::new(Cell::new(0));
        let d = drops.clone();
        let mut runtime = Runtime::new();
        runtime.disable_numeric_parameter_scopes = disabled;
        runtime
            .register(
                "probe",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| Ok(Value::NativeObject(Rc::new(Probe(d.clone())))),
                ),
            )
            .unwrap();
        runtime.execute_source("function make(){$resource=probe();return function($x){return$x+1;};}$held=make();$held(0);$held(1);",&mut Vec::new()).unwrap();
        let Value::Callable(held) = runtime.get_global("held").unwrap().clone() else {
            panic!()
        };
        runtime.set_global("held", Value::Null).unwrap();
        for _ in 0..512 {
            let Value::Callable(dead) = Value::native_function(
                "dead",
                NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
                    Ok(Value::Null)
                }),
            ) else {
                panic!()
            };
            runtime.environments.track_function(&dead.0);
        }
        let value = runtime
            .call_callable(
                held.clone(),
                vec![Value::Int(4)],
                Span::default(),
                &mut Vec::new(),
            )
            .ok()
            .unwrap();
        assert_eq!(value, Value::Int(5));
        assert_eq!(drops.get(), 0);
        assert_eq!(
            runtime.numeric_parameter_scopes_reused,
            usize::from(!disabled)
        );
        drop(held);
        runtime.collect_garbage();
        assert_eq!(drops.get(), 1);
        assert_eq!(runtime.environments.live, 1);
        runtime
            .execute_source(
                "function discard($unused){return 1;}discard(0);discard(1);$seen=discard(probe());",
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(drops.get(), 2);
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(1)));
    }
}
