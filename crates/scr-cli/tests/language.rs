use scr_diagnostics::Diagnostic;

fn run(source: &str) -> Result<String, Diagnostic> {
    let mut output = Vec::new();
    scr_cli::pipeline::run(source, &mut output)?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn examples_execute() {
    assert_eq!(
        run(include_str!("../../../examples/hello.scr")).unwrap(),
        "Hello, Scripta!\n"
    );
    assert_eq!(
        run(include_str!("../../../examples/variables.scr")).unwrap(),
        "Checking Production...\nDisk usage: 85%\n"
    );
}

#[test]
fn explicit_conversions_and_strict_equality() {
    let source = r#"<?scr
        $n = 85;
        echo("usage: " . str($n) . "%\n");
        echo str(1 === 1.0) . "," . str(1 === "1") . ",";
        echo str(true === 1) . "," . str(null === null) . ",";
        echo str("a" . "b" === "ab");
        ?>
    "#;
    assert_eq!(
        run(source).unwrap(),
        "usage: 85%\nfalse,false,false,true,true"
    );
}

#[test]
fn parser_rejects_legacy_equality_with_targeted_diagnostics() {
    for operator in ["==", "!="] {
        let source = format!("<?scr echo str(1 {operator} 1);");
        // Check enforces the same parser policy as run.
        assert!(scr_cli::pipeline::check(&source).is_err());
        let tokens = scr_lexer::lex(&source).unwrap();
        let error = scr_parser::parse(&tokens).unwrap_err();
        assert!(
            error
                .message
                .contains("unsupported legacy equality operator")
        );
        assert!(error.message.contains("=== or !=="));
        assert_eq!(&source[error.span.start..error.span.end], operator);
    }
}

#[test]
fn reassignment_and_unicode_strings() {
    assert_eq!(
        run("<?scr $v = 1; $v = \"世界\"; echo $v;").unwrap(),
        "世界"
    );
}

#[test]
fn invalid_programs_report_errors() {
    for (source, message) in [
        ("echo \"hi\";", "expected opening tag"),
        ("<?script", "expected opening tag"),
        ("<?scr echo \"hi\"", "expected Semicolon"),
        ("<?scr echo \"open", "unterminated string"),
        ("<?scr echo $missing;", "undefined variable"),
        ("<?scr echo \"usage: \" . 85;", "concatenation requires"),
        ("<?scr echo 85;", "echo requires a string"),
        ("<?scr echo str();", "exactly one argument"),
        ("<?scr echo str(1, 2);", "exactly one argument"),
        ("<?scr unknown();", "undefined function"),
        ("<?scr echo (", "expected expression"),
        ("<?scr ?> trailing", "unexpected content"),
        (
            "<?scr $x = 9223372036854775808;",
            "integer literal out of range",
        ),
    ] {
        let error = run(source).unwrap_err();
        assert!(error.message.contains(message), "{source}: {error}");
        assert!(error.span.end <= source.len());
        assert!(source.is_char_boundary(error.span.start));
        assert!(source.is_char_boundary(error.span.end));
    }
}

#[test]
fn diagnostics_count_unicode_columns() {
    let source = "<?scr\necho \"é\" . 1;";
    let error = run(source).unwrap_err();
    let rendered = error.render("test.scr", source);
    assert!(rendered.contains("test.scr:2:6"));
    assert!(rendered.contains("error[E0401]: concatenation requires two strings"));
    assert!(rendered.contains("2 | echo \"*\" . 1;"));
}

#[test]
fn parser_rejects_invalid_token_streams() {
    assert!(scr_parser::parse(&[]).is_err());
    let mut tokens = scr_lexer::lex("<?scr").unwrap();
    tokens.push(tokens[0].clone());
    assert!(scr_parser::parse(&tokens).is_err());
}

#[test]
fn io_failure_is_a_runtime_error() {
    struct FailingWriter;
    impl std::io::Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("test output failure"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let tokens = scr_lexer::lex("<?scr echo \"hello\";").unwrap();
    let program = scr_parser::parse(&tokens).unwrap();
    let error = scr_runtime::Runtime::new()
        .execute(&program, &mut FailingWriter)
        .unwrap_err();
    assert!(error.message.contains("output failed"));
}

#[test]
fn echo_forms_have_identical_output_and_type_rules() {
    for value in [r#""é\n""#, r#""Count: " . str(10)"#, r#"str(1 === 1.0)"#] {
        assert_eq!(
            run(&format!("<?scr echo {value};")),
            run(&format!("<?scr echo({value});"))
        );
    }
    for value in ["1", "null", "true"] {
        let plain = run(&format!("<?scr echo {value};")).unwrap_err();
        let grouped = run(&format!("<?scr echo({value});")).unwrap_err();
        assert_eq!(plain.message, grouped.message);
    }
}

#[test]
fn complete_statements_execute() {
    let source = include_str!("../../../tests/fixtures/statements.scr");
    scr_cli::pipeline::check(source).unwrap();
    assert_eq!(run(source).unwrap(), "Starting...\nSuccess\n1\n2\n3\n");
}

#[test]
fn explicit_conversions_execute_through_the_registered_standard_library() {
    let source = r#"<?scr
        $number = float(int("42")) + 0.5;
        if (bool("true")) echo str($number) . "," . str(bool(0)) . "," . str(int(true));
    "#;
    assert_eq!(run(source).unwrap(), "42.5,false,1");
    for source in ["<?scr int(1.5);", "<?scr float(\"NaN\");", "<?scr bool(2);"] {
        let error = scr_cli::pipeline::run(source, &mut Vec::new()).unwrap_err();
        assert!(error.message.contains("invalid conversion"));
        assert_eq!(error.trace.len(), 2);
    }
}

#[test]
fn syntax_errors_prevent_earlier_output_and_exit_statements_from_running() {
    let mut bytes = Vec::new();
    assert!(scr_cli::pipeline::run("<?scr echo \"before\"; exit(0); if (", &mut bytes).is_err());
    assert!(bytes.is_empty());
}

#[test]
fn standard_library_type_len_and_sleep_are_registered() {
    assert_eq!(
        run(r#"<?scr
        echo type(null) . "," . type(true) . "," . type(1) . "," . type(1.0) . ",";
        echo type("x") . "," . type([]) . "," . type(["a" => 1]) . "," . type(str);
        echo ":" . str(len("é🦀")) . "," . str(len([1, 2, 3])) . "," . str(len(["a" => 1]));
        echo ":" . type(sleep(0));
    "#)
        .unwrap(),
        "null,bool,int,float,string,list,map,callable:2,3,1:null"
    );
}

#[test]
fn all_builtin_argument_errors_are_catchable_type_errors() {
    for expression in [
        "str()",
        "str(1, 2)",
        "str([])",
        "int()",
        "int(1.5)",
        "int(null)",
        "float()",
        "float(\"NaN\")",
        "bool()",
        "bool(2)",
        "type()",
        "type(1, 2)",
        "len()",
        "len(null)",
        "len(true)",
        "len(1.0)",
        "len(1, 2)",
        "sleep()",
        "sleep(-1)",
        "sleep(1.0)",
        "sleep(\"0\")",
        "sleep(0, 0)",
    ] {
        let source =
            format!("<?scr try {{ {expression}; }} catch (TypeError $e) {{ echo \"caught\"; }}");
        assert_eq!(run(&source).unwrap(), "caught", "{expression}");
    }
}

#[test]
fn len_type_and_native_errors_preserve_call_sites() {
    let source = "<?scr function f() { return len(42); } f();";
    let error = scr_cli::pipeline::run(source, &mut Vec::new()).unwrap_err();
    assert_eq!(&source[error.span.start..error.span.end], "len(42)");
    assert_eq!(
        error
            .trace
            .iter()
            .map(|frame| frame.function.as_str())
            .collect::<Vec<_>>(),
        ["<main>", "f", "len"]
    );
}
