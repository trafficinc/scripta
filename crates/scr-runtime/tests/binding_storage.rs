use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value};

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, scr_runtime::RuntimeError> {
    runtime.execute_source(source, &mut Vec::new())
}

#[test]
fn reused_call_and_block_storage_preserves_shadowing_and_global_isolation() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        $global = 42; $seen = 0;
        function first($parameter = 7) { $local = 9; return $parameter; }
        function second($parameter = 11) { return $parameter; }
        for ($i = 0; $i < 100; $i += 1) {
            $a = first(); $b = second(); { $seen = $global; }
        }
        $result = [$seen, first(13), second()];
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::List([42, 13, 11].map(Value::Int).into()))
    );
    for source in [
        "function missingParameter() { return $parameter; } missingParameter();",
        "function missingLocal() { return $local; } missingLocal();",
        "function globalRead() { return $global; } globalRead();",
    ] {
        assert!(
            run(&mut runtime, source)
                .unwrap_err()
                .message
                .contains("undefined variable")
        );
    }
    run(&mut runtime, "{ $temporary = 99; } { $seen = $global; }").unwrap();
    assert!(
        run(&mut runtime, "$temporary;")
            .unwrap_err()
            .message
            .contains("undefined variable")
    );
    assert_eq!(runtime.get_global("seen"), Some(&Value::Int(42)));
}

#[test]
fn captures_remain_independent_while_transient_frames_are_reused() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        function factory($value) {
            $local = $value;
            return function ($delta = 1) { $local += $delta; return $local; };
        }
        function transient($value) { $local = $value; return $local; }
        $first = factory(10); $second = factory(100);
        for ($i = 0; $i < 1000; $i += 1) { transient($i); }
        $a = $first(); $b = $second(); $c = $first(2);
    "#,
    )
    .unwrap();
    runtime.collect_garbage();
    run(&mut runtime, "$d = $second(3);").unwrap();
    for (name, expected) in [("a", 11), ("b", 101), ("c", 13), ("d", 104)] {
        assert_eq!(runtime.get_global(name), Some(&Value::Int(expected)));
    }
}

#[test]
fn lookups_observe_host_injection_and_new_declarations_after_previous_misses() {
    let mut runtime = Runtime::new();
    assert!(
        run(&mut runtime, "$value;")
            .unwrap_err()
            .message
            .contains("undefined variable")
    );
    runtime.set_global("value", Value::Int(13)).unwrap();
    run(&mut runtime, "$first = $value;").unwrap();
    runtime
        .set_global("value", Value::String("changed".into()))
        .unwrap();
    run(&mut runtime, "$second = $value;").unwrap();
    assert!(
        run(&mut runtime, "late();")
            .unwrap_err()
            .message
            .contains("undefined function")
    );
    runtime
        .register(
            "late",
            NativeFunction::new(NativeSignature::new(vec![], NativeType::Int), |_, _| {
                Ok(Value::Int(42))
            }),
        )
        .unwrap();
    run(
        &mut runtime,
        "$third = late(); function script() { return 7; } $fourth = script();",
    )
    .unwrap();
    assert_eq!(runtime.get_global("first"), Some(&Value::Int(13)));
    assert_eq!(
        runtime.get_global("second"),
        Some(&Value::String("changed".into()))
    );
    assert_eq!(runtime.get_global("third"), Some(&Value::Int(42)));
    assert_eq!(runtime.get_global("fourth"), Some(&Value::Int(7)));
}

#[test]
fn reentrant_callbacks_keep_resolved_targets_and_lexical_updates() {
    let mut runtime = Runtime::new();
    runtime
        .register(
            "bridge",
            NativeFunction::reentrant(
                NativeSignature::new(
                    vec![NativeType::Callable, NativeType::String],
                    NativeType::Any,
                ),
                |context, args| context.invoke(args[0].clone(), &args[1..]),
            ),
        )
        .unwrap();
    run(
        &mut runtime,
        r#"
        function exercise() {
            $target = "old";
            $success = function ($value) { $target = "prior"; return $value; };
            $target = bridge($success, "new"); $saved = $target;
            $failure = function ($value) { $target = $value; return 1 / 0; };
            try { $target = bridge($failure, "changed"); } catch (ArithmeticError $e) {}
            return [$saved, $target];
        }
        $result = exercise();
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::list(vec![
            Value::String("new".into()),
            Value::String("changed".into())
        ]))
    );
}

#[test]
fn reused_storage_respects_exact_scope_limits_and_recovers_after_failed_calls() {
    for max_scopes in [3, 4] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_scopes: Some(max_scopes),
            ..Default::default()
        });
        run(
            &mut runtime,
            "function identity($value: int): int { return $value; } $result = 0;",
        )
        .unwrap();
        // Global, block, parameter frame, and function body are four live scopes.
        let result = run(&mut runtime, "{ $result = identity(7); }");
        if max_scopes == 3 {
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("execution scope limit exceeded")
            );
            assert_eq!(runtime.get_global("result"), Some(&Value::Int(0)));
        } else {
            result.unwrap();
            assert_eq!(runtime.get_global("result"), Some(&Value::Int(7)));
        }
        assert!(
            run(&mut runtime, "identity('wrong');")
                .unwrap_err()
                .message
                .starts_with("TypeError:")
        );
        run(&mut runtime, "$recovered = identity(9);").unwrap();
        assert_eq!(runtime.get_global("recovered"), Some(&Value::Int(9)));
    }
}
