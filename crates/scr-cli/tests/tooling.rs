use proptest::prelude::*;
use scr_cli::{formatter, pipeline};
use std::process::Command;

#[test]
fn formatter_snapshots_preserve_tokens_and_are_idempotent() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/formatter");
    for name in [
        "basic",
        "comments",
        "loops",
        "literals",
        "branches",
        "functions",
    ] {
        let source = std::fs::read_to_string(root.join(format!("{name}.input.scripta"))).unwrap();
        let expected =
            std::fs::read_to_string(root.join(format!("{name}.expected.scripta"))).unwrap();
        let formatted = formatter::format(&source).unwrap();
        assert_eq!(formatted, expected, "{name}");
        if name != "branches" {
            let mut before = Vec::new();
            let mut after = Vec::new();
            assert_eq!(
                pipeline::run(&source, &mut before).unwrap(),
                pipeline::run(&formatted, &mut after).unwrap()
            );
            assert_eq!(before, after, "execution changed for {name}");
        }
        assert_eq!(formatter::format(&formatted).unwrap(), formatted, "{name}");
    }
}
#[test]
fn check_visits_anonymous_functions_in_defaults_and_nested_expressions() {
    for source in [
        "<?scr $f = function ($x, $x) {};",
        "<?scr function f($callback = function ($x, $x) {}) {}",
        "<?scr $a = [function ($x, $x) {}];",
        "<?scr $a = ['key' => function ($x, $x) {}];",
        "<?scr function f() { return function ($x, $x) {}; }",
        "<?scr if ((function ($x, $x) {})()) {}",
        "<?scr for ($f = function ($x, $x) {}; false;) {}",
    ] {
        let errors = pipeline::check(source).unwrap_err();
        assert_eq!(
            errors.iter().filter(|e| e.code == "E0302").count(),
            1,
            "{source}: {errors:?}"
        );
    }
    let errors = pipeline::check(
        "<?scr $f = function () { function duplicate() {} function duplicate() {} };",
    )
    .unwrap_err();
    assert!(errors.iter().any(|e| e.code == "E0301"));
    pipeline::check("<?scr function f($value = unknown()) {} $callback = function (...$items: Custom): Custom { return missing(); };").unwrap();
    let source = "<?scr $f=function($callback=function($n=1){if($n>0)return $n;return 0;}){return $callback();};echo str($f());";
    let formatted = formatter::format(source).unwrap();
    assert_eq!(formatter::format(&formatted).unwrap(), formatted);
    let mut output = Vec::new();
    pipeline::run(&formatted, &mut output).unwrap();
    assert_eq!(output, b"1");
}

#[test]
fn finally_survives_check_format_and_execution_and_validates_nested_declarations() {
    let source = "<?scr try{try{echo 'a';}finally{if(true)echo 'b';}}catch($e){echo 'bad';}finally{echo 'c';}";
    pipeline::check(source).unwrap();
    let formatted = formatter::format(source).unwrap();
    assert!(formatted.contains("} finally {"));
    assert_eq!(formatter::format(&formatted).unwrap(), formatted);
    for source in [source, &formatted] {
        let mut output = Vec::new();
        pipeline::run(source, &mut output).unwrap();
        assert_eq!(output, b"abc");
    }
    for source in [
        "<?scr try {} finally { function duplicate() {} function duplicate() {} }",
        "<?scr try {} finally { $f = function ($x, $x) {}; }",
    ] {
        assert!(pipeline::check(source).is_err());
    }
    for source in [
        "<?scr try {} finally { return; }",
        "<?scr try {} finally { break; }",
        "<?scr finally {}",
    ] {
        assert!(pipeline::check(source).is_err());
    }
}

#[test]
fn append_survives_check_format_and_execution() {
    let source = "<?scr $items=[];$items[]=1;$items[len($items)]=2;$groups=[[]];$groups[0][]=3;echo Json::encode([$items,$groups]);";
    pipeline::check(source).unwrap();
    let formatted = formatter::format(source).unwrap();
    assert_eq!(formatter::format(&formatted).unwrap(), formatted);
    for script in [source, formatted.as_str()] {
        let mut output = Vec::new();
        pipeline::run(script, &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "[[1,2],[[3]]]");
    }
    for target in ["$items[]", "$items[] += 1", "$items[][0] = 1"] {
        assert!(pipeline::check(&format!("<?scr {target};")).is_err());
    }
}

#[test]
fn check_validates_declarations_control_flow_and_legacy_syntax_without_execution() {
    for (source, code) in [
        ("<?scr function f() {} function f() {}", "E0301"),
        ("<?scr function f($x, $x) {}", "E0302"),
        (
            "<?scr function f() { function g() {} function g() {} }",
            "E0301",
        ),
        ("<?scr break;", "E0201"),
        ("<?scr return;", "E0201"),
        ("<?scr while (true) { function f() { continue; } }", "E0201"),
        ("<?scr $x == 1;", "E0202"),
        ("<?scr $x++;", "E0101"),
        ("<?scr $x <> 1;", "E0101"),
        ("<?scr $x and true;", "E0101"),
        ("<?php echo 'wrong';", "E0101"),
        ("<?scr ?> echo 'after';", "E0101"),
    ] {
        let errors = pipeline::check(source).unwrap_err();
        assert!(
            errors.iter().any(|e| e.code == code),
            "{source}: {errors:?}"
        );
    }
    // Unknown values, calls and annotations are dynamic, not a static type claim.
    pipeline::check("<?scr function f($x: Custom): Custom { return unknown($x); } echo $unknown; require 'missing.scripta'; exit(19);").unwrap();
    pipeline::check("<?scr { function f() {} } { function f() {} }").unwrap();
}
#[test]
fn fmt_cli_check_does_not_write_and_invalid_sources_are_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.scripta");
    let original = "<?scr echo   'hello';";
    std::fs::write(&file, original).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("fmt")
        .arg("--check")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    assert!(String::from_utf8(result.stderr).unwrap().contains("E0502"));
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("fmt")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "<?scr\necho 'hello';\n"
    );
    assert!(
        Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg("fmt")
            .arg(&file)
            .arg("--check")
            .status()
            .unwrap()
            .success()
    );
    let malformed = "<?scr if (true { echo 'bad';";
    std::fs::write(&file, malformed).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("fmt")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), malformed);
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("error[E0201]")
    );
}
#[test]
fn check_cli_never_executes_and_errors_have_codes_excerpts_and_help() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.scripta");
    std::fs::write(
        &file,
        "<?scr File::writeText('should-not-exist', 'bad'); require 'missing.scripta'; exit(19);",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .current_dir(dir.path())
        .arg("check")
        .arg(&file)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(!dir.path().join("should-not-exist").exists());
    std::fs::write(&file, "<?scr\nif ($disk > ) { echo 'bad'; }").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("check")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("error[E0201]"));
    assert!(stderr.contains("test.scripta:2:13"));
    assert!(stderr.contains("2 | if ($disk > )"));
    assert!(stderr.contains('^') && stderr.contains("help: provide a value after '>'"));
    for args in [
        vec!["fmt"],
        vec!["fmt", "--wat", "x"],
        vec!["fmt", "x", "y"],
    ] {
        assert_eq!(
            Command::new(env!("CARGO_BIN_EXE_scripta"))
                .args(args)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
}
#[test]
fn malformed_and_deep_input_is_rejected_without_panics() {
    for source in [
        format!("<?scr {}1{};", "(".repeat(10000), ")".repeat(10000)),
        format!("<?scr {}1;", "!".repeat(10000)),
        format!("<?scr {}{}", "{".repeat(10000), "}".repeat(10000)),
        format!("<?scr {};", vec!["1"; 10000].join(" + ")),
        format!("<?scr {}echo 'x';", "if (true) ".repeat(10000)),
    ] {
        assert!(pipeline::check(&source).is_err());
        assert!(formatter::format(&source).is_err());
    }
    for source in [
        "",
        "<?scr $",
        "<?scr if () {}",
        "<?scr function f( {",
        "<?scr try {}",
        "<?scr [1 => ];",
        "<?scr echo \\\"",
        "<?scr /*",
    ] {
        assert!(pipeline::check(source).is_err(), "{source}");
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn formatter_is_idempotent_for_generated_valid_scripts(a in -1000i32..1000, b in 0i32..1000, text in "[a-zA-Z0-9 ]{0,60}", comment in "[a-zA-Z ]{0,30}") {
        let source = format!("<?scr // {comment}\n$x={a}+{b};if($x>0){{echo\"{text}\";}}else echo 'fallback'; ?>");
        let formatted = formatter::format(&source).unwrap();
        prop_assert_eq!(formatter::format(&formatted).unwrap(), formatted.clone());
        prop_assert!(formatted.contains(&format!("// {comment}")), "missing preserved comment");
        let before = scr_lexer::lex(&source).unwrap(); let after = scr_lexer::lex(&formatted).unwrap();
        prop_assert!(before.iter().map(|t| &t.kind).eq(after.iter().map(|t| &t.kind)));
    }
}

#[cfg(unix)]
#[test]
fn formatter_saves_through_symlinks_and_retains_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.scripta");
    let link = dir.path().join("link.scripta");
    std::fs::write(&target, "<?scr echo 'x';").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    formatter::write_file(&link, "<?scr\necho 'x';\n").unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "<?scr\necho 'x';\n"
    );
}

#[test]
fn cli_resource_budgets_stop_execution_and_redact_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("budget.scripta");
    std::fs::write(&file, "<?scr while (true) {}").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(&file)
        .env("SCR_MAX_STEPS", "20")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("instruction limit")
    );
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(&file)
        .env("SCR_MAX_SOURCE_BYTES", "8")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(&file)
        .env("SCR_MAX_STEPS", "invalid")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    std::fs::write(&file, "<?scr throw Env::get(\"TEST_PASSWORD\");").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(&file)
        .env("TEST_PASSWORD", "sensitive-value-123")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(
        !String::from_utf8(result.stderr)
            .unwrap()
            .contains("sensitive-value-123")
    );
}

#[test]
fn formatter_preserves_each_opening_spelling_and_preamble_without_changing_execution() {
    for tag in ["<?scripta", "<?scr"] {
        let source = format!(
            "\u{feff}#!/usr/bin/env scripta\r\n{tag}/* <?scr and <?scripta stay literal */$name=\"Scripta\";echo \"Hello from \".$name;?>"
        );
        let formatted = formatter::format(&source).unwrap();
        assert!(
            formatted.starts_with(&format!("\u{feff}#!/usr/bin/env scripta\n{tag}\n")),
            "{formatted}"
        );
        assert!(formatted.contains("/* <?scr and <?scripta stay literal */"));
        assert_eq!(formatter::format(&formatted).unwrap(), formatted);
        let mut original_output = Vec::new();
        let mut formatted_output = Vec::new();
        pipeline::run(&source, &mut original_output).unwrap();
        pipeline::run(&formatted, &mut formatted_output).unwrap();
        assert_eq!(original_output, b"Hello from Scripta");
        assert_eq!(formatted_output, original_output);
    }
}
