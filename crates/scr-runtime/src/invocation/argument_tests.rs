use super::*;
use std::cell::{Cell, RefCell};

fn runtime(disabled: bool) -> Runtime {
    let mut runtime = Runtime::new();
    runtime.disable_argument_validation_reuse = disabled;
    runtime
        .register(
            "Host::repeat",
            NativeFunction::reentrant(
                NativeSignature::new(
                    vec![NativeType::Callable, NativeType::Any],
                    NativeType::List,
                ),
                |context, args| {
                    let argument = InvocationArgument::new(&args[1]);
                    let mut out = Vec::new();
                    for _ in 0..4 {
                        out.push(context.invoke_arguments(args[0].clone(), [&argument])?);
                    }
                    Ok(Value::list(out))
                },
            ),
        )
        .unwrap();
    runtime
}

#[test]
fn immutable_record_checks_are_reused_without_sharing_callback_writes() {
    let mut fast = runtime(false);
    let mut reference = runtime(true);
    let source = r#"
        $input=['rank'=>3,'payload'=>['世界',1.25]];
        function f($row){$row['rank']+=1;return $row['rank'];}
        $seen=Host::repeat(f,$input);
    "#;
    crate::test_support::compare_source(&mut fast, &mut reference, source).unwrap();
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::list(vec![Value::Int(4); 4]))
    );
    assert_eq!(fast.argument_validations_reused, 3);
    assert_eq!(reference.argument_validations_reused, 0);
}

#[test]
fn reused_arguments_keep_exact_limits_diagnostics_and_recovery() {
    let mut options = Vec::new();
    for limit in 0..80 {
        options.push(RuntimeOptions {
            instruction_limit: Some(limit),
            ..Default::default()
        });
    }
    for limit in 0..8 {
        options.push(RuntimeOptions {
            max_scopes: Some(limit),
            ..Default::default()
        });
    }
    for depth in 0..5 {
        options.push(RuntimeOptions {
            recursion_limit: depth,
            ..Default::default()
        });
    }
    let input = Value::map(vec![(MapKey::String("rank".into()), Value::Int(3))]);
    let exact = input.measure(0, None).unwrap();
    for bytes in [0, exact - 1, exact, exact + 1, 1024] {
        options.push(RuntimeOptions {
            max_value_bytes: Some(bytes),
            ..Default::default()
        });
    }
    for option in options {
        let quota = option.max_value_bytes.is_some();
        let mut fast = runtime(false);
        let mut reference = runtime(true);
        for runtime in [&mut fast, &mut reference] {
            runtime
                .execute_source("function f($row){return $row['rank']+1;}", &mut Vec::new())
                .unwrap();
            runtime.set_global("input", input.clone()).unwrap();
            runtime.options = option.clone();
        }
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            "$seen=42;$seen=Host::repeat(f,$input);",
            "$recovered=7;",
        );
        if quota {
            assert_eq!(fast.argument_validations_reused, 0);
        }
    }
    for body in [
        "throw 17;",
        "return 1/0;",
        "exit 7;",
        "return Host::repeat(function($x){return $x;},$row);",
    ] {
        let mut fast = runtime(false);
        let mut reference = runtime(true);
        let source =
            format!("function f($row){{{body}}}$seen=42;$seen=Host::repeat(f,['rank'=>3]);");
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            &source,
            "$recovered=7;",
        );
    }
}

#[derive(Debug)]
struct Payload {
    size: Rc<Cell<usize>>,
    visits: Rc<RefCell<Vec<usize>>>,
    identity: std::rc::Weak<Payload>,
}
impl NativeObject for Payload {
    fn type_name(&self) -> &str {
        "Payload"
    }
    fn payload_bytes(&self) -> usize {
        self.visits.borrow_mut().push(self.identity.strong_count());
        self.size.get()
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("no property".into())
    }
}

#[test]
fn opaque_payloads_are_rechecked_with_original_hook_order_and_alias_counts() {
    for grow in [false, true] {
        let mut runtimes = Vec::new();
        let mut observed = Vec::new();
        let mut counters = Vec::new();
        for disabled in [false, true] {
            let mut runtime = runtime(disabled);
            let size = Rc::new(Cell::new(0));
            let visits = Rc::new(RefCell::new(Vec::new()));
            let object = Rc::new_cyclic(|identity| Payload {
                size: size.clone(),
                visits: visits.clone(),
                identity: identity.clone(),
            });
            runtime
                .set_global(
                    "input",
                    Value::map(vec![
                        (MapKey::String("rank".into()), Value::Int(3)),
                        (MapKey::String("object".into()), Value::NativeObject(object)),
                    ]),
                )
                .unwrap();
            let calls = Rc::new(Cell::new(0));
            let counted = calls.clone();
            runtime
                .register(
                    "grow",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Int),
                        move |_, _| {
                            counted.set(counted.get() + 1);
                            if grow {
                                size.set(usize::MAX);
                            }
                            Ok(Value::Int(1))
                        },
                    ),
                )
                .unwrap();
            observed.push(visits);
            counters.push(calls);
            runtimes.push(runtime);
        }
        let (a, b) = runtimes.split_at_mut(1);
        let result = crate::test_support::compare_with_recovery(
            &mut a[0],
            &mut b[0],
            "function f($row){grow();return $row['rank'];}$seen=42;$seen=Host::repeat(f,$input);",
            "$recovered=7;",
        );
        assert_eq!(result.is_err(), grow);
        assert_eq!(a[0].argument_validations_reused, 0);
        assert_eq!(*observed[0].borrow(), *observed[1].borrow());
        assert_eq!(counters[0].get(), if grow { 1 } else { 4 });
        assert_eq!(counters[0].get(), counters[1].get());
    }
}

#[test]
fn invalid_arguments_are_not_cached_and_classification_work_is_bounded() {
    let mut deep = Value::Int(1);
    for _ in 0..129 {
        deep = Value::list(vec![deep]);
    }
    let cases = [
        (Value::Float(f64::NAN), false),
        (
            Value::map(vec![
                (MapKey::Int(1), Value::Null),
                (MapKey::Int(1), Value::Int(2)),
            ]),
            false,
        ),
        (deep, false),
        (
            Value::list(vec![Value::Int(1); MAX_CACHEABLE_ARGUMENT_NODES - 1]),
            true,
        ),
        (
            Value::list(vec![Value::Int(1); MAX_CACHEABLE_ARGUMENT_NODES]),
            true,
        ),
        (
            Value::native_function(
                "arg",
                NativeFunction::new(NativeSignature::new(vec![], NativeType::Int), |_, _| {
                    Ok(Value::Int(1))
                }),
            ),
            true,
        ),
    ];
    for (value, valid) in cases {
        let mut runtime = Runtime::new();
        let mut fuel = MAX_CACHEABLE_ARGUMENT_NODES;
        let eligible = valid && immutable_argument(&value, &mut fuel);
        let callee = Value::native_function(
            "accept",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Any], NativeType::Int),
                |_, _| Ok(Value::Int(7)),
            ),
        );
        runtime
            .register(
                "Host::local",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |context, _| {
                        let argument = InvocationArgument::new(&value);
                        let first = context.invoke_arguments(callee.clone(), [&argument]);
                        assert_eq!(first.is_ok(), valid);
                        let second = context.invoke_arguments(callee.clone(), [&argument]);
                        assert_eq!(second.is_ok(), valid);
                        if !valid {
                            assert!(matches!(argument.state.get(), ArgumentState::Unknown));
                        }
                        second
                    },
                ),
            )
            .unwrap();
        assert_eq!(
            runtime
                .execute_source("Host::local();", &mut Vec::new())
                .is_ok(),
            valid
        );
        assert_eq!(runtime.argument_validations_reused, usize::from(eligible));
    }
}

#[test]
fn a_later_byte_quota_cannot_reuse_an_unbudgeted_success() {
    for disabled in [false, true] {
        let mut runtime = runtime(disabled);
        let value = Value::map(vec![(MapKey::String("rank".into()), Value::Int(3))]);
        let limit = value.measure(0, None).unwrap() - 1;
        let callee = Value::native_function(
            "accept",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Any], NativeType::Int),
                |_, _| Ok(Value::Int(1)),
            ),
        );
        runtime
            .register(
                "Host::quota",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |context, _| {
                        let argument = InvocationArgument::new(&value);
                        context.invoke_arguments(callee.clone(), [&argument])?;
                        context.runtime.options.max_value_bytes = Some(limit);
                        let error = context
                            .invoke_arguments(callee.clone(), [&argument])
                            .unwrap_err();
                        assert!(
                            error
                                .diagnostic()
                                .unwrap()
                                .message
                                .contains("value byte limit")
                        );
                        context.runtime.options.max_value_bytes = None;
                        Err(error)
                    },
                ),
            )
            .unwrap();
        assert!(
            runtime
                .execute_source("Host::quota();", &mut Vec::new())
                .is_err()
        );
        assert_eq!(runtime.argument_validations_reused, 0);
    }
}
