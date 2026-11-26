use super::*;
use std::cell::RefCell;

fn expr(kind: ExprKind) -> Expr {
    Expr {
        kind,
        span: Span::new(10, 20),
    }
}
fn int(value: i64) -> Expr {
    expr(ExprKind::Literal(Literal::Int(value)))
}
fn text(value: impl Into<String>) -> Expr {
    expr(ExprKind::Literal(Literal::String(value.into())))
}
fn call(name: &str, value: i64) -> Expr {
    expr(ExprKind::Call(
        Box::new(expr(ExprKind::Identifier(name.into()))),
        vec![int(value)],
    ))
}
fn program(rows: Vec<(Expr, Expr)>) -> Program {
    Program {
        statements: vec![Statement {
            kind: StatementKind::Expression(expr(ExprKind::Assignment(
                Box::new(expr(ExprKind::Variable("seen".into()))),
                AssignmentOp::Assign,
                Box::new(expr(ExprKind::Map(rows))),
            ))),
            span: Span::new(0, 20),
        }],
    }
}
fn rows(count: usize) -> Vec<(Expr, Expr)> {
    (0..count)
        .map(|i| (text(format!("key{i}")), int(i as i64)))
        .collect()
}
fn paired() -> (Runtime, Runtime) {
    let (fast, mut slow) = crate::test_support::runtimes(RuntimeOptions::default());
    slow.disable_map_literal_index = true;
    (fast, slow)
}
fn compare(
    fast: &mut Runtime,
    slow: &mut Runtime,
    program: &Program,
) -> std::result::Result<u8, RuntimeError> {
    crate::test_support::compare_execution(fast, slow, "map literal program", |runtime, output| {
        runtime.execute(program, output)
    })
}
#[test]
fn literal_indices_keep_typed_keys_order_copies_duplicates_and_allocation_fallback() {
    for count in [8, 63, 64, 95, 96, 120] {
        for failure in [None, Some(0), Some(24)] {
            let (mut fast, mut slow) = paired();
            fast.fail_map_literal_index_after = failure;
            let mut input = rows(count);
            input[0] = (int(0), int(11));
            input[1] = (text("0"), int(22));
            input[2] = (text("é🙂"), expr(ExprKind::List(vec![int(2)])));
            for (i, entry) in input.iter_mut().enumerate().skip(8) {
                if i % 7 == 0 {
                    entry.0 = text("key7");
                }
            }
            let p = program(input);
            compare(&mut fast, &mut slow, &p).unwrap();
            let Some(Value::Map(a)) = fast.get_global("seen") else {
                panic!()
            };
            let Some(Value::Map(b)) = slow.get_global("seen") else {
                panic!()
            };
            assert_eq!(a, b);
            assert_eq!(a[0], (MapKey::Int(0), Value::Int(11)));
            assert_eq!(a[1], (MapKey::String("0".into()), Value::Int(22)));
            for runtime in [&mut fast, &mut slow] {
                runtime
                    .execute_source("$copy=$seen;$copy['é🙂'][0]=900;", &mut Vec::new())
                    .unwrap();
            }
            assert_eq!(fast.get_global("copy"), slow.get_global("copy"));
            let Some(Value::Map(a)) = fast.get_global("seen") else {
                panic!()
            };
            assert_eq!(a[2].1, Value::list(vec![Value::Int(2)]));
        }
    }
}

#[derive(Debug)]
struct Probe;
impl NativeObject for Probe {
    fn type_name(&self) -> &str {
        "Probe"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("unknown".into())
    }
}
#[test]
fn borrowed_key_positions_never_add_value_owners_during_reentrant_gc() {
    let (mut fast, mut slow) = paired();
    let mut events = Vec::new();
    let mut weak = Vec::new();
    for runtime in [&mut fast, &mut slow] {
        let resource: Rc<dyn NativeObject> = Rc::new(Probe);
        weak.push(Rc::downgrade(&resource));
        let counts = Rc::new(RefCell::new(Vec::new()));
        events.push(counts.clone());
        runtime
            .register(
                "probe",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![NativeType::Int], NativeType::Any),
                    move |ctx, _| {
                        ctx.runtime.collect_garbage();
                        counts.borrow_mut().push(Rc::strong_count(&resource));
                        Ok(Value::NativeObject(resource.clone()))
                    },
                ),
            )
            .unwrap();
    }
    let p = program(
        (0..128)
            .map(|i| (text(format!("key{}", i % 64)), call("probe", i)))
            .collect(),
    );
    compare(&mut fast, &mut slow, &p).unwrap();
    assert_eq!(*events[0].borrow(), *events[1].borrow());
    assert_eq!(events[0].borrow()[64], 65);
    for runtime in [&mut fast, &mut slow] {
        runtime.set_global("seen", Value::Null).unwrap();
        runtime.environments.scope_mut(0).functions.remove("probe");
        runtime.collect_garbage();
    }
    assert!(weak.iter().all(|value| value.upgrade().is_none()));
}

#[test]
fn pending_closures_survive_gc_and_release_after_success_or_scope_failure() {
    for limit in [None, Some(12)] {
        let (mut fast, mut slow) = paired();
        for runtime in [&mut fast, &mut slow] {
            runtime
                .register(
                    "gc",
                    NativeFunction::reentrant(
                        NativeSignature::new(vec![], NativeType::Null),
                        |ctx, _| {
                            ctx.runtime.collect_garbage();
                            Ok(Value::Null)
                        },
                    ),
                )
                .unwrap();
            runtime
                .execute_source(
                    "function make($x){gc();return function(){return $x;};}$seen='kept';",
                    &mut Vec::new(),
                )
                .unwrap();
            runtime.options.max_scopes = limit;
        }
        let p = program(
            (0..96)
                .map(|i| (text(format!("key{i}")), call("make", i)))
                .collect(),
        );
        let result = compare(&mut fast, &mut slow, &p);
        if limit.is_none() {
            result.unwrap();
            for runtime in [&mut fast, &mut slow] {
                runtime
                    .execute_source(
                        "$first=$seen['key0']();$last=$seen['key95']();",
                        &mut Vec::new(),
                    )
                    .unwrap();
                assert_eq!(runtime.get_global("first"), Some(&Value::Int(0)));
                assert_eq!(runtime.get_global("last"), Some(&Value::Int(95)));
            }
        } else {
            assert!(result.is_err());
            assert_eq!(fast.get_global("seen"), Some(&Value::String("kept".into())));
        }
        for runtime in [&mut fast, &mut slow] {
            runtime.options.max_scopes = None;
            runtime.set_global("seen", Value::Null).unwrap();
            runtime.collect_garbage();
            assert_eq!(runtime.environments.live, 1);
        }
    }
}

#[test]
fn literal_limits_and_final_only_validation_preserve_destinations() {
    let p = program(rows(96));
    let (mut sample, mut legacy) = paired();
    compare(&mut sample, &mut legacy, &p).unwrap();
    let exact = sample.get_global("seen").unwrap().measure(0, None).unwrap();
    for bytes in [exact - 1, exact, exact + 1] {
        let (mut fast, mut slow) = paired();
        for runtime in [&mut fast, &mut slow] {
            runtime
                .set_global("seen", Value::String("kept".into()))
                .unwrap();
            runtime.options.max_value_bytes = Some(bytes);
        }
        let result = compare(&mut fast, &mut slow, &p);
        assert_eq!(result.is_ok(), bytes >= exact);
        assert_eq!(fast.get_global("seen"), slow.get_global("seen"));
    }
    let mut shrinking: Vec<_> = (0..96)
        .map(|i| (text(format!("key{i}")), text("payload".repeat(10))))
        .collect();
    shrinking.extend(rows(96));
    let p = program(shrinking);
    let (mut fast, mut slow) = paired();
    fast.options.max_value_bytes = Some(exact);
    slow.options.max_value_bytes = Some(exact);
    compare(&mut fast, &mut slow, &p).unwrap();
    assert_eq!(fast.get_global("seen"), sample.get_global("seen"));
    for steps in 0..8 {
        let (mut fast, mut slow) = paired();
        fast.options.instruction_limit = Some(steps);
        slow.options.instruction_limit = Some(steps);
        let _ = compare(&mut fast, &mut slow, &p);
    }
}

#[test]
fn key_errors_and_mid_literal_cancellation_keep_effect_order_and_recover() {
    use std::sync::atomic::Ordering;
    for cancel in [false, true] {
        let (mut fast, mut slow) = paired();
        let mut flags = Vec::new();
        let mut events = Vec::new();
        for runtime in [&mut fast, &mut slow] {
            let flag = Arc::new(AtomicBool::new(false));
            flags.push(flag.clone());
            runtime.options.interrupt_flag = Some(flag.clone());
            let log = Rc::new(RefCell::new(Vec::new()));
            events.push(log.clone());
            runtime
                .register(
                    "record",
                    NativeFunction::new(
                        NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                        move |_, args| {
                            log.borrow_mut().push(args[0].clone());
                            if args[0] == Value::Int(80) && cancel {
                                flag.store(true, Ordering::Relaxed);
                            }
                            Ok(args[0].clone())
                        },
                    ),
                )
                .unwrap();
            runtime
                .set_global("seen", Value::String("kept".into()))
                .unwrap();
        }
        let mut input: Vec<_> = (0..96)
            .map(|i| (text(format!("key{i}")), call("record", i)))
            .collect();
        if !cancel {
            input[80].0 = expr(ExprKind::Literal(Literal::Bool(true)));
        }
        let result = compare(&mut fast, &mut slow, &program(input));
        assert!(result.is_err());
        assert_eq!(*events[0].borrow(), *events[1].borrow());
        assert_eq!(fast.get_global("seen"), Some(&Value::String("kept".into())));
        for flag in flags {
            flag.store(false, Ordering::Relaxed);
        }
        for runtime in [&mut fast, &mut slow] {
            runtime
                .execute_source("$recovered=7;", &mut Vec::new())
                .unwrap();
            assert_eq!(runtime.get_global("recovered"), Some(&Value::Int(7)));
        }
    }
}
