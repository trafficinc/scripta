use super::*;
use crate::{RuntimeOptions, test_support};

fn pair(options: RuntimeOptions) -> (Runtime, Runtime) {
    let (fast, mut old) = test_support::runtimes(options);
    old.disable_method_call_loops = true;
    (fast, old)
}

type FieldState = (Vec<(String, Option<Value>)>, usize);

fn fields(runtime: &Runtime) -> Option<FieldState> {
    let Value::NativeObject(object) = runtime.get_global("c")? else {
        return None;
    };
    let instance = object
        .as_any()?
        .downcast_ref::<crate::objects::ScriptObject>()?;
    Some((
        instance
            .class
            .declaration
            .fields
            .iter()
            .map(|field| {
                (
                    field.binding.name.clone(),
                    runtime
                        .environments
                        .scope(instance.scope)
                        .variables
                        .get(&field.binding.name)
                        .map(|value| match value {
                            Value::NativeObject(_) | Value::Callable(_) => {
                                Value::String(value.type_name().into())
                            }
                            _ => value.clone(),
                        }),
                )
            })
            .collect(),
        object.payload_bytes(),
    ))
}

#[test]
fn counters_getters_parameters_and_aliases_match() {
    for source in [
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}$field=$c->x;",
        "class C{$x=7;function read():int{return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->read();}",
        "class C{$x=0;function add($v:int):int{$this->x+=$v;return $this->x;}}$c=new C();$alias=$c;$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$alias->add(($i+$total)%7);}$field=$c->x;",
        "class C{$x=0;$y=1;function add($v:int):int{$this->x+=$v;$this->y+=1;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total=$c->add($i);}$field=$c->x;",
        "class C{$x=0;function one():int{$this->x+=1;return $this->x;}function two():int{$this->x+=2;return $this->x;}}$c=new C();$alias=$c;$total=0;for($i=0;$i<128;$i+=1){$total+=$c->one()+$alias->two();}$field=$c->x;",
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}function run($c){$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}return $total;}$seen=run(new C());",
    ] {
        let (mut a, mut b) = pair(RuntimeOptions::default());
        test_support::compare_source(&mut a, &mut b, source).unwrap();
        assert_eq!(a.method_call_loops_executed, 1, "{source}");
        assert_eq!(b.method_call_loops_executed, 0);
        assert_eq!(fields(&a), fields(&b));
    }
    for (count, optimized) in [(0, false), (127, false), (128, true), (129, true)] {
        let source = format!(
            "class C{{$x=0;function next():int{{$this->x+=1;return $this->x;}}}}$c=new C();$total=0;for($i=0;$i<{count};$i+=1){{$total+=$c->next();}}$field=$c->x;"
        );
        let (mut a, mut b) = pair(RuntimeOptions::default());
        b.disable_numeric_call_loops = true;
        b.disable_integer_loops = true;
        b.disable_method_plans = true;
        test_support::compare_source(&mut a, &mut b, &source).unwrap();
        assert_eq!(a.method_call_loops_executed, usize::from(optimized));
        assert_eq!(
            a.get_global("total"),
            Some(&Value::Int(count * (count + 1) / 2))
        );
        assert_eq!(a.get_global("field"), Some(&Value::Int(count)));
    }
}

#[test]
fn method_argument_return_arithmetic_and_caller_failures_keep_partial_state() {
    for source in [
        "class C{$x=0;function next():int{$this->x+=1;return 10/(125-$this->x);}}$c=new C();$i=0;$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next();}",
        "class C{$x=0;function next():int{$this->x=9;return true;}}$c=new C();$i=0;$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next();}",
        "class C{$x=0;function next($v:bool):int{$this->x=9;return 1;}}$c=new C();$i=0;$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next($i);}",
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$i=0;$total=9223372036854775807;for($i=0;$i<128;$i+=1){$total+=$c->next();}",
        "class C{$x=9223372036854775807;function next():int{$this->x+=1;return $this->x;}}$c=new C();$i=0;$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next();}",
        "class C{$x=0;function next($a:int,$b:int):int{$this->x+=$a+$b;return $this->x;}}$c=new C();$i=0;$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next(10/(2-$i),10/(3-$i));}",
    ] {
        let (mut a, mut b) = pair(RuntimeOptions::default());
        assert!(
            test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;").is_err()
        );
        assert_eq!(a.method_call_loops_executed, 1, "{source}");
        assert_eq!(fields(&a), fields(&b), "{source}");
    }
}

#[test]
fn exact_method_loop_limits_match_warmed_general_execution() {
    let source = "for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next($i%3);}";
    let mut options = Vec::new();
    for scopes in 0..8 {
        for steps in (0..45).chain(760..790) {
            options.push(RuntimeOptions {
                max_scopes: Some(scopes),
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            });
        }
    }
    for recursion_limit in 0..4 {
        options.push(RuntimeOptions {
            recursion_limit,
            ..RuntimeOptions::default()
        });
    }
    for max_value_bytes in [0, 1, 31, 32, 64, 65, 66, 1024] {
        options.push(RuntimeOptions {
            max_value_bytes: Some(max_value_bytes),
            ..RuntimeOptions::default()
        });
    }
    for options in options {
        let (mut a, mut b) = pair(RuntimeOptions::default());
        test_support::compare_source(&mut a, &mut b,
            "class C{$x=0;function next($v:int):int{$this->x+=$v;return $this->x;}}$c=new C();$warm=$c->next(0);$i=0;$total=0;").unwrap();
        a.options = options.clone();
        b.options = options;
        let _ = test_support::compare_source(&mut a, &mut b, source);
        assert_eq!(fields(&a), fields(&b));
        if a.options.max_value_bytes.is_some() {
            assert_eq!(a.method_call_loops_executed, 0);
        }
    }
}

#[test]
fn unsupported_receiver_effects_types_rebinding_and_loop_shapes_fall_back() {
    for source in [
        "class C{$x=0;function next(){$this->x+=1;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}",
        "class C{$x=0;function next():bool{$this->x+=1;return true;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total=$c->next();}",
        "class C{$x=0;function next($v=1):int{$this->x+=$v;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}",
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$c=1;$total+=$c->next();}",
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$total=0;$n=128;for($i=0;$i<$n;$i+=1){$total+=$c->next();$n-=1;}",
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=2){$total+=$c->next();}",
        "class C{$x=0;function next($v:int):int{$this->x+=$v;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next($c->next(1));}",
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}function f($v){return $v+1;}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=f($i)+$c->next();}",
        "class C{function next():int{return $missing%7;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}",
        "class D{}class C{$x=0;$opaque=new D();function next():int{$this->x+=1;return $this->x;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}",
    ] {
        let (mut a, mut b) = pair(RuntimeOptions::default());
        let _ = test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;");
        assert_eq!(a.method_call_loops_executed, 0, "{source}");
        assert_eq!(fields(&a), fields(&b));
    }
}

#[test]
fn call_slot_and_planning_bounds_keep_general_fallbacks() {
    for count in [8, 9] {
        let expr = vec!["$c->read()"; count].join("+");
        let source = format!(
            "class C{{$x=7;function read():int{{return $this->x;}}}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){{$total+={expr};}}"
        );
        let (mut a, mut b) = pair(RuntimeOptions::default());
        test_support::compare_source(&mut a, &mut b, &source).unwrap();
        assert_eq!(a.method_call_loops_executed, usize::from(count == 8));
    }
    for count in [15, 16] {
        let vars = (0..count).map(|i| format!("$v{i}=0;")).collect::<String>();
        let body = (0..count)
            .map(|i| {
                if i == 0 {
                    "$v0+=$c->read();".to_owned()
                } else {
                    format!("$v{i}+=$i;")
                }
            })
            .collect::<String>();
        let source = format!(
            "class C{{$x=7;function read():int{{return $this->x;}}}}$c=new C();{vars}for($i=0;$i<128;$i+=1){{{body}}}"
        );
        let (mut a, mut b) = pair(RuntimeOptions::default());
        test_support::compare_source(&mut a, &mut b, &source).unwrap();
        assert_eq!(a.method_call_loops_executed, usize::from(count == 15));
    }
}

#[test]
fn native_bridges_and_proxy_receivers_keep_live_values_and_observable_downcasts() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::cell::Cell;
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    let mut runtime = Runtime::new();
    runtime
        .register(
            "observe",
            NativeFunction::reentrant(
                NativeSignature::new(vec![], NativeType::Int),
                move |ctx, _| {
                    assert_eq!(
                        ctx.runtime.get_global("total"),
                        Some(&Value::Int(seen.get() * 2 + 1))
                    );
                    seen.set(seen.get() + 1);
                    Ok(Value::Int(1))
                },
            ),
        )
        .unwrap();
    runtime.execute_source("class C{$x=0;function next():int{return observe();}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=1;$total+=$c->next();}",&mut Vec::new()).unwrap();
    assert_eq!(calls.get(), 128);
    assert_eq!(runtime.method_call_loops_executed, 0);
    #[derive(Debug)]
    struct Proxy {
        object: Rc<dyn NativeObject>,
        casts: Rc<Cell<usize>>,
    }
    impl NativeObject for Proxy {
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            self.casts.set(self.casts.get() + 1);
            self.object.as_any()
        }
        fn type_name(&self) -> &str {
            "Proxy"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            panic!("script downcast")
        }
    }
    let (mut a, mut b) = pair(RuntimeOptions::default());
    test_support::compare_source(
        &mut a,
        &mut b,
        "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$base=new C();",
    )
    .unwrap();
    let mut casts = Vec::new();
    for runtime in [&mut a, &mut b] {
        let Value::NativeObject(object) = runtime.get_global("base").unwrap().clone() else {
            panic!("object")
        };
        let count = Rc::new(Cell::new(0));
        runtime
            .set_global(
                "c",
                Value::NativeObject(Rc::new(Proxy {
                    object,
                    casts: count.clone(),
                })),
            )
            .unwrap();
        casts.push(count);
    }
    test_support::compare_source(
        &mut a,
        &mut b,
        "$total=0;for($i=0;$i<128;$i+=1){$total+=$c->next();}$field=$c->x;",
    )
    .unwrap();
    assert_eq!(a.method_call_loops_executed, 0);
    assert_eq!(casts[0].get(), casts[1].get());
    assert!(casts[0].get() > 0);
}

#[test]
fn collector_observes_no_extra_receiver_owners_between_calls() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::{
        cell::{Cell, RefCell},
        rc::Weak,
    };
    #[derive(Debug)]
    struct Observer {
        receiver: Weak<dyn NativeObject>,
        begun: Rc<Cell<bool>>,
        events: Rc<RefCell<Vec<usize>>>,
    }
    impl Drop for Observer {
        fn drop(&mut self) {
            assert!(self.begun.get(), "observer dropped before loop planning");
            self.events.borrow_mut().push(self.receiver.strong_count());
        }
    }
    impl NativeObject for Observer {
        fn type_name(&self) -> &str {
            "Observer"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("none".into())
        }
    }
    let mut histories = Vec::new();
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.disable_method_call_loops = disabled;
        runtime
            .execute_source(
                "class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();",
                &mut Vec::new(),
            )
            .unwrap();
        let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
            panic!("object")
        };
        let receiver = Rc::downgrade(object);
        let begun = Rc::new(Cell::new(false));
        let events = Rc::new(RefCell::new(Vec::new()));
        let start = begun.clone();
        let log = events.clone();
        runtime
            .register(
                "probe",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| {
                        Ok(Value::NativeObject(Rc::new(Observer {
                            receiver: receiver.clone(),
                            begun: start.clone(),
                            events: log.clone(),
                        })))
                    },
                ),
            )
            .unwrap();
        runtime
            .register(
                "arm",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![], NativeType::Int),
                    move |ctx, _| {
                        begun.set(true);
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
        runtime.execute_source("function dead(){$r=probe();function nested(){return 1;}}dead();$i=0;$total=0;for($i=arm();$i<128;$i+=1){$total+=$c->next();}",&mut Vec::new()).unwrap();
        assert_eq!(runtime.get_global("total"), Some(&Value::Int(8256)));
        assert_eq!(runtime.method_call_loops_executed, usize::from(!disabled));
        histories.push(events.borrow().clone());
    }
    assert!(!histories[0].is_empty());
    assert_eq!(histories[0], histories[1]);
}

#[test]
fn captured_class_resources_survive_collection_and_release_after_receiver_drop() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::cell::Cell;
    #[derive(Debug)]
    struct Resource {
        finished: Rc<Cell<bool>>,
        drops: Rc<Cell<usize>>,
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            assert!(self.finished.get());
            self.drops.set(self.drops.get() + 1);
        }
    }
    impl NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("none".into())
        }
    }
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.disable_method_call_loops = disabled;
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
                        Ok(Value::NativeObject(Rc::new(Resource {
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
                "finish",
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
        // Model a loaded module: classes are top-level declarations in a
        // retained non-global lexical environment, never nested declarations.
        let previous = runtime.environments.enter(0);
        let module = runtime.environments.current;
        runtime.execute_source("$resource=probe();class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();", &mut Vec::new()).unwrap();
        let object = runtime
            .environments
            .scope(module)
            .variables
            .get("c")
            .unwrap()
            .clone();
        runtime.set_global("c", object).unwrap();
        runtime.environments.leave(previous);
        runtime
            .execute_source(
                "$total=0;for($i=arm();$i<128;$i+=1){$total+=$c->next();}finish();$c=null;",
                &mut Vec::new(),
            )
            .unwrap();
        runtime.collect_garbage();
        assert_eq!(drops.get(), 1);
        assert_eq!(runtime.method_call_loops_executed, usize::from(!disabled));
    }
}

#[test]
fn asynchronous_cancellation_publishes_totals_and_keeps_method_partial_writes() {
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
        runtime.disable_method_call_loops = disabled;
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        let error=runtime.execute_source("class C{$x=0;function next():int{$this->x+=1;return $this->x;}}$c=new C();$i=0;$total=0;for($i=0;$i<9223372036854775807;$i+=1){$total+=$c->next();}",&mut Vec::new()).unwrap_err();
        worker.join().unwrap();
        assert!(error.message.contains("execution interrupted"));
        assert!(runtime.stack.is_empty());
        assert_eq!(runtime.environments.live, 2);
        let Some(Value::Int(i)) = runtime.get_global("i") else {
            panic!("counter")
        };
        let Some(Value::Int(total)) = runtime.get_global("total") else {
            panic!("total")
        };
        assert_eq!(
            i128::from(*total),
            i128::from(*i) * (i128::from(*i) + 1) / 2
        );
        let Some((values, _)) = fields(&runtime) else {
            panic!("fields")
        };
        let Some(Value::Int(x)) = &values[0].1 else {
            panic!("field")
        };
        assert!(*x == *i || *x == *i + 1);
        assert_eq!(runtime.method_call_loops_executed, usize::from(!disabled));
    }
}

#[test]
fn module_methods_observe_live_caller_bindings_when_the_proof_rejects_captures() {
    for (body, expected) in [
        ("return $total%7;", (128, 2)),
        ("$total+=1;return 1;", (256, 1)),
    ] {
        let (mut a, mut b) = pair(RuntimeOptions::default());
        for runtime in [&mut a, &mut b] {
            let previous = runtime.environments.enter(0);
            let module = runtime.environments.current;
            runtime
                .execute_source(
                    &format!("$total=0;$out=0;class C{{function next():int{{{body}}}}}$c=new C();"),
                    &mut Vec::new(),
                )
                .unwrap();
            let object = runtime
                .environments
                .scope(module)
                .variables
                .get("c")
                .unwrap()
                .clone();
            runtime.set_global("c", object).unwrap();
            runtime.environments.leave(previous);
        }
        test_support::compare_execution(
            &mut a,
            &mut b,
            "module caller bindings",
            |runtime, output| {
                let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
                    panic!("object")
                };
                let module = object
                    .as_any()
                    .unwrap()
                    .downcast_ref::<crate::objects::ScriptObject>()
                    .unwrap()
                    .class
                    .environment;
                runtime.environments.current = module;
                let result = runtime
                    .execute_source("for($i=0;$i<128;$i+=1){$total+=1;$out=$c->next();}", output);
                runtime.environments.current = 0;
                let values = &runtime.environments.scope(module).variables;
                let observed = Value::list(vec![
                    values.get("total").unwrap().clone(),
                    values.get("out").unwrap().clone(),
                ]);
                runtime.set_global("observed", observed).unwrap();
                result
            },
        )
        .unwrap();
        assert_eq!(a.method_call_loops_executed, 0);
        assert_eq!(
            a.get_global("observed"),
            Some(&Value::list(vec![
                Value::Int(expected.0),
                Value::Int(expected.1)
            ]))
        );
    }
}
