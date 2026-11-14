use scr_runtime::{Runtime, RuntimeOptions, Value};

fn runtime(limit: Option<usize>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime
        .execute_source(&format!("<?scr {source}"), &mut Vec::new())
        .unwrap();
}
fn value(expression: &str) -> Value {
    let mut runtime = runtime(None);
    run(&mut runtime, &format!("$result = {expression};"));
    runtime.get_global("result").unwrap().clone()
}
fn text(s: &str) -> Value {
    Value::String(s.into())
}
fn pair(k: &str, v: &str) -> Value {
    Value::List(vec![text(k), text(v)])
}

#[test]
fn components_encode_utf8_and_delimiters_and_decode_exactly_once() {
    assert_eq!(
        value(r#"Url::encodeComponent("café & tea/+%?#=😀")"#),
        text("caf%C3%A9%20%26%20tea%2F%2B%25%3F%23%3D%F0%9F%98%80")
    );
    assert_eq!(
        value(r#"Url::encodeComponent("AZaz09-._~*")"#),
        text("AZaz09-._~%2A")
    );
    assert_eq!(
        value(r#"Url::decodeComponent("caf%c3%a9+%2526%00")"#),
        text("café+%26\0")
    );
    assert_eq!(
        value(r#"Url::decodeComponent(Url::encodeComponent("é /+%😀"))"#),
        text("é /+%😀")
    );
    assert_eq!(value(r#"Url::encodeComponent("")"#), text(""));
}
#[test]
fn queries_preserve_order_duplicates_empty_fields_and_explicit_string_types() {
    assert_eq!(
        value(r#"Url::buildQuery(["q" => "café & tea", "page" => str(2)])"#),
        text("q=caf%C3%A9%20%26%20tea&page=2")
    );
    assert_eq!(
        value(r#"Url::buildQuery([["tag", "rust"], ["tag", "scripting"], ["", ""]])"#),
        text("tag=rust&tag=scripting&=")
    );
    assert_eq!(
        value(
            r#"Url::parseQuery("?tag=rust&&tag=scripting&flag&=x&a=b=c&escaped=%26%3D&plus=+&")"#
        ),
        Value::List(vec![
            pair("tag", "rust"),
            pair("tag", "scripting"),
            pair("flag", ""),
            pair("", "x"),
            pair("a", "b=c"),
            pair("escaped", "&="),
            pair("plus", "+")
        ])
    );
    for expression in [r#"Url::parseQuery("")"#, r#"Url::parseQuery("?&&")"#] {
        assert_eq!(value(expression), Value::List(vec![]));
    }
    assert_eq!(value("Url::buildQuery([])"), text(""));
    assert_eq!(
        value(r#"Url::parseQuery(Url::buildQuery([["x", "1"], ["x", "é &+"]]))"#),
        Value::List(vec![pair("x", "1"), pair("x", "é &+")])
    );
}
#[test]
fn forms_use_the_form_safe_set_and_treat_plus_as_space_before_percent_decoding() {
    assert_eq!(
        value(r#"Form::encode(["name" => "Jane Doe", "message" => "A+B", "safe" => "*~"] )"#),
        text("name=Jane+Doe&message=A%2BB&safe=*%7E")
    );
    assert_eq!(
        value(r#"Form::decode("q=caf%C3%A9+tea&literal=%2B&literal=+&?x=y")"#),
        Value::List(vec![
            pair("q", "café tea"),
            pair("literal", "+"),
            pair("literal", " "),
            pair("?x", "y")
        ])
    );
    assert_eq!(
        value(r#"Form::decode(Form::encode([["x", "1"], ["x", "é +"]]))"#),
        Value::List(vec![pair("x", "1"), pair("x", "é +")])
    );
    assert_eq!(value("Form::encode([])"), text(""));
    assert_eq!(value(r#"Form::decode("&&")"#), Value::List(vec![]));
}
#[test]
fn invalid_encodings_and_parameter_shapes_preserve_failed_assignments() {
    for decoder in ["Url::decodeComponent", "Url::parseQuery", "Form::decode"] {
        for invalid in ["%", "%0", "%GG", "%FF", "%C0%AF", "%ED%A0%80", "%E2%82"] {
            let mut runtime = runtime(None);
            run(&mut runtime, "$result = 42;");
            let error = runtime
                .execute_source(
                    &format!("<?scr $result = {decoder}(\"{invalid}\");"),
                    &mut Vec::new(),
                )
                .unwrap_err();
            assert!(error.message.starts_with("EncodingError:"), "{error}");
            assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
        }
    }
    for encoder in ["Url::buildQuery", "Form::encode"] {
        for invalid in [
            "null",
            "1",
            r#"[1 => "x"]"#,
            r#"["x" => 1]"#,
            r#"[["x"]]"#,
            r#"[["x", "y", "z"]]"#,
            r#"[["x", null]]"#,
            r#"["x"]"#,
        ] {
            let mut runtime = runtime(None);
            run(&mut runtime, "$result = 42;");
            let error = runtime
                .execute_source(
                    &format!("<?scr $result = {encoder}({invalid});"),
                    &mut Vec::new(),
                )
                .unwrap_err();
            assert!(error.message.starts_with("TypeError:"), "{error}");
            assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
        }
    }
}
#[test]
fn expanded_results_obey_limits_and_errors_are_catchable_without_mutation() {
    let mut runtime = runtime(Some(256));
    runtime.set_global("input", text(&"&".repeat(100))).unwrap();
    run(
        &mut runtime,
        r#"$result = 42; $caught = false;
        try { $result = Url::encodeComponent($input); }
        catch (ResourceLimitError $error) { $caught = true; }
    "#,
    );
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    assert_eq!(runtime.get_global("input"), Some(&text(&"&".repeat(100))));
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
}
