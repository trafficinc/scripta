use super::*;
use crate::RuntimeOptions;
fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let mut fast = Runtime::with_options(options.clone());
    let mut old = Runtime::with_options(options);
    old.disable_numeric_call_loops = true;
    for text in [source, "$recovered=1;"] {
        let mut a = Vec::new();
        let mut b = Vec::new();
        assert_eq!(
            fast.execute_source(text, &mut a),
            old.execute_source(text, &mut b),
            "{source}"
        );
        assert_eq!(a, b);
        assert_eq!(fast.steps, old.steps);
        assert_eq!(fast.environments.live, old.environments.live);
        assert_eq!(fast.stack.len(), old.stack.len());
        for (name, value) in &fast.environments.scope(0).variables {
            if matches!(value, Value::Callable(_) | Value::NativeObject(_)) {
                continue;
            }
            assert_eq!(Some(value), old.get_global(name), "{name}");
        }
    }
    fast
}
#[test]
fn integer_leaf_calls_aliases_and_multiple_assignments_match() {
    for source in [
        "function f($x:int):int{return($x%97)*3+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function f($x){return$x+1;}function g($x,$y){return$x-$y;}$total=0;for($i=0;$i<130;$i+=1){$total=f($total)+g($i,2);$total+=1;}",
        "function f($x){return-$x;}$total=0;$n=128;for($i=-2;$i<$n;$i+=1){$total-=f($i);}",
        "function make(){function f($x){return$x+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}return$total;}$seen=make();",
    ] {
        assert_eq!(
            compare(source, RuntimeOptions::default()).numeric_call_loops_executed,
            1,
            "{source}"
        );
    }
}
#[test]
fn callee_failures_argument_order_and_partial_writes_match() {
    for source in [
        "function f($x){return 10/(125-$x);}$total=3;for($i=0;$i<128;$i+=1){$total+=1;$total+=f($i);}",
        "function f($x){return$x+9223372036854775807;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function f($x:float):int{return$x;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function f($x:int):float{return$x;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function f($a,$b){return$a+$b;}$total=1;for($i=0;$i<128;$i+=1){$total+=f(1/(2-$i),1/(3-$i));}",
        "function f($x){return 10/(125-$x);}$total=0;try{for($i=0;$i<128;$i+=1){$total+=f($i);}}catch(ArithmeticError $e){$seen=$total;}",
    ] {
        assert_eq!(
            compare(source, RuntimeOptions::default()).numeric_call_loops_executed,
            1,
            "{source}"
        );
    }
}
#[test]
fn exact_instruction_scope_call_and_byte_limits_match() {
    let source =
        "function f($x:int):int{return$x+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}";
    for scopes in 0..7 {
        for steps in 0..65 {
            compare(
                source,
                RuntimeOptions {
                    max_scopes: Some(scopes),
                    instruction_limit: Some(steps),
                    ..RuntimeOptions::default()
                },
            );
        }
    }
    for depth in 0..4 {
        compare(
            source,
            RuntimeOptions {
                recursion_limit: depth,
                ..RuntimeOptions::default()
            },
        );
    }
    for bytes in [0, 1, 32, 1024] {
        assert_eq!(
            compare(
                source,
                RuntimeOptions {
                    max_value_bytes: Some(bytes),
                    ..RuntimeOptions::default()
                }
            )
            .numeric_call_loops_executed,
            0
        );
    }
}
#[test]
fn short_mutating_opaque_captured_and_nonnumeric_calls_fall_back() {
    for source in [
        "function f($x){return$x+1;}$total=0;for($i=0;$i<127;$i+=1){$total+=f($i);}",
        "function f($x){return$x+1;}$total=0;$n=128;for($i=0;$i<$n;$i+=1){$total+=f($i);$n-=1;}",
        "function f($x){return$x+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);$i+=1;}",
        "function f($x){return$x+1;}$f=f;$total=0;for($i=0;$i<128;$i+=1){$total+=$f($i);}",
        "function f($x){return$x+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f(f($i));}",
        "function f($x){return$x+0.5;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function f($x=1){return$x+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function f($x){$x+=1;return$x;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}",
        "function make(){$k=1;function f($x){return$x+$k;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}return$total;}$seen=make();",
    ] {
        assert_eq!(
            compare(source, RuntimeOptions::default()).numeric_call_loops_executed,
            0,
            "{source}"
        );
    }
}

#[test]
fn call_and_slot_planning_limits_fall_back_without_partial_execution() {
    for count in [8, 9] {
        let expr = vec!["f($i)"; count].join("+");
        let source = format!(
            "function f($x){{return$x+1;}}$total=0;for($i=0;$i<128;$i+=1){{$total+={expr};}}"
        );
        assert_eq!(
            compare(&source, RuntimeOptions::default()).numeric_call_loops_executed,
            usize::from(count == 8)
        );
    }
    for count in [15, 16] {
        let bindings = (0..count).map(|i| format!("$v{i}=0;")).collect::<String>();
        let body = (0..count)
            .map(|i| {
                if i == 0 {
                    "$v0+=f($i);".to_string()
                } else {
                    format!("$v{i}+=$i;")
                }
            })
            .collect::<String>();
        let source =
            format!("function f($x){{return$x+1;}}{bindings}for($i=0;$i<128;$i+=1){{{body}}}");
        assert_eq!(
            compare(&source, RuntimeOptions::default()).numeric_call_loops_executed,
            usize::from(count == 15)
        );
    }
    let source = "function f($x){return$x+1;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}";
    for steps in 500..660 {
        compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            },
        );
    }
}

#[test]
fn cancellation_publishes_completed_caller_writes_and_releases_scopes() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for disabled in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime.disable_numeric_call_loops = disabled;
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        let error=runtime.execute_source("function f($x){return$x+1;}$total=0;$i=0;for($i=0;$i<9223372036854775807;$i+=1){$total+=f($i);}",&mut Vec::new()).unwrap_err();
        worker.join().unwrap();
        assert!(error.message.contains("execution interrupted"));
        assert_eq!(runtime.environments.live, 1);
        assert!(runtime.stack.is_empty());
        let Value::Int(i) = runtime.get_global("i").unwrap() else {
            panic!()
        };
        let Value::Int(total) = runtime.get_global("total").unwrap() else {
            panic!()
        };
        assert_eq!(
            i128::from(*total),
            i128::from(*i) * (i128::from(*i) + 1) / 2
        );
        assert_eq!(runtime.numeric_call_loops_executed, usize::from(!disabled));
    }
}

#[test]
fn native_observers_and_user_bridges_always_see_live_caller_writes() {
    use crate::{NativeFunction, NativeSignature, NativeType};
    use std::{cell::Cell, rc::Rc};
    for bridge in [false, true] {
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        let mut runtime = Runtime::new();
        runtime
            .register(
                "observe",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                    move |ctx, _| {
                        assert_eq!(
                            ctx.runtime.get_global("total"),
                            Some(&Value::Int(seen.get()))
                        );
                        seen.set(seen.get() + 1);
                        Ok(Value::Int(1))
                    },
                ),
            )
            .unwrap();
        let (prefix, callee) = if bridge {
            ("function f($x){return observe($x);}", "f")
        } else {
            ("", "observe")
        };
        runtime
            .execute_source(
                &format!("{prefix}$total=0;for($i=0;$i<128;$i+=1){{$total+={callee}($i);}}"),
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(calls.get(), 128);
        assert_eq!(runtime.numeric_call_loops_executed, 0);
    }
}

#[test]
fn collector_keeps_callee_captures_alive_until_the_loop_finishes() {
    use crate::{NativeFunction, NativeSignature, NativeType};
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Probe {
        finished: Rc<Cell<bool>>,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            assert!(self.finished.get(), "captured resource dropped inside loop");
            self.drops.set(self.drops.get() + 1);
        }
    }
    impl crate::NativeObject for Probe {
        fn type_name(&self) -> &str {
            "Probe"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("none".into())
        }
    }
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.disable_numeric_call_loops = disabled;
        let finished = Rc::new(Cell::new(false));
        let drops = Rc::new(Cell::new(0));
        let f = finished.clone();
        let d = drops.clone();
        runtime
            .register(
                "probe",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| {
                        Ok(Value::NativeObject(Rc::new(Probe {
                            finished: f.clone(),
                            drops: d.clone(),
                        })))
                    },
                ),
            )
            .unwrap();
        let f = finished.clone();
        runtime
            .register(
                "done",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Null),
                    move |_, _| {
                        f.set(true);
                        Ok(Value::Null)
                    },
                ),
            )
            .unwrap();
        runtime
            .register(
                "arm",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![], NativeType::Int),
                    |ctx, _| {
                        for _ in 0..512 {
                            let Value::Callable(dead) = Value::native_function(
                                "dead",
                                NativeFunction::new(
                                    NativeSignature::new(vec![], NativeType::Null),
                                    |_, _| Ok(Value::Null),
                                ),
                            ) else {
                                unreachable!()
                            };
                            ctx.runtime.environments.track_function(&dead.0);
                        }
                        Ok(Value::Int(0))
                    },
                ),
            )
            .unwrap();
        runtime.execute_source("function outer(){$r=probe();function f($x){return$x+1;}$total=0;for($i=arm();$i<128;$i+=1){$total+=f($i);}return$total;}$seen=outer();done();",&mut Vec::new()).unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(8256)));
        assert_eq!(drops.get(), 1);
        assert_eq!(runtime.numeric_call_loops_executed, usize::from(!disabled));
    }
}
