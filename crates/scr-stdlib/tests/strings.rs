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

fn list(values: &[&str]) -> Value {
    Value::List(values.iter().map(|s| Value::String((*s).into())).collect())
}

#[test]
fn trim_and_case_conversion_handle_unicode_and_empty_input() {
    for (expression, expected) in [
        (
            "String::trim(\" \\t\\r\\n\u{a0}\u{2003}café\u{2003} \\n\")",
            "café",
        ),
        ("String::trim(\" a b \" )", "a b"),
        ("String::trim(\" \\t\")", ""),
        (
            "String::lower(\"HELLO Café İ ΟΣ\")",
            "hello café i\u{307} ος",
        ),
        ("String::upper(\"Straße café σς\")", "STRASSE CAFÉ ΣΣ"),
        ("String::trim(\"\")", ""),
        ("String::lower(\"\")", ""),
        ("String::upper(\"\")", ""),
    ] {
        assert_eq!(
            evaluate(expression),
            Value::String(expected.into()),
            "{expression}"
        );
    }
}

#[test]
fn literal_split_and_join_preserve_empty_fields_and_unicode() {
    for (expression, expected) in [
        (r#"String::split("a,,b,", ",")"#, vec!["a", "", "b", ""]),
        (r#"String::split(",a", ",")"#, vec!["", "a"]),
        (r#"String::split("", ",")"#, vec![""]),
        (r#"String::split("abc", "missing")"#, vec!["abc"]),
        (
            r#"String::split("café🦀東京🦀", "🦀")"#,
            vec!["café", "東京", ""],
        ),
        (r#"String::split("a.*b.*", ".*")"#, vec!["a", "b", ""]),
        (r#"String::split("aaaaa", "aa")"#, vec!["", "", "a"]),
    ] {
        assert_eq!(evaluate(expression), list(&expected), "{expression}");
    }
    for (expression, expected) in [
        (r#"String::join([], ",")"#, ""),
        (r#"String::join(["a"], ",")"#, "a"),
        (r#"String::join(["", "a", ""], ",")"#, ",a,"),
        (r#"String::join(["é", "🦀"], "")"#, "é🦀"),
        (r#"String::join(["a", "b"], "🦀")"#, "a🦀b"),
        (r#"String::join(String::split("a,,b,", ","), ",")"#, "a,,b,"),
    ] {
        assert_eq!(
            evaluate(expression),
            Value::String(expected.into()),
            "{expression}"
        );
    }
}

#[test]
fn predicates_are_literal_case_sensitive_and_do_not_normalize() {
    for (expression, expected) in [
        (r#"String::contains("café🦀", "fé")"#, true),
        (r#"String::contains("café", "CAFÉ")"#, false),
        (r#"String::contains("café", "é")"#, false),
        (r#"String::contains("a.*b", ".*")"#, true),
        (r#"String::contains("abc", ".*")"#, false),
        (r#"String::startsWith("é🦀", "é")"#, true),
        (r#"String::startsWith("é🦀", "🦀")"#, false),
        (r#"String::endsWith("é🦀", "🦀")"#, true),
        (r#"String::endsWith("é🦀", "é")"#, false),
        (r#"String::contains("", "x")"#, false),
        (r#"String::startsWith("", "x")"#, false),
        (r#"String::endsWith("", "x")"#, false),
    ] {
        assert_eq!(evaluate(expression), Value::Bool(expected), "{expression}");
    }
    for name in ["contains", "startsWith", "endsWith"] {
        for input in ["", "abc"] {
            assert_eq!(
                evaluate(&format!("String::{name}(\"{input}\", \"\")")),
                Value::Bool(true)
            );
        }
    }
}

#[test]
fn replace_uses_all_nonoverlapping_literal_matches() {
    for (expression, expected) in [
        (r#"String::replace("a-a-a", "a", "x")"#, "x-x-x"),
        (r#"String::replace("aaaaa", "aa", "b")"#, "bba"),
        (r#"String::replace("é🦀é", "é", "東京")"#, "東京🦀東京"),
        (r#"String::replace("a.*b", ".*", "$")"#, "a$b"),
        (r#"String::replace("abc", "missing", "x")"#, "abc"),
        (r#"String::replace("abc", "b", "")"#, "ac"),
        (r#"String::replace("", "a", "b")"#, ""),
        (r#"String::replace("a", "a", "aa")"#, "aa"),
    ] {
        assert_eq!(
            evaluate(expression),
            Value::String(expected.into()),
            "{expression}"
        );
    }
}

#[test]
fn substring_counts_unicode_scalars_including_combining_marks() {
    for (expression, expected) in [
        (r#"String::substring("abc", 1)"#, "bc"),
        (r#"String::substring("abc", 0, 3)"#, "abc"),
        (r#"String::substring("é🦀éZ", 1, 3)"#, "🦀é"),
        (r#"String::substring("é🦀éZ", 3, 1)"#, "́"),
        (r#"String::substring("abc", 3)"#, ""),
        (r#"String::substring("abc", 3, 0)"#, ""),
        (r#"String::substring("abc", 1, 0)"#, ""),
        (r#"String::substring("", 0)"#, ""),
        (r#"String::substring("", 0, 0)"#, ""),
    ] {
        assert_eq!(
            evaluate(expression),
            Value::String(expected.into()),
            "{expression}"
        );
    }
}

#[test]
fn invalid_types_and_arity_are_catchable_without_coercion() {
    let cases = [
        "String::trim()",
        "String::trim(1)",
        "String::trim('a', 'b')",
        "String::lower(null)",
        "String::lower('a', 'b')",
        "String::upper([])",
        "String::upper()",
        "String::split('a')",
        "String::split('a', 1)",
        "String::split(1, ',')",
        "String::split('a', ',', 1)",
        "String::join('a', ',')",
        "String::join(['a', 1], ',')",
        "String::join(['a', null], ',')",
        "String::join([['nested']], ',')",
        "String::join(['a'], null)",
        "String::join([])",
        "String::contains('a', 1)",
        "String::startsWith(1, 'a')",
        "String::endsWith('a')",
        "String::replace('a', 'a')",
        "String::replace('a', 'a', 1)",
        "String::replace('a', 'a', 'b', 'c')",
        "String::substring('a')",
        "String::substring('a', '0')",
        "String::substring('a', 0.0)",
        "String::substring('a', 0, null)",
        "String::substring('a', 0, 1, 2)",
    ];
    let mut runtime = runtime(None);
    for expression in cases {
        let source = format!(
            "<?scr $caught = false; try {{ {expression}; }} catch (TypeError $e) {{ $caught = true; }}"
        );
        runtime.execute_source(&source, &mut Vec::new()).unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{expression}"
        );
    }
}

#[test]
fn invalid_ranges_are_catchable_and_do_not_change_inputs() {
    let cases = [
        r#"String::split($input, "")"#,
        r#"String::replace($input, "", "x")"#,
        "String::substring($input, -1)",
        "String::substring($input, 4)",
        "String::substring($input, 0, -1)",
        "String::substring($input, 2, 2)",
        "String::substring($input, 3, 1)",
        "String::substring($input, 9223372036854775807)",
        "String::substring($input, 1, 9223372036854775807)",
    ];
    let mut runtime = runtime(None);
    for expression in cases {
        let source = format!(
            "<?scr $input = 'abc'; $caught = false; try {{ {expression}; }} catch (RangeError $e) {{ $caught = true; }}"
        );
        runtime.execute_source(&source, &mut Vec::new()).unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{expression}"
        );
        assert_eq!(
            runtime.get_global("input"),
            Some(&Value::String("abc".into()))
        );
    }
}

#[test]
fn expanded_outputs_respect_runtime_budgets_and_failed_assignments_are_unchanged() {
    let slot = std::mem::size_of::<Value>();
    let mut runtime = runtime(Some(4 * slot));
    runtime
        .execute_source(
            "<?scr $input = 'aaaaaaaaaaaaaaaa'; $result = 'unchanged';",
            &mut Vec::new(),
        )
        .unwrap();
    for expression in [
        "String::replace($input, 'a', 'xxxxxxxxxxxxxxxx')",
        "String::split($input, 'a')",
    ] {
        let source = format!(
            "<?scr $caught = false; try {{ $result = {expression}; }} catch (ResourceLimitError $e) {{ $caught = true; }}"
        );
        runtime.execute_source(&source, &mut Vec::new()).unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{expression}"
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
        assert_eq!(
            runtime.get_global("input"),
            Some(&Value::String("aaaaaaaaaaaaaaaa".into()))
        );
    }
    runtime
        .execute_source("<?scr $result = String::upper('ok');", &mut Vec::new())
        .unwrap();
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("OK".into()))
    );
}
