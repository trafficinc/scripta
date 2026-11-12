use scr_runtime::{Runtime, RuntimeOptions, Value};

fn runtime(limit: Option<usize>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}

fn evaluate(expression: &str) -> Value {
    let mut runtime = runtime(None);
    runtime
        .execute_source(&format!("<?scr $result = {expression};"), &mut Vec::new())
        .unwrap();
    runtime.get_global("result").unwrap().clone()
}

#[test]
fn numeric_helpers_preserve_types_and_cover_boundaries() {
    for (expression, expected) in [
        ("Math::abs(-7)", Value::Int(7)),
        ("Math::abs(9223372036854775807)", Value::Int(i64::MAX)),
        ("Math::abs(-2.5)", Value::Float(2.5)),
        (
            "Math::min(int('-9223372036854775808'), 9223372036854775807)",
            Value::Int(i64::MIN),
        ),
        (
            "Math::max(int('-9223372036854775808'), 9223372036854775807)",
            Value::Int(i64::MAX),
        ),
        ("Math::min(-2.5, 3.0)", Value::Float(-2.5)),
        ("Math::max(-2.5, 3.0)", Value::Float(3.0)),
        ("Math::floor(-1.2)", Value::Float(-2.0)),
        ("Math::ceil(-1.2)", Value::Float(-1.0)),
        ("Math::floor(1.2)", Value::Float(1.0)),
        ("Math::ceil(1.2)", Value::Float(2.0)),
        ("Math::round(2.5)", Value::Float(3.0)),
        ("Math::round(-2.5)", Value::Float(-3.0)),
        ("Math::round(2.49)", Value::Float(2.0)),
        ("Math::round(-2.49)", Value::Float(-2.0)),
        (
            "Math::round(1.7976931348623157e308)",
            Value::Float(f64::MAX),
        ),
        (
            "Math::ceil(-1.7976931348623157e308)",
            Value::Float(-f64::MAX),
        ),
    ] {
        assert_eq!(evaluate(expression), expected, "{expression}");
    }
    for name in ["floor", "ceil", "round"] {
        for value in [i64::MIN, 0, i64::MAX] {
            assert_eq!(
                evaluate(&format!("Math::{name}(int('{value}'))")),
                Value::Int(value)
            );
        }
    }
    for expression in [
        "Math::min(-0.0, 0.0)",
        "Math::max(-0.0, 0.0)",
        "Math::ceil(-0.1)",
        "Math::round(-0.1)",
    ] {
        let Value::Float(value) = evaluate(expression) else {
            panic!("expected float")
        };
        assert_eq!(value.to_bits(), (-0.0f64).to_bits());
    }
    let Value::Float(value) = evaluate("Math::abs(-0.0)") else {
        panic!("expected float")
    };
    assert_eq!(value.to_bits(), 0.0f64.to_bits());
}

#[test]
fn formatting_supports_unicode_scalar_display_and_literal_escapes() {
    for (expression, expected) in [
        (r#"String::format("", [])"#, ""),
        (r#"String::format("café 🦀 é", [])"#, "café 🦀 é"),
        (r#"String::format("{{{}}} {{}}", ["東京"])"#, "{東京} {}"),
        (
            r#"String::format("{} {} {} {} {}", [null, true, -12, 1.0, "{}{{}}🦀"])"#,
            "null true -12 1 {}{{}}🦀",
        ),
        (r#"String::format("{}{}", ["", false])"#, "false"),
    ] {
        assert_eq!(
            evaluate(expression),
            Value::String(expected.into()),
            "{expression}"
        );
    }
}

#[test]
fn invalid_inputs_are_catchable_and_preserve_failed_destinations() {
    let mut runtime = runtime(None);
    let mut cases = vec![(
        "Math::abs(int('-9223372036854775808'))".to_string(),
        "ArithmeticError",
    )];
    for name in ["abs", "floor", "ceil", "round"] {
        for args in ["", "1, 2", "true", "null", "'2'", "[]"] {
            cases.push((format!("Math::{name}({args})"), "TypeError"));
        }
    }
    for name in ["min", "max"] {
        for args in [
            "",
            "1",
            "1, 2, 3",
            "1, 2.0",
            "1.0, 2",
            "'1', '2'",
            "false, false",
        ] {
            cases.push((format!("Math::{name}({args})"), "TypeError"));
        }
    }
    for expression in [
        "String::format('{}', [])",
        "String::format('', [1])",
        "String::format('{', [])",
        "String::format('}', [])",
        "String::format('{0}', [1])",
        "String::format('{:.2f}', [1.0])",
        "String::format('{{}', [])",
    ] {
        cases.push((expression.into(), "FormatError"));
    }
    for expression in [
        "String::format()",
        "String::format(1, [])",
        "String::format('', 1)",
        "String::format('{}', [[]])",
        "String::format('{}', [function () {}])",
        "String::format('', [], 1)",
    ] {
        cases.push((expression.into(), "TypeError"));
    }
    for (expression, kind) in cases {
        runtime.execute_source(&format!("<?scr $result = 'unchanged'; $caught = false; try {{ $result = {expression}; }} catch ({kind} $e) {{ $caught = true; }}"), &mut Vec::new()).unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{expression}"
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
}

#[test]
fn expanded_format_output_obeys_limits_and_runtime_can_be_reused() {
    let mut runtime = runtime(Some(4 * std::mem::size_of::<Value>()));
    let slot = std::mem::size_of::<Value>();
    let template = "x".repeat(3 * slot - 2) + "{}";
    let value = "y".repeat(slot);
    runtime.execute_source(&format!("<?scr $result = 'unchanged'; $caught = false; try {{ $result = String::format('{template}', ['{value}']); }} catch (ResourceLimitError $e) {{ $caught = true; }}"), &mut Vec::new()).unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
    runtime
        .execute_source(
            "<?scr $result = String::format('{}', [7]);",
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("7".into()))
    );
}
