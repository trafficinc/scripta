use super::*;
use crate::test_support;

fn base() -> Expr {
    Expr {
        kind: ExprKind::Variable("c".into()),
        span: Span::new(1, 3),
    }
}

#[test]
fn prepared_loop_calls_match_normal_dispatch_with_slots_hashes_and_failures() {
    for hashed in [false, true] {
        for body in [
            "$total+=$c->add($i%7);",
            "$total+=1;$total+=($c)->add(($i+$total)%7);",
            "$total+=$c->add(10/(125-$i));",
            "$total+=$c->add($i)+$alias->add(1);",
        ] {
            let (mut a, mut b) = test_support::runtimes(RuntimeOptions::default());
            b.disable_prepared_method_calls = true;
            test_support::compare_source(&mut a,&mut b,"class C{$x=0;function add($v:int):int{$this->x+=$v;return $this->x;}}$c=new C();$alias=$c;$total=0;").unwrap();
            if hashed {
                a.environments.force_hash_bindings();
                b.environments.force_hash_bindings();
            }
            let source = format!("for($i=0;$i<128;$i+=1){{{body}}}");
            let _ = test_support::compare_with_recovery(
                &mut a,
                &mut b,
                &source,
                "$field=$c->x;$recovered=1;",
            );
            assert_eq!(a.method_call_loops_executed, 1);
            assert_eq!(b.method_call_loops_executed, 1);
            assert!(a.prepared_method_calls_executed > 0);
            assert_eq!(b.prepared_method_calls_executed, 0);
        }
    }
    let mut runtime = Runtime::new();
    runtime.execute_source("class C{function one():int{return 1;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->one();}",&mut Vec::new()).unwrap();
    assert_eq!(
        runtime.prepared_method_calls_executed, 127,
        "first call retains normal dispatch"
    );
}

#[test]
fn prepared_dispatch_retains_base_function_owners_and_lexical_roots_during_arguments() {
    for fail in [false, true] {
        let mut histories = Vec::new();
        let mut steps = Vec::new();
        let mut errors = Vec::new();
        for prepared in [false, true] {
            let mut runtime = Runtime::new();
            let previous = runtime.environments.enter(0);
            runtime
                .execute_source(
                    "class C{function read($v:int):int{return $v;}}$c=new C();",
                    &mut Vec::new(),
                )
                .unwrap();
            let environment = runtime.environments.current;
            let object = runtime
                .environments
                .scope_mut(environment)
                .variables
                .remove("c")
                .unwrap();
            runtime.environments.leave(previous);
            runtime.set_global("c", object).unwrap();
            let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
                panic!("object")
            };
            let receiver = Rc::downgrade(object);
            let events = Rc::new(RefCell::new(Vec::new()));
            let history = events.clone();
            runtime
                .register(
                    "prune",
                    NativeFunction::reentrant(
                        NativeSignature::new(vec![], NativeType::Int),
                        move |context, _| {
                            history.borrow_mut().push((
                                receiver.strong_count(),
                                receiver.weak_count(),
                                context.runtime.environments.temporary_roots.clone(),
                            ));
                            context
                                .runtime
                                .environments
                                .scope_mut(0)
                                .variables
                                .remove("c");
                            context.runtime.collect_garbage();
                            if fail {
                                Err(context
                                    .error(NativeError::new("ProbeError", "argument failed")))
                            } else {
                                Ok(Value::Int(7))
                            }
                        },
                    ),
                )
                .unwrap();
            let base = base();
            let metadata = runtime.prepare_loop_method(&base, "read").unwrap();
            metadata.warm.set(true);
            let arguments = [Expr {
                kind: ExprKind::Call(
                    Box::new(Expr {
                        kind: ExprKind::Identifier("prune".into()),
                        span: Span::new(10, 15),
                    }),
                    vec![],
                ),
                span: Span::new(10, 17),
            }];
            let result = if prepared {
                runtime
                    .call_prepared_loop_method(
                        &metadata,
                        "read",
                        &arguments,
                        Span::new(1, 9),
                        Span::new(1, 18),
                        &mut Vec::new(),
                    )
                    .unwrap()
            } else {
                runtime.call_property(
                    &base,
                    "read",
                    &arguments,
                    Span::new(1, 9),
                    Span::new(1, 18),
                    &mut Vec::new(),
                )
            };
            if fail {
                let Err(Signal::NativeException(_, error)) = result else {
                    panic!("argument error")
                };
                assert!(error.message.contains("ProbeError: argument failed"));
                errors.push(error);
            } else {
                assert!(matches!(result, Ok(Value::Int(7))));
            }
            assert!(runtime.environments.temporary_roots.is_empty());
            assert!(runtime.stack.is_empty());
            assert_eq!(events.borrow()[0], (3, 2, vec![environment]));
            histories.push(events.borrow().clone());
            steps.push(runtime.steps);
            runtime.collect_garbage();
            assert_eq!(runtime.environments.live, 1);
        }
        assert_eq!(histories[0], histories[1]);
        assert_eq!(steps[0], steps[1]);
        if fail {
            assert_eq!(errors[0], errors[1]);
        }
    }
}

#[test]
fn stale_prepared_metadata_falls_back_before_argument_or_proxy_effects() {
    #[derive(Debug)]
    struct Proxy {
        object: Rc<dyn NativeObject>,
        casts: Rc<Cell<usize>>,
    }
    impl NativeObject for Proxy {
        fn type_name(&self) -> &str {
            "Proxy"
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            self.casts.set(self.casts.get() + 1);
            self.object.as_any()
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            panic!("not needed")
        }
    }
    for replacement in ["$c=1;", "$c=new C();", "$c=new D();", "$c=$proxy;"] {
        let mut runtime = Runtime::new();
        runtime.execute_source("class C{function read():int{return 1;}}class D{function read():int{return 2;}}$c=new C();",&mut Vec::new()).unwrap();
        let base = base();
        let metadata = runtime.prepare_loop_method(&base, "read").unwrap();
        metadata.warm.set(true);
        let Value::NativeObject(object) = runtime.get_global("c").unwrap().clone() else {
            panic!("object")
        };
        let casts = Rc::new(Cell::new(0));
        runtime
            .set_global(
                "proxy",
                Value::NativeObject(Rc::new(Proxy {
                    object,
                    casts: casts.clone(),
                })),
            )
            .unwrap();
        runtime
            .execute_source(replacement, &mut Vec::new())
            .unwrap();
        assert!(
            runtime
                .call_prepared_loop_method(
                    &metadata,
                    "read",
                    &[],
                    Span::new(1, 9),
                    Span::new(1, 11),
                    &mut Vec::new()
                )
                .is_none()
        );
        assert_eq!(casts.get(), 0);
        assert_eq!(runtime.prepared_method_calls_executed, 0);
        assert!(runtime.environments.temporary_roots.is_empty());
    }
}

#[test]
fn prepared_metadata_does_not_keep_receivers_or_classes_alive() {
    let mut runtime = Runtime::new();
    runtime
        .execute_source(
            "class C{function read():int{return 1;}}$c=new C();",
            &mut Vec::new(),
        )
        .unwrap();
    let base = base();
    let metadata = runtime.prepare_loop_method(&base, "read").unwrap();
    let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
        panic!("object")
    };
    let receiver = Rc::downgrade(object);
    assert_eq!(receiver.strong_count(), 1);
    drop(runtime);
    assert!(receiver.upgrade().is_none());
    assert!(metadata.class.upgrade().is_none());
}
