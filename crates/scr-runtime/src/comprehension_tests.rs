use super::*;

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let (mut fast, mut reference) = crate::test_support::runtimes(options);
    reference.environments.disable_parameter_reuse = true;
    reference.disable_shared_parameter_names = true;
    reference.disable_comprehension_plans = true;
    let _ = crate::test_support::compare_with_recovery(
        &mut fast,
        &mut reference,
        source,
        "$recovered=7;",
    );
    fast
}

#[test]
fn comprehension_scopes_preserve_captures_shadowing_snapshots_and_nested_names() {
    let runtime = compare(
        "$x='outer';$source=[1,2,3];$mapped=[$x*3 for $x in $source if $x>1];\
         $callbacks=[function(){return $x+=1;} for $x in $source];\
         $seen=[$callbacks[0](),$callbacks[1](),$callbacks[0](),$callbacks[2]()];\
         $nested=[[$x for $x in [4,5]] for $x in [1,2]];\
         $maps=[$v for $v in ['a'=>1,'b'=>2]];\
         $copies=[$v for $v in [[1],[2]]];$copies[0][0]=9;\
         function snapshot(){$items=[1,2,3];function next($v){$items[1]=99;return $v;}\
         return [next($x) for $x in $items];}$snapshot=snapshot();",
        RuntimeOptions::default(),
    );
    assert_eq!(
        runtime.get_global("x"),
        Some(&Value::String("outer".into()))
    );
    assert_eq!(
        runtime.get_global("mapped"),
        Some(&Value::list(vec![Value::Int(6), Value::Int(9)]))
    );
    assert_eq!(
        runtime.get_global("seen"),
        Some(&Value::List(
            [2, 3, 3, 4].into_iter().map(Value::Int).collect()
        ))
    );
    assert_eq!(
        runtime.get_global("snapshot"),
        Some(&Value::List(
            [1, 2, 3].into_iter().map(Value::Int).collect()
        ))
    );
    assert!(runtime.comprehension_plans_executed > 0);
}

#[test]
fn scalar_plans_preserve_short_circuiting_errors_and_unsupported_inputs() {
    for source in [
        "$seen=[($x*3+1)/2 for $x in [1,2,3] if $x%2===0];",
        "$seen=[$x for $x in [null,true,1] if $x!==null];",
        "$seen=[$x for $x in [1,2,3] if true || (1/0===0)];",
        "$seen=[$x for $x in [1,2,3] if false && (1/0===0)];",
        "$seen='kept';$seen=[$x*9223372036854775807 for $x in [1,2]];",
        "$seen='kept';$seen=[-$x for $x in [-9223372036854775807-1]];",
        "$seen='kept';$seen=[$x for $x in [1] if $x];",
        "$seen=[$x*2.0 for $x in [1.0,2.0]];",
        "$seen=[$x for $x in ['é',1,[]]];",
        "$outside=3;$seen=[$x+$outside for $x in [1,2]];",
        "$seen=[($x+=1) for $x in [1,2]];",
    ] {
        compare(source, RuntimeOptions::default());
    }
    let source = "$seen='kept';$seen=[$x*3 for $x in [1,2,3] if $x%2===0];";
    for limit in 0..18 {
        compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    for limit in 0..5 {
        compare(
            source,
            RuntimeOptions {
                max_scopes: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
}

#[test]
fn scalar_plan_bounds_fall_back_and_cancellation_before_iteration_preserves_scopes() {
    for depth in [4, 24] {
        let source = format!(
            "$seen=[{}$x{} for $x in [1,2]];",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        let runtime = compare(&source, RuntimeOptions::default());
        assert_eq!(
            runtime.comprehension_plans_executed,
            usize::from(depth == 4)
        );
    }
    // Enter expression evaluation directly so cancellation occurs after the
    // outer statement checkpoint, during the new bounded scalar admission scan.
    let source = "$seen=[$x*3 for $x in $items];";
    let program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let StatementKind::Expression(Expr {
        kind: ExprKind::Assignment(_, _, expression),
        ..
    }) = &program.statements[0].kind
    else {
        panic!("assignment")
    };
    let flag = Arc::new(AtomicBool::new(true));
    let (mut fast, mut reference) = crate::test_support::runtimes(RuntimeOptions {
        interrupt_flag: Some(flag),
        ..RuntimeOptions::default()
    });
    reference.disable_comprehension_plans = true;
    for runtime in [&mut fast, &mut reference] {
        runtime
            .set_global(
                "items",
                Value::List([1, 2, 3].into_iter().map(Value::Int).collect()),
            )
            .unwrap();
    }
    let result = crate::test_support::compare_execution(
        &mut fast,
        &mut reference,
        source,
        |runtime, output| match runtime.evaluate(expression, output) {
            Err(Signal::Interrupted(error)) => Err(*error),
            _ => panic!("cancellation must interrupt the comprehension"),
        },
    );
    assert!(result.unwrap_err().message.contains("interrupted"));
}

#[test]
fn comprehension_failures_keep_destinations_limits_and_recovery() {
    for source in [
        "$result='kept';$result=[$x for $x in [1,2] if 1];",
        "$result='kept';$result=[$x/0 for $x in [1,2]];",
        "$result='kept';$result=[$x for $x in 1];",
        "$result='kept';function fail($v){if($v===2){throw 7;}return $v;}\
         try{$result=[fail($x) for $x in [1,2,3]];}catch(int $e){}",
    ] {
        let runtime = compare(source, RuntimeOptions::default());
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("kept".into()))
        );
    }
    let source = "$result='kept';$result=[[$y for $y in [1,2]] for $x in [1,2,3] if $x>1];";
    for limit in 0..30 {
        compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    for limit in 0..8 {
        compare(
            source,
            RuntimeOptions {
                max_scopes: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    let slot = size_of::<Value>();
    for limit in [
        0,
        slot - 1,
        slot,
        slot * 3 - 1,
        slot * 3,
        slot * 3 + 1,
        slot * 7 - 1,
        slot * 7,
        slot * 7 + 1,
    ] {
        compare(
            source,
            RuntimeOptions {
                max_value_bytes: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
}

#[test]
fn cancellation_in_a_comprehension_releases_transient_values_and_recovers() {
    let flag = Arc::new(AtomicBool::new(false));
    let (mut fast, mut reference) = crate::test_support::runtimes(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..RuntimeOptions::default()
    });
    reference.environments.disable_parameter_reuse = true;
    reference.disable_shared_parameter_names = true;
    for runtime in [&mut fast, &mut reference] {
        runtime
            .register(
                "trip",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                    |context, args| {
                        context
                            .cancellation
                            .as_ref()
                            .unwrap()
                            .store(true, Ordering::Relaxed);
                        Ok(args[0].clone())
                    },
                ),
            )
            .unwrap();
    }
    let source = "$result='kept';$result=[trip($x) for $x in [1,2,3]];";
    // Both executions share the flag; reset it before each run.
    let result = crate::test_support::compare_execution(
        &mut fast,
        &mut reference,
        source,
        |runtime, output| {
            flag.store(false, Ordering::Relaxed);
            runtime.execute_source(source, output)
        },
    );
    assert!(result.unwrap_err().message.contains("interrupted"));
    assert_eq!(
        fast.get_global("result"),
        Some(&Value::String("kept".into()))
    );
    flag.store(false, Ordering::Relaxed);
    crate::test_support::compare_source(&mut fast, &mut reference, "$recovered=7;").unwrap();
}
