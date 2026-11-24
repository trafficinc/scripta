use super::*;

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let (mut fast, mut reference) = crate::test_support::runtimes(options);
    reference.environments.disable_parameter_reuse = true;
    reference.disable_shared_parameter_names = true;
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
        Some(&Value::List(vec![Value::Int(6), Value::Int(9)]))
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
