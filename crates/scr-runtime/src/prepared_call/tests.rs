use super::*;
use crate::{NativeFunction, NativeSignature, NativeType, RuntimeOptions};

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let (mut fast, mut slow) = crate::test_support::runtimes(options);
    fast.force_numeric_calls = true;
    slow.force_numeric_calls = true;
    slow.disable_prepared_calls = true;
    let _ =
        crate::test_support::compare_with_recovery(&mut fast, &mut slow, source, "$recovered=7;");
    fast
}
const FIB: &str = "function f($n:int):int{if($n<2){return $n;}return f($n-1)+f($n-2);}";

#[test]
fn flat_recursion_reuses_reserved_storage_without_recursive_environments() {
    let mut runtime = Runtime::new();
    runtime.force_numeric_calls = true;
    runtime
        .execute_source(&format!("{FIB} f(3);"), &mut Vec::new())
        .unwrap();
    let values = runtime.prepared_stack.as_ref().unwrap().values.as_ptr();
    let capacity = runtime.prepared_stack.as_ref().unwrap().values.capacity();
    let frames = runtime.prepared_stack.as_ref().unwrap().frames.capacity();
    let trace = runtime.stack.capacity();
    let entries = runtime.environments.scope_entries;
    runtime
        .execute_source("$answer=f(20);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(6765)));
    assert_eq!(runtime.environments.scope_entries - entries, 1);
    assert_eq!(
        runtime.prepared_stack.as_ref().unwrap().values.as_ptr(),
        values
    );
    assert_eq!(
        runtime.prepared_stack.as_ref().unwrap().values.capacity(),
        capacity
    );
    assert_eq!(
        runtime.prepared_stack.as_ref().unwrap().frames.capacity(),
        frames
    );
    assert_eq!(runtime.stack.capacity(), trace);
    assert!(runtime.prepared_stack.as_ref().unwrap().is_empty());
    runtime.options.instruction_limit = Some(20);
    assert!(
        runtime
            .execute_source("$answer=f(20);", &mut Vec::new())
            .is_err()
    );
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(6765)));
    assert!(runtime.prepared_stack.as_ref().unwrap().is_empty());
    runtime.options.instruction_limit = None;
    runtime
        .execute_source("$answer=f(4);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(3)));
}

#[test]
fn recursive_plans_preserve_results_frames_and_scalar_errors() {
    for source in [
        format!("{FIB} $answer=3+f(12)*7;"),
        "function f($n){if($n<1)return 1;return 2+f($n-1)*3;}$answer=f(20);".into(),
        "function f($n){if($n===0)return true;return f($n-1)&&true;}$answer=f(8);".into(),
        "function f($n){if($n===0)return null;return f($n-1);}$answer=f(4);".into(),
        "function f($n):int{if($n===0)return true;return f($n-1);}$answer=7;$answer=f(4);".into(),
        "function f($n){if($n===0)return 1/0;return f($n-1);}$answer=7;try{$answer=f(4);}catch(ArithmeticError $e){$kind=$e->message;}".into(),
        "function f($n){if($n===0)return 9223372036854775807;return f($n-1)+1;}$answer=7;$answer=f(4);".into(),
        "function f($n){if($n===0)return 1;return f($n<0);}$answer=f(2);".into(),
    ] {assert!(compare(&source,RuntimeOptions::default()).prepared_calls_executed>0,"{source}");}
}

#[test]
fn exact_instruction_and_call_limits_preserve_failed_destinations_and_recovery() {
    let source = format!("{FIB} $answer=7;$answer=f(6);");
    for budget in 0..400 {
        compare(
            &source,
            RuntimeOptions {
                instruction_limit: Some(budget),
                ..Default::default()
            },
        );
    }
    for depth in [0, 1, 2, 4, 63, 64, 65, 1000] {
        compare(
            "function f($n){if($n===0)return 1;return f($n-1);}$answer=7;$answer=f(70);",
            RuntimeOptions {
                recursion_limit: depth,
                ..Default::default()
            },
        );
    }
}

#[test]
fn configured_scope_and_value_limits_keep_existing_accounting() {
    let source = format!("{FIB} $answer=f(5);");
    for limit in 0..22 {
        assert_eq!(
            compare(
                &source,
                RuntimeOptions {
                    max_scopes: Some(limit),
                    ..Default::default()
                }
            )
            .prepared_calls_executed,
            0
        );
    }
    for limit in [
        0,
        1,
        std::mem::size_of::<Value>() - 1,
        std::mem::size_of::<Value>(),
        4096,
    ] {
        assert_eq!(
            compare(
                &source,
                RuntimeOptions {
                    max_value_bytes: Some(limit),
                    ..Default::default()
                }
            )
            .prepared_calls_executed,
            0
        );
    }
}

#[test]
fn unsupported_shapes_and_non_integer_owners_fall_back_without_restarting() {
    for source in [
        "function f($n){if($n<1)return 0;$n-=1;return f($n);}$answer=f(4);",
        "function f($n=3){if($n<1)return 0;return f($n-1);}$answer=f();",
        "function f($n,...$rest){if($n<1)return 0;return f($n-1);}$answer=f(4);",
        "function f($n,$unused){if($n<1)return 0;return f($n-1,$unused);}$answer=f(4,'text');",
        "function f($n){if($n<1.0)return 0.0;return f($n-1.0);}$answer=f(4.0);",
        "function outer($x){function f($n){if($n<1)return $x;return f($n-1);}return f(3);}$answer=outer(7);",
        "class C{function f($n){if($n<1)return 0;return $this->f($n-1);}}$c=new C();$answer=$c->f(4);",
        "function f($n){try{if($n<1)return 0;return f($n-1);}finally{}}$answer=f(4);",
    ] {
        assert_eq!(
            compare(source, RuntimeOptions::default()).prepared_calls_executed,
            0,
            "{source}"
        );
    }
}

#[test]
fn changed_global_binding_is_checked_for_cached_host_held_callables() {
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.force_numeric_calls = true;
        runtime.disable_prepared_calls = disabled;
        runtime
            .execute_source(
                &format!("{FIB} f(6);function g($n){{return 41;}}"),
                &mut Vec::new(),
            )
            .unwrap();
        let held = runtime.get_callable("f").unwrap();
        let replacement = runtime.get_callable("g").unwrap();
        runtime
            .environments
            .scope_mut(0)
            .functions
            .insert("f".into(), replacement);
        let before = runtime.prepared_calls_executed;
        assert_eq!(
            runtime
                .invoke(held, &[Value::Int(4)], &mut Vec::new())
                .unwrap(),
            Value::Int(82)
        );
        assert_eq!(runtime.prepared_calls_executed, before);
        assert!(runtime.stack.is_empty());
        assert_eq!(runtime.environments.live, 1);
    }
}

#[test]
fn host_and_native_reentry_preserve_exception_signals_and_shared_budgets() {
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.force_numeric_calls = true;
        runtime.disable_prepared_calls = disabled;
        runtime
            .register(
                "Host::call",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::Int],
                        NativeType::Any,
                    ),
                    |context, args| context.invoke(args[0].clone(), &args[1..]),
                ),
            )
            .unwrap();
        runtime
            .execute_source(&format!("{FIB} $answer=Host::call(f,12);"), &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("answer"), Some(&Value::Int(144)));
        let f = runtime.get_callable("f").unwrap();
        assert_eq!(
            runtime
                .invoke(f, &[Value::Int(8)], &mut Vec::new())
                .unwrap(),
            Value::Int(21)
        );
        runtime.execute_source("function bad($n){if($n===0)return 1/0;return bad($n-1);}$caught=0;try{Host::call(bad,4);}catch(ArithmeticError $e){$caught=1;}",&mut Vec::new()).unwrap();
        assert_eq!(runtime.get_global("caught"), Some(&Value::Int(1)));
        let bad = runtime.get_callable("bad").unwrap();
        let error = runtime
            .invoke(bad, &[Value::Int(4)], &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.exception_kind(), Some("ArithmeticError"));
        assert!(error.diagnostic().unwrap().source.is_some());
        assert_eq!(runtime.environments.live, 1);
        assert!(runtime.stack.is_empty());
        assert_eq!(runtime.prepared_calls_executed > 0, !disabled);
    }
}

#[test]
fn unsupported_effects_execute_once_and_call_arguments_keep_order() {
    use std::cell::Cell;
    let count = Rc::new(Cell::new(0));
    let calls = count.clone();
    let mut runtime = Runtime::new();
    runtime.force_numeric_calls = true;
    runtime
        .register(
            "next",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Int),
                move |_, _| {
                    calls.set(calls.get() + 1);
                    Ok(Value::Int(calls.get()))
                },
            ),
        )
        .unwrap();
    runtime
        .execute_source(
            "function effect($n){if($n<1)return 0;next();return effect($n-1);}$answer=effect(5);",
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(count.get(), 5);
    assert_eq!(runtime.prepared_calls_executed, 0);
    runtime
        .execute_source(&format!("{FIB} $answer=f(next()-3);"), &mut Vec::new())
        .unwrap();
    assert_eq!(count.get(), 6);
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(2)));
    assert!(runtime.prepared_calls_executed > 0);
}

#[test]
fn cancellation_inside_prepared_calls_unwinds_and_allows_recovery() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    runtime.force_numeric_calls = true;
    runtime.execute_source(FIB, &mut Vec::new()).unwrap();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        flag.store(true, Ordering::Relaxed);
    });
    let error = runtime
        .execute_source("$answer=f(28);", &mut Vec::new())
        .unwrap_err();
    worker.join().unwrap();
    assert!(error.message.contains("interrupted"));
    assert!(runtime.prepared_calls_executed > 0);
    assert_eq!(runtime.environments.live, 1);
    assert!(runtime.stack.is_empty());
    runtime
        .options
        .interrupt_flag
        .as_ref()
        .unwrap()
        .store(false, Ordering::Relaxed);
    runtime
        .execute_source("$answer=f(5);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(5)));
}

#[test]
fn suspended_ast_resource_survives_collection_during_prepared_execution() {
    use std::cell::Cell;
    #[derive(Debug)]
    struct Resource(Rc<Cell<usize>>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl crate::NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("unknown property".into())
        }
    }
    let drops = Rc::new(Cell::new(0));
    let dropped = drops.clone();
    let alive = drops.clone();
    let mut runtime = Runtime::new();
    runtime.force_numeric_calls = true;
    runtime
        .register(
            "resource",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Object("Resource")),
                move |_, _| Ok(Value::NativeObject(Rc::new(Resource(dropped.clone())))),
            ),
        )
        .unwrap();
    runtime
        .register(
            "consume",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Callable], NativeType::Int),
                |_, _| Ok(Value::Int(6)),
            ),
        )
        .unwrap();
    runtime
        .register(
            "alive",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Bool),
                move |_, _| Ok(Value::Bool(alive.get() == 0)),
            ),
        )
        .unwrap();
    // The final argument creates a tracked closure immediately before the pure
    // call. Eventually its first checkpoint collects while outer is suspended.
    runtime.execute_source(&format!("{FIB} function outer(){{$r=resource();for($i=0;$i<600;$i+=1){{f(consume(function(){{}}));if(!alive())throw 'lost caller';}}return $r;}}$held=outer();"),&mut Vec::new()).unwrap();
    assert!(runtime.prepared_calls_executed >= 600);
    assert_eq!(drops.get(), 0);
    runtime
        .execute_source("$held=null;", &mut Vec::new())
        .unwrap();
    runtime.collect_garbage();
    assert_eq!(drops.get(), 1);
    assert_eq!(runtime.environments.live, 1);
    assert!(runtime.stack.is_empty());
}

#[test]
fn base_cases_use_the_ordinary_path_and_recursive_entries_use_the_plan() {
    let mut runtime = Runtime::new();
    runtime.force_numeric_calls = true;
    runtime
        .execute_source(&format!("{FIB} f(0);f(1);f(0);"), &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.prepared_calls_executed, 0);
    runtime
        .execute_source("$answer=f(8);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.prepared_calls_executed, 1);
    let before = runtime.prepared_calls_executed;
    runtime
        .execute_source("f(0);f(1);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.prepared_calls_executed, before);
}

#[test]
fn definition_and_caller_sources_survive_prepared_errors_across_files() {
    use std::{path::PathBuf, sync::Arc};
    let definition = "function f($n){if($n===0)return 1/0;return f($n-1);}";
    let call = "$answer=7;$answer=f(4);";
    let parse = |s: &str| scr_parser::parse(&scr_lexer::lex(s).unwrap()).unwrap();
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.force_numeric_calls = true;
        runtime.disable_prepared_calls = disabled;
        for (path, text) in [("definition.scripta", definition), ("caller.scripta", call)] {
            let source = Arc::new(crate::SourceFile {
                path: PathBuf::from(path),
                text: text.into(),
            });
            let result = runtime.execute_program_with_source(&parse(text), source, &mut Vec::new());
            if text == call {
                errors.push(result.unwrap_err());
            } else {
                result.unwrap();
            }
        }
        assert_eq!(runtime.prepared_calls_executed > 0, !disabled);
        assert_eq!(runtime.get_global("answer"), Some(&Value::Int(7)));
    }
    assert_eq!(errors[0], errors[1]);
    assert_eq!(
        errors[0].source.as_ref().unwrap().path,
        PathBuf::from("definition.scripta")
    );
    assert_eq!(
        errors[0].trace[1].source.as_ref().unwrap().path,
        PathBuf::from("caller.scripta")
    );
}

#[test]
fn bounded_plans_reject_oversized_parameter_lists_depth_and_node_counts() {
    let deep = format!(
        "function f($n){{if($n<1)return 0;return f({}$n-1{});}}$answer=f(3);",
        "(".repeat(24),
        ")".repeat(24)
    );
    let many = format!(
        "function f($n){{{}if($n<1)return 0;return f($n-1);}}$answer=f(3);",
        "if(false)return 0;".repeat(70)
    );
    let wide = "function f($n,$a,$b,$c,$d,$e,$g,$h,$i){if($n<1)return 0;return f($n-1,$a,$b,$c,$d,$e,$g,$h,$i);}$answer=f(3,0,0,0,0,0,0,0,0);";
    for source in [deep.as_str(), many.as_str(), wide] {
        assert_eq!(
            compare(source, RuntimeOptions::default()).prepared_calls_executed,
            0
        );
    }
}

#[test]
fn scalar_operations_preserve_strict_results_and_failure_spans() {
    for expression in [
        "+f($n-1)",
        "-f($n-1)",
        "f($n-1)/2",
        "f($n-1)%2",
        "f($n-1)<=3",
        "f($n-1)>=3",
        "f($n-1)>3",
        "f($n-1)<3",
        "f($n-1)===3",
        "f($n-1)!==3",
        "!f($n-1)",
    ] {
        let source = format!(
            "function f($n){{if($n===0)return 7;return {expression};}}$answer=f(1);$later=f(2);"
        );
        assert!(
            compare(&source, RuntimeOptions::default()).prepared_calls_executed > 0,
            "{expression}"
        );
    }
    for (base, expression) in [
        ("false", "!f($n-1)"),
        ("false", "f($n-1)||true"),
        ("true", "f($n-1)&&false"),
        ("true", "+f($n-1)"),
        ("null", "f($n-1)+1"),
    ] {
        assert!(
            compare(
                &format!(
                    "function f($n){{if($n===0)return {base};return {expression};}}$answer=f(3);"
                ),
                RuntimeOptions::default()
            )
            .prepared_calls_executed
                > 0
        );
    }
    assert_eq!(
        compare(
            "function f($n){if($n===0)return false;return true||f($n-1);}$answer=f(3);",
            RuntimeOptions::default()
        )
        .prepared_calls_executed,
        0
    );
}
