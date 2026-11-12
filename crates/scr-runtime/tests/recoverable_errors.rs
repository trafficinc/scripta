use scr_runtime::{
    ContextOptions, NativeFunction, NativeSignature, NativeType, Runtime, RuntimeError,
    RuntimeOptions, Value,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, RuntimeError> {
    runtime.execute_source(&format!("<?scr {source}"), &mut Vec::new())
}

#[test]
fn core_arithmetic_and_index_errors_are_catchable_by_name() {
    for (kind, body) in [
        ("ArithmeticError", "1 / 0;"),
        ("ArithmeticError", "1 % 0;"),
        ("ArithmeticError", "1.0 / -0.0;"),
        ("ArithmeticError", "1.0 % 0.0;"),
        ("ArithmeticError", "9223372036854775807 + 1;"),
        ("ArithmeticError", "(-9223372036854775807 - 1) - 1;"),
        ("ArithmeticError", "9223372036854775807 * 2;"),
        ("ArithmeticError", "-(-9223372036854775807 - 1);"),
        ("ArithmeticError", "(-9223372036854775807 - 1) / -1;"),
        ("ArithmeticError", "(-9223372036854775807 - 1) % -1;"),
        ("ArithmeticError", "1e308 * 2.0;"),
        ("ArithmeticError", "1e308 + 1e308;"),
        ("ArithmeticError", "-1e308 - 1e308;"),
        ("IndexError", "$a = [1]; $a[1];"),
        ("IndexError", "([1])[1];"),
        ("IndexError", "function items() { return [1]; } items()[2];"),
        ("IndexError", "$a = [1]; $a[-1];"),
        ("IndexError", "$a = [1]; $a[2] = 7;"),
        ("IndexError", "$a = [1]; $a[1] += 7;"),
        ("IndexError", "$a = [[]]; $a[1][] = 7;"),
        ("IndexError", "$a = [\"x\" => 1]; $a[\"missing\"];"),
        ("IndexError", "([\"x\" => 1])[\"missing\"];"),
        ("IndexError", "$a = [\"1\" => 1]; $a[1];"),
        ("IndexError", "$a = [\"x\" => []]; $a[\"missing\"][0] = 7;"),
    ] {
        let mut runtime = Runtime::new();
        let source = format!(
            "$caught = false; $message = ''; try {{ {body} }} catch ({kind} $e) {{ $caught = true; $message = $e; }} $after = true;"
        );
        run(&mut runtime, &source).unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{body}"
        );
        assert_eq!(runtime.get_global("after"), Some(&Value::Bool(true)));
        let Some(Value::String(message)) = runtime.get_global("message") else {
            panic!()
        };
        assert!(
            message.starts_with(&format!("{kind}:")),
            "{body}: {message}"
        );
        assert!(runtime.get_global("e").is_none());
    }
}

#[test]
fn operand_condition_and_index_type_failures_share_type_error() {
    for body in [
        "1 + 1.0;",
        "1 + '1';",
        "true * 2;",
        "-true;",
        "!1;",
        "true && 1;",
        "false || 1;",
        "'1' < 1;",
        "'a' . 1;",
        "$s = 'a'; $s .= 1;",
        "$s = 'a'; $r = ($s .= 1);",
        "if (1) {}",
        "while (0) {}",
        "for (; 1;) {}",
        "$a = [1]; $a['0'];",
        "$a = [1]; $a[true] = 2;",
        "$a = ['x' => 1]; $a[null];",
        "$a = ['x' => 1]; $a[true] = 2;",
        "[true => 1];",
        "$a = null; $a[0];",
        "null[0];",
        "$a = 1; $a[0] = 2;",
        "$a = null; $a[] = 2;",
    ] {
        let mut runtime = Runtime::new();
        run(
            &mut runtime,
            &format!(
                "$caught = false; try {{ {body} }} catch (TypeError $e) {{ $caught = true; }}"
            ),
        )
        .unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{body}"
        );
    }
}

#[test]
fn catch_matching_uses_exception_kind_and_unmatched_errors_keep_their_source() {
    for (kind, body) in [
        ("ArithmeticError", "1 / 0;"),
        ("IndexError", "[1][4];"),
        ("TypeError", "1 + true;"),
    ] {
        for catch in [kind, "Error", ""] {
            let mut runtime = Runtime::new();
            run(&mut runtime, &format!("$caught = false; try {{ try {{ {body} }} catch (string $e) {{ throw 'wrong catch'; }} }} catch ({catch} $e) {{ $caught = true; }}")).unwrap();
            assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
        }
        let mut runtime = Runtime::new();
        let error = run(
            &mut runtime,
            &format!("try {{ {body} }} catch (OtherError $e) {{}}"),
        )
        .unwrap_err();
        assert!(error.message.starts_with(kind));
        assert_eq!(error.code, "E0401");
    }
    let mut runtime = Runtime::new();
    run(&mut runtime, "$caught = false; try { try { 1 / 0; } catch (ArithmeticError $e) { throw $e; } } catch (string $e) { $caught = true; }").unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
}

#[test]
fn failed_writes_preserve_destinations_but_not_prior_rhs_side_effects() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        $items = [[1], ["value" => 2]]; $original = $items;
        try { $items[0][0] /= 0; } catch (ArithmeticError $e) {}
        try { $items[0][9] = 3; } catch (IndexError $e) {}
        try { $items[1]["missing"] += 1; } catch (IndexError $e) {}
        try { $items[1]["missing"] = 1 / 0; } catch (ArithmeticError $e) {}
        try { $items[0][0] += true; } catch (TypeError $e) {}
        $text = "safe"; try { $text .= 1; } catch (TypeError $e) {}
        $number = 9223372036854775807; try { $number += 1; } catch (ArithmeticError $e) {}
        function effects() {
            $items = [1]; $indexes = 0; $calls = 0;
            function index() { $indexes += 1; return 0; }
            function change() { $calls += 1; $items[0] = 9; return 0; }
            try { $items[index()] /= change(); } catch (ArithmeticError $e) {}
            return [$items, $indexes, $calls];
        }
        $effects = effects();
    "#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("items"), runtime.get_global("original"));
    assert_eq!(
        runtime.get_global("text"),
        Some(&Value::String("safe".into()))
    );
    assert_eq!(runtime.get_global("number"), Some(&Value::Int(i64::MAX)));
    assert_eq!(
        runtime.get_global("effects"),
        Some(&Value::List(vec![
            Value::List(vec![Value::Int(9)]),
            Value::Int(1),
            Value::Int(1)
        ]))
    );
    // Runtime reuse and the optional transactional write path preserve the same behavior.
    let mut budgeted = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(4096),
        ..Default::default()
    });
    run(
        &mut budgeted,
        "$items = [[1]]; try { $items[0][9] = 2; } catch (IndexError $e) {} $after = $items[0][0];",
    )
    .unwrap();
    assert_eq!(budgeted.get_global("after"), Some(&Value::Int(1)));
}

#[test]
fn uncaught_failures_retain_inner_spans_frames_and_diagnostic_redaction() {
    for (expression, kind) in [
        ("1 / 0", "ArithmeticError"),
        ("$items[9]", "IndexError"),
        ("1 + true", "TypeError"),
    ] {
        let source = format!(
            "<?scr function inner($items) {{ return {expression}; }} function outer() {{ return inner([1]); }} outer();"
        );
        let mut runtime = Runtime::new();
        let error = runtime
            .execute_source(&source, &mut Vec::new())
            .unwrap_err();
        assert!(error.message.starts_with(kind));
        assert_eq!(&source[error.span.start..error.span.end], expression);
        assert_eq!(
            error
                .trace
                .iter()
                .map(|f| f.function.as_str())
                .collect::<Vec<_>>(),
            vec!["<main>", "outer", "inner"]
        );
        assert_eq!(error.source.as_ref().unwrap().text, source);
        let rendered = error.render("test.scr", &source);
        assert!(rendered.contains("error[E0401]"));
        assert!(rendered.find("at inner").unwrap() < rendered.find("at outer").unwrap());
        run(&mut runtime, "$recovered = true;").unwrap();
    }
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            redacted_values: vec!["division by zero".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    run(&mut runtime, "$caught = false; $message = ''; try { 1 / 0; } catch (ArithmeticError $e) { $caught = true; $message = $e; }").unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    let Some(Value::String(message)) = runtime.get_global("message") else {
        panic!()
    };
    assert!(!message.contains("division by zero"));
    let error = run(&mut runtime, "1 / 0;").unwrap_err();
    assert!(!format!("{error:?}").contains("division by zero"));
    assert!(
        !error
            .render("test.scr", "<?scr 1 / 0;")
            .contains("division by zero")
    );
}

#[test]
fn core_execution_limits_and_undefined_names_still_bypass_catches() {
    for (options, body, message) in [
        (
            RuntimeOptions {
                instruction_limit: Some(30),
                ..Default::default()
            },
            "while (true) {}",
            "instruction limit",
        ),
        (
            RuntimeOptions {
                max_scopes: Some(3),
                ..Default::default()
            },
            "{ { {} } }",
            "scope limit",
        ),
        (
            RuntimeOptions {
                max_value_bytes: Some(8 * std::mem::size_of::<Value>()),
                ..Default::default()
            },
            "$items = [1,2,3,4,5,6,7]; $items[] = 8;",
            "byte limit",
        ),
        (
            RuntimeOptions {
                max_source_bytes: 32,
                ..Default::default()
            },
            "echo 'too much source text';",
            "source byte limit",
        ),
        (
            RuntimeOptions {
                recursion_limit: 3,
                ..Default::default()
            },
            "function recurse() { recurse(); } recurse();",
            "maximum call depth",
        ),
        (
            RuntimeOptions::default(),
            "$nested = null; for ($i = 0; $i < 129; $i += 1) { $nested = [$nested]; }",
            "nesting exceeds 128",
        ),
        (RuntimeOptions::default(), "$missing;", "undefined variable"),
        (
            RuntimeOptions::default(),
            "missing();",
            "undefined function",
        ),
    ] {
        let mut runtime = Runtime::with_options(options);
        let error = run(
            &mut runtime,
            &format!("$caught = false; try {{ {body} }} catch (Error $e) {{ $caught = true; }}"),
        )
        .unwrap_err();
        assert!(error.message.contains(message), "{error}");
        assert_ne!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    }
}

#[test]
fn host_cancellation_inside_a_try_block_still_bypasses_catches() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    let trigger = flag.clone();
    runtime
        .register(
            "cancel",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                move |_, _| {
                    trigger.store(true, Ordering::Relaxed);
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    let error = run(
        &mut runtime,
        "$caught = false; try { cancel(); } catch (Error $e) { $caught = true; }",
    )
    .unwrap_err();
    assert!(error.message.contains("interrupted"));
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
    flag.store(false, Ordering::Relaxed);
    run(&mut runtime, "$recovered = true;").unwrap();
}
