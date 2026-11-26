use scr_runtime::{
    ExecutionOutcome, NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value,
};

fn register_bridge(runtime: &mut Runtime) {
    runtime
        .register(
            "Host::invoke",
            NativeFunction::reentrant(
                NativeSignature::new(vec![NativeType::Callable], NativeType::Any),
                |context, args| context.invoke(args[0].clone(), &[]),
            ),
        )
        .unwrap();
}

#[test]
fn reentrant_callbacks_preserve_exception_kinds_values_and_runtime_recovery() {
    let mut runtime = Runtime::new();
    register_bridge(&mut runtime);
    let mut output = Vec::new();
    runtime.execute_source("try { Host::invoke(function () { return 1 / 0; }); } catch (ArithmeticError $e) { echo 'arithmetic'; } try { Host::invoke(function () { throw 7; }); } catch (int $e) { if ($e === 7) echo '7'; }", &mut output).unwrap();
    assert_eq!(output, b"arithmetic7");
    runtime
        .execute_source(
            "function fail() { return 1 + 'x'; } function value() { return 42; }",
            &mut output,
        )
        .unwrap();
    let error = runtime
        .invoke(runtime.get_callable("fail").unwrap(), &[], &mut output)
        .unwrap_err();
    assert_eq!(error.exception_kind(), Some("TypeError"));
    assert!(error.diagnostic().unwrap().source.is_some());
    assert_eq!(
        runtime
            .invoke(runtime.get_callable("value").unwrap(), &[], &mut output)
            .unwrap(),
        Value::Int(42)
    );
}

#[test]
fn exits_are_distinct_from_completion_in_source_and_host_invocations() {
    let mut runtime = Runtime::new();
    let mut output = Vec::new();
    assert_eq!(
        runtime
            .execute_source_outcome("function stop() { exit 0; }", &mut output)
            .unwrap(),
        ExecutionOutcome::Completed
    );
    let error = runtime
        .invoke(runtime.get_callable("stop").unwrap(), &[], &mut output)
        .unwrap_err();
    assert_eq!(error.exit_code(), Some(0));
    assert_eq!(error.exception_kind(), None);
    assert_eq!(
        runtime
            .execute_source_outcome("exit 0;", &mut output)
            .unwrap(),
        ExecutionOutcome::Exited(0)
    );
    assert_eq!(runtime.execute_source("exit 7;", &mut output).unwrap(), 7);
}

#[test]
fn callback_budgets_foreign_owners_and_recursion_remain_enforced() {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(20),
        ..Default::default()
    });
    register_bridge(&mut runtime);
    runtime
        .execute_source(
            "function spin() { Host::invoke(function () { while (true) {} }); }",
            &mut Vec::new(),
        )
        .unwrap();
    let error = runtime
        .invoke(runtime.get_callable("spin").unwrap(), &[], &mut Vec::new())
        .unwrap_err();
    assert!(error.is_interrupted());
    assert_eq!(error.exception_kind(), None);
    let mut other = Runtime::new();
    other
        .execute_source("function f() { return 1; }", &mut Vec::new())
        .unwrap();
    let mut target = Runtime::new();
    let error = target
        .invoke(other.get_callable("f").unwrap(), &[], &mut Vec::new())
        .unwrap_err();
    assert_eq!(error.exception_kind(), None);
    assert!(
        error
            .diagnostic()
            .unwrap()
            .message
            .contains("different runtime")
    );
    let mut limited = Runtime::with_options(RuntimeOptions {
        recursion_limit: 1,
        ..Default::default()
    });
    limited
        .execute_source("function f() { return f(); }", &mut Vec::new())
        .unwrap();
    assert!(
        limited
            .invoke(limited.get_callable("f").unwrap(), &[], &mut Vec::new())
            .unwrap_err()
            .diagnostic()
            .unwrap()
            .message
            .contains("maximum call depth")
    );
}

fn register_owned_bridge(runtime: &mut Runtime) {
    runtime
        .register(
            "Host::owned",
            NativeFunction::reentrant(
                NativeSignature::new(vec![NativeType::Any], NativeType::Any).with_optional(vec![
                    NativeType::Any,
                    NativeType::Any,
                    NativeType::Any,
                ]),
                |context, args| match args.len() {
                    1 => context.invoke_owned(args[0].clone(), []),
                    2 => context.invoke_owned(args[0].clone(), [args[1].clone()]),
                    3 => context.invoke_owned(args[0].clone(), [args[1].clone(), args[2].clone()]),
                    4 => context.invoke_owned(
                        args[0].clone(),
                        [args[1].clone(), args[2].clone(), args[3].clone()],
                    ),
                    _ => unreachable!(),
                },
            ),
        )
        .unwrap();
}

#[test]
fn owned_invocation_moves_opaque_list_storage_and_borrowed_invocation_copies_it() {
    use std::{cell::Cell, rc::Rc};
    for owned in [false, true] {
        let pointer = Rc::new(Cell::new(0));
        let expected = pointer.clone();
        let mut runtime = Runtime::new();
        runtime
            .register(
                "Host::inspect",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::List], NativeType::Int),
                    move |_, args| {
                        let Value::List(items) = &args[0] else {
                            unreachable!()
                        };
                        assert_eq!(items.as_ptr() as usize == expected.get(), owned);
                        assert_eq!(
                            items,
                            &[Value::Bytes(Rc::from(&b"data"[..])), Value::Int(9)]
                        );
                        Ok(Value::Int(42))
                    },
                ),
            )
            .unwrap();
        let callee = runtime.get_callable("Host::inspect").unwrap();
        runtime
            .register(
                "Host::send",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |context, _| {
                        let items = vec![Value::Bytes(Rc::from(&b"data"[..])), Value::Int(9)];
                        pointer.set(items.as_ptr() as usize);
                        if owned {
                            context.invoke_owned(callee.clone(), [Value::list(items)])
                        } else {
                            context.invoke(callee.clone(), &[Value::list(items)])
                        }
                    },
                ),
            )
            .unwrap();
        runtime
            .execute_source("$result=Host::send();", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    }
}

#[test]
fn owned_invocation_preserves_arity_snapshots_reentrancy_and_control_flow() {
    let mut runtime = Runtime::new();
    register_owned_bridge(&mut runtime);
    let mut output = Vec::new();
    runtime
        .execute_source(
            r#"
        $input=[[1]];
        $changed=Host::owned(function($v){$v[0][]=2;return $v;},$input);
        $zero=Host::owned(function(){return 7;});
        $two=Host::owned(function($a,$b){return $a+$b;},2,3);
        $three=Host::owned(function($a,$b,$c){return $a+$b+$c;},2,3,4);
        $nested=Host::owned(function($v){return Host::owned(function($n){return $n+1;},$v);},10);
        $caught=0;
        try {Host::owned(function(){throw 17;});} catch(int $e) {$caught=$e;}
        $invalid=0;
        try {Host::owned(7);} catch(TypeError $e) {$invalid+=1;}
        try {Host::owned(function($v){return $v;});} catch(TypeError $e) {$invalid+=1;}
        $result=42;
    "#,
            &mut output,
        )
        .unwrap();
    assert_eq!(
        runtime.get_global("input"),
        Some(&Value::list(vec![Value::list(vec![Value::Int(1)])]))
    );
    assert_eq!(
        runtime.get_global("changed"),
        Some(&Value::list(vec![Value::list(vec![
            Value::Int(1),
            Value::Int(2)
        ])]))
    );
    for (name, value) in [
        ("zero", 7),
        ("two", 5),
        ("three", 9),
        ("nested", 11),
        ("caught", 17),
        ("invalid", 2),
    ] {
        assert_eq!(runtime.get_global(name), Some(&Value::Int(value)), "{name}");
    }
    let source = "$result=Host::owned(function($v){return $v/0;},1);";
    let error = runtime.execute_source(source, &mut output).unwrap_err();
    assert!(error.message.starts_with("ArithmeticError:"));
    assert_eq!(error.span.start, source.find("$v/0").unwrap());
    assert!(
        error
            .trace
            .iter()
            .any(|frame| frame.function == "Host::owned")
    );
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    assert_eq!(
        runtime
            .execute_source("$result=Host::owned(function(){exit 7;});", &mut output)
            .unwrap(),
        7
    );
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
}

#[test]
fn owned_invocation_keeps_validation_limits_and_foreign_owner_checks() {
    use scr_runtime::MapKey;
    use std::{cell::Cell, rc::Rc};
    let text = "x".repeat(60);
    let exact = 2 * size_of::<Value>() + text.len();
    let mut cases = Vec::new();
    for limit in [exact - 1, exact, exact + 1] {
        cases.push((
            Value::list(vec![Value::String(text.clone())]),
            Some(limit),
            limit >= exact,
        ));
    }
    cases.push((Value::Float(f64::NAN), None, false));
    cases.push((
        Value::Map(vec![
            (MapKey::Int(1), Value::Int(1)),
            (MapKey::Int(1), Value::Int(2)),
        ]),
        None,
        false,
    ));
    let mut deep = Value::Int(1);
    for _ in 0..129 {
        deep = Value::list(vec![deep]);
    }
    cases.push((deep, None, false));
    for (value, limit, success) in cases {
        let calls = Rc::new(Cell::new(0));
        let recorded = calls.clone();
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: limit,
            ..Default::default()
        });
        runtime
            .register(
                "Host::accept",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::Any], NativeType::Int),
                    move |_, _| {
                        recorded.set(recorded.get() + 1);
                        Ok(Value::Int(7))
                    },
                ),
            )
            .unwrap();
        let callee = runtime.get_callable("Host::accept").unwrap();
        runtime
            .register(
                "Host::send",
                NativeFunction::reentrant(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |context, _| context.invoke_owned(callee.clone(), [value.clone()]),
                ),
            )
            .unwrap();
        assert_eq!(
            runtime
                .execute_source("$result=Host::send();", &mut Vec::new())
                .is_ok(),
            success
        );
        assert_eq!(calls.get(), usize::from(success));
    }
    let mut foreign = Runtime::new();
    foreign
        .execute_source("function f(){return 1;}", &mut Vec::new())
        .unwrap();
    let mut runtime = Runtime::new();
    register_owned_bridge(&mut runtime);
    runtime
        .set_global("foreign", foreign.get_callable("f").unwrap())
        .unwrap();
    let error = runtime
        .execute_source("Host::owned($foreign);", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("different runtime"));
}

#[test]
fn owned_invocation_preserves_fatal_budgets_and_cancellation() {
    use std::sync::{Arc, atomic::AtomicBool};
    let cases = [
        (
            RuntimeOptions {
                instruction_limit: Some(20),
                ..Default::default()
            },
            "Host::owned(function(){while(true){}});",
            "instruction",
        ),
        (
            RuntimeOptions {
                max_scopes: Some(1),
                ..Default::default()
            },
            "Host::owned(function($v){return $v;},1);",
            "scope",
        ),
        (
            RuntimeOptions {
                recursion_limit: 2,
                ..Default::default()
            },
            "function f(){return Host::owned(f);} f();",
            "call depth",
        ),
        (
            RuntimeOptions {
                interrupt_flag: Some(Arc::new(AtomicBool::new(true))),
                ..Default::default()
            },
            "Host::owned(function(){return 1;});",
            "interrupt",
        ),
    ];
    for (options, source, message) in cases {
        let mut runtime = Runtime::with_options(options);
        register_owned_bridge(&mut runtime);
        let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
        assert!(error.message.contains(message), "{error:?}");
    }
}
