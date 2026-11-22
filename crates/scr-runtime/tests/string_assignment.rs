use scr_ast::{ExprKind, StatementKind};
use scr_runtime::{
    NativeFunction, NativeSignature, NativeType, Runtime, RuntimeError, RuntimeOptions, SourceFile,
    Value,
};
use std::{path::PathBuf, sync::Arc};

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, RuntimeError> {
    runtime.execute_source(source, &mut Vec::new())
}

// A grouped assignment uses the existing value-producing path. Preserve the
// original source/spans so errors can be compared exactly against buffer reuse.
fn variant(runtime: &mut Runtime, source: &str, general: bool) -> Result<u8, RuntimeError> {
    let mut program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
    if general {
        for statement in &mut program.statements {
            if let StatementKind::Expression(expr) = &mut statement.kind {
                if matches!(expr.kind, ExprKind::Assignment(_, _, _)) {
                    *expr = scr_ast::Expr {
                        span: expr.span,
                        kind: ExprKind::Grouped(Box::new(expr.clone())),
                    };
                }
            }
        }
    }
    runtime.execute_program_with_source(
        &program,
        Arc::new(SourceFile {
            path: PathBuf::from("string-assignment.scripta"),
            text: source.into(),
        }),
        &mut Vec::new(),
    )
}

#[test]
fn discarded_self_concatenation_reuses_capacity_and_preserves_string_copies() {
    let mut initial = String::with_capacity(4096);
    initial.push('é');
    let pointer = initial.as_ptr();
    let mut runtime = Runtime::new();
    runtime.set_global("s", Value::String(initial)).unwrap();
    run(
        &mut runtime,
        r#"
        $copy = $s;
        $suffix = "東京";
        $pieces = ["!", "?"];
        $s = $s . $suffix;
        $s = (($s) . ($pieces[0] . $pieces[1]));
        $s = $s . "";
        $s = $s . $s;
        $s .= "👍🏽";
    "#,
    )
    .unwrap();
    let Some(Value::String(text)) = runtime.get_global("s") else {
        panic!("string result");
    };
    assert_eq!(text, "é東京!?é東京!?👍🏽");
    assert_eq!(
        text.as_ptr(),
        pointer,
        "existing capacity should survive discarded appends"
    );
    assert_eq!(runtime.get_global("copy"), Some(&Value::String("é".into())));
}

#[test]
fn used_results_chains_indexed_targets_and_captured_variables_keep_value_semantics() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        $s = "a"; $copy = $s;
        $result = ($s = $s . "b");
        $outer = $s = $s . "c";
        $s = $s . "d" . "e";
        $a = "left"; $b = "right"; $a = $b . "!";
        $prepend = "a"; $prepend = "b" . $prepend;
        $items = ["x"]; $saved = $items;
        $items[0] = $items[0] . "y";
        class Box { $text = "old"; }
        $box = new Box(); $box->text = $box->text . "!"; $field = $box->text;
        function factory() {
            $s = "é";
            function append($suffix) { $s = $s . $suffix; return $s; }
            return append;
        }
        $append = factory(); $first = $append("東"); $second = $append("京");
        function indexed() {
            $values = ["x"]; $calls = 0;
            function position() { $calls += 1; return 0; }
            $values[position()] = $values[0] . "y";
            return [$values[0], $calls];
        }
        $indexed = indexed();
    "#,
    )
    .unwrap();
    for (name, expected) in [
        ("a", "right!"),
        ("b", "right"),
        ("prepend", "ba"),
        ("copy", "a"),
        ("result", "ab"),
        ("outer", "abc"),
        ("s", "abcde"),
        ("field", "old!"),
        ("first", "é東"),
        ("second", "é東京"),
    ] {
        assert_eq!(
            runtime.get_global(name),
            Some(&Value::String(expected.into())),
            "{name}"
        );
    }
    assert_eq!(
        runtime.get_global("items"),
        Some(&Value::List(vec![Value::String("xy".into())]))
    );
    assert_eq!(
        runtime.get_global("saved"),
        Some(&Value::List(vec![Value::String("x".into())]))
    );
    assert_eq!(
        runtime.get_global("indexed"),
        Some(&Value::List(vec![
            Value::String("xy".into()),
            Value::Int(1)
        ]))
    );
}

#[test]
fn effectful_suffixes_keep_the_original_prefix_snapshot_and_prior_effects_on_failure() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        function called() {
            $s = "a";
            function change() { $s = "b"; return "c"; }
            $s = $s . change(); return $s;
        }
        function assigned() { $s = "a"; $s = $s . ($s = "b"); return $s; }
        function failed() {
            $s = "a";
            function change() { $s = "b"; throw 7; }
            try { $s = $s . change(); } catch (int $e) {}
            return $s;
        }
        $called = called(); $assigned = assigned(); $failed = failed();
    "#,
    )
    .unwrap();
    for (name, expected) in [("called", "ac"), ("assigned", "ab"), ("failed", "b")] {
        assert_eq!(
            runtime.get_global(name),
            Some(&Value::String(expected.into())),
            "{name}"
        );
    }
}

#[test]
fn optimized_and_general_errors_keep_categories_spans_sources_and_destinations() {
    for source in [
        "$s = 'é'; $s = $s . 1;",
        "$s = 'é'; $s = $s . $missing;",
        "$s = 'é'; $parts = []; $s = $s . $parts[0];",
        "$s = 'é'; $s = $s . (1 / 0);",
        "$s = null; $s = $s . 'x';",
        "$s = 'é'; $s = $s . (1 + 'x');",
        "$s = $s . 'x';",
    ] {
        let mut outcomes = Vec::new();
        for general in [false, true] {
            let mut runtime = Runtime::new();
            let error = variant(&mut runtime, source, general).unwrap_err();
            outcomes.push((error, runtime.get_global("s").cloned()));
            run(&mut runtime, "$recovered = true;").unwrap();
        }
        assert_eq!(outcomes[0], outcomes[1], "{source}");
    }
    let definition = "function fail($s) { $s = $s . 1; }";
    let mut runtime = Runtime::new();
    run(&mut runtime, definition).unwrap();
    let error = run(&mut runtime, "fail('é');").unwrap_err();
    assert!(
        error
            .message
            .starts_with("TypeError: concatenation requires two strings")
    );
    assert_eq!(error.source.as_ref().unwrap().text, definition);
    assert_eq!(&definition[error.span.start..error.span.end], "$s . 1");
    assert_eq!(error.trace.last().unwrap().function, "fail");
}

#[test]
fn byte_limits_are_exact_and_fail_without_mutation_including_intermediate_concats() {
    let slot = std::mem::size_of::<Value>();
    for (source, payload, succeeds) in [
        ("$s = ''; $s = $s . '';", 0, true),
        ("$s = 'é'; $s = $s . 'é';", 4, true),
        ("$s = 'é'; $s = $s . 'ab';", 3, false),
        ("$s = 'ab'; $s = (($s) . ('c' . 'd'));", 4, true),
        ("$s = 'ab'; $s = (($s) . ('c' . 'd'));", 3, false),
        ("$s = 'ab'; $s = $s . 'cd' . 'ef';", 3, false),
        ("$s = 'ab'; $s = $s . 'cd' . 'ef';", 5, false),
        ("$s = 'ab'; $s = $s . 'toolong';", 3, false),
        ("$s = 'ab'; $s = $s . $s;", 4, true),
        ("$s = 'ab'; $s = $s . $s;", 3, false),
    ] {
        let mut outcomes = Vec::new();
        for general in [false, true] {
            let mut runtime = Runtime::with_options(RuntimeOptions {
                max_value_bytes: Some(slot + payload),
                ..Default::default()
            });
            let outcome = variant(&mut runtime, source, general);
            assert_eq!(outcome.is_ok(), succeeds, "{source}, budget {payload}");
            outcomes.push((outcome, runtime.get_global("s").cloned()));
        }
        assert_eq!(outcomes[0], outcomes[1], "{source}, budget {payload}");
    }
}

#[test]
fn instruction_limits_during_pure_suffix_evaluation_leave_the_destination_unchanged() {
    for general in [false, true] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            instruction_limit: Some(1),
            ..Default::default()
        });
        runtime
            .set_global("s", Value::String("unchanged".into()))
            .unwrap();
        runtime.set_global("needle", Value::Int(99)).unwrap();
        runtime
            .set_global("values", Value::List(vec![Value::Int(0); 10]))
            .unwrap();
        let source = "$s = $s . ($needle in $values);";
        let error = variant(&mut runtime, source, general).unwrap_err();
        assert!(
            error
                .message
                .contains("execution instruction limit exceeded")
        );
        assert_eq!(
            &source[error.span.start..error.span.end],
            "$needle in $values"
        );
        assert_eq!(
            runtime.get_global("s"),
            Some(&Value::String("unchanged".into()))
        );
        run(&mut runtime, "$recovered = true;").unwrap();
    }
}

#[test]
fn cancellation_before_assignment_or_in_an_effectful_suffix_prevents_commit() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = flag.clone();
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    runtime
        .register(
            "cancel",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::String),
                move |_, _| {
                    trigger.store(true, Ordering::Relaxed);
                    Ok(Value::String("suffix".into()))
                },
            ),
        )
        .unwrap();
    run(&mut runtime, "$s = 'unchanged';").unwrap();
    for source in ["$s = $s . 'x';", "$s = $s . cancel();"] {
        flag.store(!source.contains("cancel"), Ordering::Relaxed);
        let error = run(&mut runtime, source).unwrap_err();
        assert!(error.message.contains("execution interrupted"));
        assert_eq!(
            runtime.get_global("s"),
            Some(&Value::String("unchanged".into()))
        );
        flag.store(false, Ordering::Relaxed);
        run(&mut runtime, "$recovered = true;").unwrap();
    }
}

#[test]
fn native_property_suffixes_keep_ordinary_getter_evaluation() {
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Suffix(Rc<Cell<usize>>);
    impl scr_runtime::NativeObject for Suffix {
        fn type_name(&self) -> &str {
            "Suffix"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            self.0.set(self.0.get() + 1);
            Ok(Value::String("é".into()))
        }
    }
    let calls = Rc::new(Cell::new(0));
    let mut runtime = Runtime::new();
    runtime
        .set_global(
            "object",
            Value::NativeObject(Rc::new(Suffix(calls.clone()))),
        )
        .unwrap();
    run(&mut runtime, "$s = 'a'; $s = $s . $object->text;").unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(runtime.get_global("s"), Some(&Value::String("aé".into())));
}
