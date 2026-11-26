use scr_runtime::{ContextOptions, MapKey, Runtime, RuntimeOptions, Value};

fn runtime() -> Runtime {
    let mut context = ContextOptions::default();
    context.permissions.allowed.clear();
    let mut rt = Runtime::with_options(RuntimeOptions {
        context,
        ..Default::default()
    });
    scr_stdlib::register(&mut rt).unwrap();
    rt
}
fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (MapKey::String(k.into()), v))
            .collect(),
    )
}
fn text(s: &str) -> Value {
    Value::String(s.into())
}
fn parse(rt: &mut Runtime, format: &str, source: &str, options: &str) -> Value {
    rt.set_global("input", text(source)).unwrap();
    rt.execute_source(
        &format!("$result={format}::parse($input{options});"),
        &mut Vec::new(),
    )
    .unwrap();
    rt.get_global("result").unwrap().clone()
}
fn failure(rt: &mut Runtime, format: &str, source: &str, options: &str, kind: &str) -> String {
    rt.set_global("input", text(source)).unwrap();
    rt.set_global("result", Value::Int(42)).unwrap();
    let e = rt
        .execute_source(
            &format!("$result={format}::parse($input{options});"),
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(
        e.message.starts_with(&format!("{kind}:")),
        "{format} {source:?}: {e}"
    );
    assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
    e.message
}
#[test]
fn toml_converts_types_arrays_tables_and_lexical_keys_with_explicit_newline_bytes() {
    let mut rt = runtime();
    let source = "z = true\nnumber = -9223372036854775808\nratio = 1.0\nitems = [1, '世界', false]\n[database]\nport = 5432\n[[workers]]\nname = 'a'\n[[workers]]\nname = 'b'\n";
    let expected = map(vec![
        ("database", map(vec![("port", Value::Int(5432))])),
        (
            "items",
            Value::list(vec![Value::Int(1), text("世界"), Value::Bool(false)]),
        ),
        ("number", Value::Int(i64::MIN)),
        ("ratio", Value::Float(1.0)),
        (
            "workers",
            Value::list(vec![
                map(vec![("name", text("a"))]),
                map(vec![("name", text("b"))]),
            ]),
        ),
        ("z", Value::Bool(true)),
    ]);
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        assert_eq!(parse(&mut rt, "Toml", &source, ""), expected);
    }
    assert_eq!(
        parse(&mut rt, "Toml", "A=1\na=2\n'1'=3", ""),
        map(vec![
            ("1", Value::Int(3)),
            ("A", Value::Int(1)),
            ("a", Value::Int(2))
        ])
    );
    assert_eq!(
        parse(&mut rt, "Toml", "date='1979-05-27T07:32:00Z'", ""),
        map(vec![("date", text("1979-05-27T07:32:00Z"))])
    );
}
#[test]
fn toml_rejects_duplicate_definitions_dates_nonfinite_and_overflow() {
    let mut rt = runtime();
    for source in [
        "a=1\na=2",
        "[a]\n[a]",
        "a=1\na.b=2",
        "x=9223372036854775808",
        "x=inf",
        "x=-nan",
        "x=1e999",
        "x=1979-05-27",
        "x=07:32:00",
        "x=1979-05-27T07:32:00Z",
        "x = [",
        "x = null",
    ] {
        failure(&mut rt, "Toml", source, "", "TomlError");
    }
}
#[test]
fn yaml_converts_core_scalars_without_yaml_11_boolean_or_timestamp_coercion() {
    let mut rt = runtime();
    let value = parse(
        &mut rt,
        "Yaml",
        "[null, ~, TRUE, False, 0, +12, -9223372036854775808, 0o17, 0xFF, 01, 1.0, .5, 1e3, yes, OFF, 2026-11-25, 1:20, 1_000, 'true', \"42\"]",
        "",
    );
    assert_eq!(
        value,
        Value::list(vec![
            Value::Null,
            Value::Null,
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(12),
            Value::Int(i64::MIN),
            Value::Int(15),
            Value::Int(255),
            Value::Int(1),
            Value::Float(1.0),
            Value::Float(0.5),
            Value::Float(1000.0),
            text("yes"),
            text("OFF"),
            text("2026-11-25"),
            text("1:20"),
            text("1_000"),
            text("true"),
            text("42")
        ])
    );
    assert_eq!(parse(&mut rt, "Yaml", "-0.0", ""), Value::Float(-0.0));
    for source in [
        "9223372036854775808",
        "-9223372036854775809",
        "0x8000000000000000",
        "0o1000000000000000000000",
        ".inf",
        ".NaN",
        "-.INF",
        "1e999",
    ] {
        failure(&mut rt, "Yaml", source, "", "YamlError");
    }
}
#[test]
fn yaml_preserves_mapping_and_sequence_order_block_strings_and_unicode_with_lf_crlf() {
    let mut rt = runtime();
    let source = "z: 世界\na:\n  - {name: café, enabled: true}\n  - []\nblock: |\n  first\n  second\n'1': integer key quoted\n'': empty\n";
    let expected = map(vec![
        ("z", text("世界")),
        (
            "a",
            Value::list(vec![
                map(vec![("name", text("café")), ("enabled", Value::Bool(true))]),
                Value::list(vec![]),
            ]),
        ),
        ("block", text("first\nsecond\n")),
        ("1", text("integer key quoted")),
        ("", text("empty")),
    ]);
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        assert_eq!(parse(&mut rt, "Yaml", &source, ""), expected);
    }
}
#[test]
fn yaml_rejects_alias_expansion_tags_merges_duplicates_and_nonstring_keys() {
    let mut rt = runtime();
    for source in [
        "a: &x [1, 2]\nb: *x",
        "&x [*x]",
        "&x hello",
        "!!str true",
        "!!map {}",
        "!custom value",
        "a: 1\na: 2",
        "a: 1\n'a': 2",
        "<<: {}",
        "'<<': {}",
        "1: a",
        "true: a",
        "null: a",
        "? [a, b]\n: c",
        "---\na\n---\nb",
        "a: [1,",
        "[a, b",
        "a:\n  b: 1\n c: 2",
        "*unknown",
    ] {
        failure(&mut rt, "Yaml", source, "", "YamlError");
    }
    // An alias bomb is refused at its first anchored node, not expanded.
    failure(
        &mut rt,
        "Yaml",
        "a: &a ['x','x']\nb: &b [*a,*a,*a]\nc: [*b,*b,*b]",
        "",
        "YamlError",
    );
}
#[test]
fn ini_preserves_literal_strings_sections_case_and_empty_values_in_both_newline_forms() {
    let mut rt = runtime();
    let source = " ; comment\n # comment\nroot = true\nroot2 = 0123\n[ service.api ]\n name = café 世界 \n port = 443\nPort = 8443\nempty=\nquoted='literal'\npath=C:\\tmp\\a\nmore=a=b # literal ; too\ninterpolation=${HOME}\n[other]\nname=second\n";
    let expected = map(vec![
        (
            "",
            map(vec![("root", text("true")), ("root2", text("0123"))]),
        ),
        (
            "service.api",
            map(vec![
                ("name", text("café 世界")),
                ("port", text("443")),
                ("Port", text("8443")),
                ("empty", text("")),
                ("quoted", text("'literal'")),
                ("path", text("C:\\tmp\\a")),
                ("more", text("a=b # literal ; too")),
                ("interpolation", text("${HOME}")),
            ]),
        ),
        ("other", map(vec![("name", text("second"))])),
    ]);
    for source in [source.to_owned(), source.replace('\n', "\r\n")] {
        assert_eq!(parse(&mut rt, "Ini", &source, ""), expected);
    }
}
#[test]
fn ini_rejects_ambiguous_duplicates_malformed_sections_controls_and_bare_cr() {
    let mut rt = runtime();
    for source in [
        "a=1\na=2",
        "[s]\na=1\na=2",
        "[s]\n[s]",
        "[]",
        "[ ]",
        "[s",
        "[s] trailing",
        "[a[b]]",
        "=value",
        "key:value",
        "a=1\rnext=2",
        "a=1\r",
        "a=\0",
        "a=\u{7f}",
        "a=\u{85}",
        "[s]\na=one\n continuation",
    ] {
        failure(&mut rt, "Ini", source, "", "IniError");
    }
    assert_eq!(
        parse(&mut rt, "Ini", "[S]\nx=1\n[s]\nx=2", ""),
        map(vec![
            ("", map(vec![])),
            ("S", map(vec![("x", text("1"))])),
            ("s", map(vec![("x", text("2"))]))
        ])
    );
}
#[test]
fn empty_documents_bom_and_zero_input_limits_have_explicit_results() {
    let mut rt = runtime();
    for (format, expected) in [
        ("Toml", map(vec![])),
        ("Yaml", Value::Null),
        ("Ini", map(vec![("", map(vec![]))])),
    ] {
        assert_eq!(
            parse(&mut rt, format, "", ",['maxInputBytes'=>0]"),
            expected
        );
        assert_eq!(parse(&mut rt, format, "\u{feff}", ""), expected);
        failure(
            &mut rt,
            format,
            "\u{feff}",
            ",['maxInputBytes'=>2]",
            "ResourceLimitError",
        );
    }
    for source in ["# comment\n", "---\n", "---\n...\n"] {
        assert_eq!(parse(&mut rt, "Yaml", source, ""), Value::Null);
    }
}
#[test]
fn input_values_and_depth_limits_accept_exact_boundaries_and_reject_one_more() {
    let mut rt = runtime();
    for (format, source, values, depth) in [
        ("Toml", "a=['é']", 3, 2),
        ("Yaml", "a: [é]", 3, 2),
        ("Ini", "a=é", 3, 2),
    ] {
        parse(
            &mut rt,
            format,
            source,
            &format!(
                ",['maxInputBytes'=>{},'maxValues'=>{values},'maxDepth'=>{depth}]",
                source.len()
            ),
        );
        for options in [
            format!(",['maxInputBytes'=>{}]", source.len() - 1),
            format!(",['maxValues'=>{}]", values - 1),
            format!(",['maxDepth'=>{}]", depth - 1),
            ",['maxOutputBytes'=>0]".into(),
            ",['maxValues'=>0]".into(),
        ] {
            failure(&mut rt, format, source, &options, "ResourceLimitError");
        }
    }
    assert_eq!(
        parse(&mut rt, "Yaml", "42", ",['maxDepth'=>0,'maxValues'=>1]"),
        Value::Int(42)
    );
    assert_eq!(
        parse(&mut rt, "Toml", "", ",['maxDepth'=>0,'maxValues'=>1]"),
        map(vec![])
    );
    for depth in [64, 65] {
        let yaml = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        if depth == 64 {
            parse(&mut rt, "Yaml", &yaml, "");
        } else {
            failure(&mut rt, "Yaml", &yaml, "", "ResourceLimitError");
        }
        let toml = format!("a={}0{}", "[".repeat(depth - 1), "]".repeat(depth - 1));
        if depth == 64 {
            parse(&mut rt, "Toml", &toml, "");
        } else {
            failure(&mut rt, "Toml", &toml, "", "ResourceLimitError");
        }
    }
}
#[test]
fn hard_input_and_value_limits_are_bounded_for_tiny_and_large_records() {
    let mut rt = runtime();
    for (format, comment) in [("Toml", "#"), ("Yaml", "#"), ("Ini", ";")] {
        let exact = format!("{comment}{}", "x".repeat(1048575));
        parse(&mut rt, format, &exact, "");
        failure(&mut rt, format, &(exact + "x"), "", "ResourceLimitError");
    }
    let yaml = format!("[{}]", vec!["0"; 9999].join(","));
    parse(&mut rt, "Yaml", &yaml, "");
    let yaml = format!("[{}]", vec!["0"; 10000].join(","));
    failure(&mut rt, "Yaml", &yaml, "", "ResourceLimitError");
    // Table-root + array + 9998 elements reaches exactly 10000 Values.
    let toml = format!("a=[{}]", vec!["0"; 9998].join(","));
    parse(&mut rt, "Toml", &toml, "");
    let toml = format!("a=[{}]", vec!["0"; 9999].join(","));
    failure(&mut rt, "Toml", &toml, "", "ResourceLimitError");
    let ini = (0..9998).map(|n| format!("k{n}=0\n")).collect::<String>();
    parse(&mut rt, "Ini", &ini, "");
    failure(&mut rt, "Ini", &(ini + "last=0"), "", "ResourceLimitError");
}
#[test]
fn options_and_signatures_are_strict_and_failures_remain_catchable() {
    let mut rt = runtime();
    for format in ["Toml", "Yaml", "Ini"] {
        for options in [
            ",['unknown'=>1]",
            ",['maxDepth'=>'1']",
            ",['maxValues'=>true]",
            ",[1=>2]",
            ",null",
        ] {
            failure(&mut rt, format, "", options, "TypeError");
        }
        for options in [
            ",['maxDepth'=>-1]",
            ",['maxDepth'=>65]",
            ",['maxValues'=>10001]",
            ",['maxInputBytes'=>1048577]",
            ",['maxOutputBytes'=>16777217]",
        ] {
            failure(&mut rt, format, "", options, "RangeError");
        }
        for argument in ["1", "true", "[]", "Bytes::encodeUtf8('')"] {
            let e = rt
                .execute_source(&format!("{format}::parse({argument});"), &mut Vec::new())
                .unwrap_err();
            assert!(e.message.starts_with("TypeError:"));
        }
    }
    rt.execute_source(
        "$caught=false; try { Yaml::parse('a: ['); } catch (YamlError $e) { $caught=true; }",
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(rt.get_global("caught"), Some(&Value::Bool(true)));
}
#[test]
fn syntax_diagnostics_report_positions_without_echoing_input_secrets() {
    let mut rt = runtime();
    for (format, source) in [
        ("Toml", "a=1\npassword=SUPER_SECRET"),
        ("Yaml", "a: ok\npassword: [SUPER_SECRET"),
        ("Ini", "a=1\nSUPER_SECRET"),
    ] {
        let message = failure(&mut rt, format, source, "", &format!("{format}Error"));
        assert!(message.contains("line "), "{message}");
        assert!(message.contains("column "), "{message}");
        assert!(!message.contains("SUPER_SECRET"), "{message}");
    }
}

#[test]
fn native_failures_keep_call_spans_and_function_traces() {
    let mut rt = runtime();
    let source = "function readConfig(): any {\n    return Yaml::parse('a: [');\n}\nreadConfig();";
    let error = rt.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with("YamlError:"));
    assert_eq!(error.span.start, source.find("Yaml::parse").unwrap());
    assert!(
        error
            .trace
            .iter()
            .any(|frame| frame.function == "readConfig")
    );
}
