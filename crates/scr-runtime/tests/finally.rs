use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn execute(runtime: &mut Runtime, body: &str) -> (u8, String) {
    let mut output = Vec::new();
    let code = runtime
        .execute_source(&format!("<?scr {body}"), &mut output)
        .unwrap();
    (code, String::from_utf8(output).unwrap())
}

#[test]
fn cleanup_runs_once_for_normal_caught_and_uncaught_paths() {
    let mut runtime = Runtime::new();
    let (_, output) = execute(
        &mut runtime,
        r#"
        try { echo 'a'; } finally { echo 'b'; }
        try { throw 'x'; } catch (string $e) { echo $e; } finally { echo 'c'; }
        try { 1 / 0; } catch (ArithmeticError $e) { echo 'd'; } finally { echo 'e'; }
        try {
            try { throw 7; } catch (string $e) { echo 'wrong'; } finally { echo 'f'; }
        } catch (int $e) { echo 'g'; } finally { echo 'h'; }
        try {
            try { throw 'first'; } catch ($e) { throw 'second'; } finally { echo 'i'; }
        } catch ($e) { echo $e; }
    "#,
    );
    assert_eq!(output, "abxcdefghisecond");
}

#[test]
fn return_values_are_preserved_and_cleanup_can_replace_control_flow() {
    let mut runtime = Runtime::new();
    let (code, output) = execute(
        &mut runtime,
        r#"
        function original() {
            $value = [1]; try { return $value; } finally { $value[] = 2; echo 'a'; }
        }
        function replaced() { try { return 1; } finally { return 2; } }
        function suppressed() { try { throw 'old'; } finally { return 3; } }
        $original = original(); $replaced = replaced(); $suppressed = suppressed();
        try { exit 7; } finally { echo 'b'; }
        echo 'wrong';
    "#,
    );
    assert_eq!(code, 7);
    assert_eq!(output, "ab");
    assert_eq!(
        runtime.get_global("original"),
        Some(&Value::List(vec![Value::Int(1)]))
    );
    assert_eq!(runtime.get_global("replaced"), Some(&Value::Int(2)));
    assert_eq!(runtime.get_global("suppressed"), Some(&Value::Int(3)));
    assert_eq!(
        execute(&mut runtime, "try { exit 4; } finally { exit 9; }").0,
        9
    );
    let (_, output) = execute(
        &mut runtime,
        "try { try { exit 4; } finally { throw 'new'; } } catch ($e) { echo $e; }",
    );
    assert_eq!(output, "new");
}

#[test]
fn loop_control_unwinds_nested_cleanup_and_can_be_replaced() {
    let mut runtime = Runtime::new();
    let (_, output) = execute(
        &mut runtime,
        r#"
        for ($i = 0; $i < 3; $i += 1) {
            try { try { continue; } finally { echo 'a'; } } finally { echo 'b'; }
        }
        while (true) { try { break; } finally { echo 'c'; } }
        for ($i = 0; $i < 3; $i += 1) {
            try { break; } finally { echo 'd'; continue; }
        }
        while (true) { try { continue; } finally { echo 'e'; break; } }
    "#,
    );
    assert_eq!(output, "abababcddde");
}

#[test]
fn replacement_errors_keep_their_location_and_stack() {
    let mut runtime = Runtime::new();
    let source = "<?scr\nfunction fail() {\ntry { throw 'old'; } finally {\n1 / 0;\n}\n}\nfail();";
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with("ArithmeticError:"));
    assert_eq!(&source[error.span.start..error.span.end], "1 / 0");
    assert_eq!(error.trace.last().unwrap().function, "fail");
    execute(&mut runtime, "$after = true;");
    assert_eq!(runtime.get_global("after"), Some(&Value::Bool(true)));
}

#[test]
fn finally_has_its_own_scope_and_runs_after_catch_scope_is_released() {
    let mut runtime = Runtime::new();
    execute(
        &mut runtime,
        r#"
        $seen = '';
        try { $local = 'body'; throw 'value'; }
        catch ($e) { $seen = $e; $local = 'catch'; }
        finally { $local = 'cleanup'; $seen .= '!'; }
    "#,
    );
    assert_eq!(
        runtime.get_global("seen"),
        Some(&Value::String("value!".into()))
    );
    assert!(runtime.get_global("local").is_none());
    assert!(runtime.get_global("e").is_none());
}

#[test]
fn fatal_errors_skip_cleanup_and_cleanup_cannot_escape_limits() {
    for (options, body, message) in [
        (
            RuntimeOptions::default(),
            "$undefined;",
            "undefined variable",
        ),
        (
            RuntimeOptions {
                instruction_limit: Some(40),
                ..Default::default()
            },
            "while (true) {}",
            "instruction limit",
        ),
        (
            RuntimeOptions {
                max_scopes: Some(2),
                ..Default::default()
            },
            "{}",
            "scope limit",
        ),
        (
            RuntimeOptions {
                recursion_limit: 2,
                ..Default::default()
            },
            "recurse();",
            "call depth",
        ),
        (
            RuntimeOptions {
                max_value_bytes: Some(64),
                ..Default::default()
            },
            "'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx';",
            "byte limit",
        ),
    ] {
        let mut runtime = Runtime::with_options(options);
        let source = format!(
            "<?scr function recurse() {{ recurse(); }} $cleaned = false; try {{ {body} }} finally {{ $cleaned = true; }}"
        );
        let error = runtime
            .execute_source(&source, &mut Vec::new())
            .unwrap_err();
        assert!(error.message.contains(message), "{}", error.message);
        assert_eq!(runtime.get_global("cleaned"), Some(&Value::Bool(false)));
    }
    let mut runtime = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(40),
        ..Default::default()
    });
    let error = runtime
        .execute_source(
            "<?scr try { exit 3; } finally { while (true) {} }",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.contains("instruction limit"));
}

#[test]
fn cancellation_bypasses_catches_and_finally() {
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    let cancel = flag.clone();
    runtime
        .register(
            "cancel",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                move |_, _| {
                    cancel.store(true, Ordering::Relaxed);
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    let error = runtime.execute_source("<?scr $cleaned = false; $caught = false; try { cancel(); } catch ($e) { $caught = true; } finally { $cleaned = true; }", &mut Vec::new()).unwrap_err();
    assert!(error.message.contains("interrupted"));
    assert_eq!(runtime.get_global("cleaned"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
    flag.store(false, Ordering::Relaxed);
    execute(&mut runtime, "$reused = true;");
    assert_eq!(runtime.get_global("reused"), Some(&Value::Bool(true)));
}

#[test]
fn cleanup_errors_preserve_diagnostic_redaction() {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: scr_runtime::ContextOptions {
            redacted_values: vec!["cleanup-secret".into()],
            ..Default::default()
        },
        ..Default::default()
    });
    let source = "<?scr try { throw 'old'; } finally { throw 'cleanup-secret'; }";
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    let rendered = error.render("test.scr", source);
    assert!(!rendered.contains("cleanup-secret"));
    assert!(rendered.contains("[REDACTED]"));
}
