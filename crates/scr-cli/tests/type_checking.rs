use scr_cli::pipeline;
use std::process::Command;

fn rejects(source: &str, message: &str) {
    let errors = pipeline::check(source).expect_err(source);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "E0310" && e.message.contains(message)),
        "{source}: {errors:?}"
    );
}

#[test]
fn strict_operators_conditions_and_collections() {
    for (source, message) in [
        ("$x = 1; $x + 1.0;", "incompatible operands"),
        ("true + $unknown;", "incompatible operands"),
        ("'a' . 2;", "incompatible operands"),
        ("1 < '2';", "incompatible operands"),
        ("true && 1;", "logical operator"),
        ("![];", "unary operator"),
        ("-'2';", "unary operator"),
        ("if (1) {}", "condition"),
        ("while ('yes') {}", "condition"),
        ("for (; 1;) {}", "condition"),
        ("echo 1;", "echo"),
        ("exit('1');", "exit"),
        ("require 1;", "require"),
        ("$x = []; $x['key'];", "index"),
        ("$x = 1; $x[0];", "index access"),
        ("$x = []; $x[true] = 1;", "index"),
        ("$x = 1; $x[] = 2;", "append"),
        ("$x = [true => 1];", "map key"),
        ("$x = [...1];", "spread"),
        ("$x = [1]; $x['1':];", "slice bound"),
        ("$x = 'a'; $x[:];", "slice"),
        ("1 in 'abc';", "string membership"),
        ("true in ['a' => 1];", "map membership"),
        ("1 in 2;", "membership"),
        ("foreach (1 as $x) {}", "foreach"),
        ("$x = [$v for $v in 1];", "comprehension"),
        ("$x = [$v for $v in [] if 1];", "condition"),
        ("$x = []; echo \"{$x}\";", "interpolation"),
        ("$x = 1; $x += 1.0;", "incompatible operands"),
    ] {
        rejects(source, message);
    }
    for source in [
        "1 === '1'; 1 !== 1.0;",
        "$x = 1; $x = 'ok'; echo $x;",
        "$x = 1; { $x = 'ok'; } echo $x;",
        "{ $x = 1; } echo $x;",
        "echo str(1) . 'a'; float(1) + 2.0;",
        "$x = []; $x[] = 1; $x[0] = 'a';",
        "$x = ['a' => 1]; $x['a'] = true;",
        "$x = [1]; $x[-1]; $x[0:1:-1];",
        "$x = [1, ...[2]]; true in $x;",
    ] {
        pipeline::check(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    }
}

#[test]
fn functions_defaults_variadics_and_inferred_results() {
    for (source, message) in [
        (
            "f('bad'); function f($n: int): int { return $n; }",
            "argument 1",
        ),
        ("function f($n) {} f();", "arguments"),
        ("function f() {} f(1);", "arguments"),
        ("function f($n: int = 'bad') {}", "parameter default"),
        ("function f(): int { return 'bad'; }", "return"),
        ("f(); exit; function f(): int { return 'bad'; }", "return"),
        ("function f(): int {}", "return"),
        ("function f($b: bool): int { if ($b) return 1; }", "return"),
        ("function f(): void { return 1; }", "return"),
        ("function f(...$ns: int) {} f(1, 'bad');", "argument 2"),
        (
            "function f(...$ns: int) { $ns + 1; }",
            "incompatible operands",
        ),
        (
            "$f = function ($x: int): int { return $x; }; $f('bad');",
            "argument 1",
        ),
        ("$f = 1; $f();", "call requires callable"),
        ("function f() { return 1; } echo f();", "echo"),
        ("echo f(); function f() { return 1; }", "echo"),
        ("function f($a: int) {} f(...['bad']);", "argument 1"),
        (
            "function f($n: int): int { return f('bad'); }",
            "argument 1",
        ),
    ] {
        rejects(source, message);
    }
    for source in [
        "function f($x: int): string { $x = 'ok'; return $x; } f(1);",
        "function f($a: int = 1, ...$rest: int): int { return $a; } f(); f(1, 2, 3);",
        "function f($a: int = 1, $b: int = $a): int { return $b; }",
        "function f($b: bool): int { if ($b) return 1; else return 2; }",
        "function f(): int { if (true) return 1; }",
        "function f(): int { while (true) { return 1; } }",
        "function f(): int { for (;;) { return 1; } }",
        "function f(): void { return; }",
        "function f($n: int): int { if ($n === 0) return 0; return g($n - 1); } function g($n: int): int { return f($n); }",
        "function f($x: Custom): Custom { return unknown($x); }",
        "function f($x: int) {} f(...$unknown);",
    ] {
        pipeline::check(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    }
}

#[test]
fn native_contracts_use_the_registered_feature_set() {
    for (source, message) in [
        ("len();", "arguments"),
        ("len(1);", "argument 1"),
        ("String::upper(1);", "argument 1"),
        ("File::write('path', 1);", "argument 2"),
        ("sleep('1');", "argument 1"),
        ("echo len('abc');", "echo"),
        (
            "$b = Bytes::encodeUtf8('a'); $b + 1;",
            "incompatible operands",
        ),
        ("$x = 1; len('abc'); $x + 1.0;", "incompatible operands"),
    ] {
        rejects(source, message);
    }
    #[cfg(feature = "sqlite")]
    rejects("Sqlite::open(1);", "argument 1");
    #[cfg(not(feature = "sqlite"))]
    pipeline::check("Sqlite::open(1);").unwrap();
    pipeline::check("str(1); int('1'); float(1); bool(1); len([]); len('abc');").unwrap();
}

#[test]
fn control_flow_merges_and_dynamic_boundaries_are_conservative() {
    for source in [
        "$x = 1; if ($unknown) { $x = 'a'; } $x + 1;",
        "$x = 1; while ($unknown) { echo str($x); $x = 'a'; } echo str($x);",
        "$x = 1; for (; $unknown; $x = 'a') { echo str($x); }",
        "$x = 1; foreach ([] as $v) { $x = 'a'; } echo str($x);",
        "$x = 1; try { $x = 'a'; unknown(); } catch ($e) { echo str($x); }",
        "function f(): int { try { return 'overridden'; } finally { return 1; } }",
        "function f(): int { try { return 1; } finally { echo 'cleanup'; } }",
        "function outer() { $x = 1; $f = function () { $x = 'a'; }; $f(); echo $x; }",
        "function outer() { $x = 1; $f = function () { return $x; }; $x = 'a'; echo $f(); }",
        "function outer() { $x = 1; $f = function () { $x = 'a'; throw 0; }; try { $f(); } catch ($e) { echo $x; } }",
        "$x = 1; function f() { echo $x; }",
        "require 'missing.scripta'; echo missing();",
        "require 'missing.scripta' as String; String::upper(1);",
        "$x = 1; require 'missing.scripta'; echo $x;",
        "false && (1 + 'unreachable'); true || (1 + 'unreachable');",
        "if (false) { echo 1; } else { echo 'ok'; }",
        "while (false) { echo 1; }",
        "for (; false;) { echo 1; }",
    ] {
        pipeline::check(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    }
    rejects(
        "$x = 1; if ($unknown) { $x = 'a'; } else { $x = []; } $x + 1;",
        "incompatible operands",
    );
    rejects(
        "function f($b: bool): int { if ($b) return 'a'; else return []; }",
        "return",
    );
    rejects(
        "function f(): int { try { return 1; } finally { return 'bad'; } }",
        "return",
    );
    rejects(
        "$x = 1; if ($unknown) { $x = []; } else { $x = []; } echo $x;",
        "echo",
    );
}

#[test]
fn class_contracts_and_methods_are_checked() {
    for (source, message) in [
        (
            "class Box { function __construct() { return 1; } }",
            "constructor return",
        ),
        ("class Box { $n: int = 'bad'; }", "field default"),
        (
            "class Box { function f(): int { return 'bad'; } }",
            "return",
        ),
        (
            "class Box { function __construct($n: int) {} } new Box('bad');",
            "argument 1",
        ),
        (
            "class Box { function f($n: int): int { return $n; } } $b = new Box(); $b->f('bad');",
            "argument 1",
        ),
        (
            "class Box { $n: int; } $b = new Box(); $b->n = 'bad';",
            "field assignment",
        ),
        (
            "class Box { $n: int; function f() { $this->n = 'bad'; } }",
            "field assignment",
        ),
        (
            "class Box { $n: int; function f(): string { return $this->n; } }",
            "return",
        ),
        ("class Box {} new Box(1);", "arguments"),
        (
            "new Box('bad'); class Box { function __construct($n: int) {} }",
            "argument 1",
        ),
    ] {
        rejects(source, message);
    }
    for source in [
        "class Box { $n: int = 1; function f(): int { return $this->n; } } $b = new Box(); $b->f();",
        "class Base {} class Child extends Base {} function f($x: Base): Base { return $x; } f(new Child());",
        "interface Named { function name(): string; } class Person implements Named { function name(): string { return 'name'; } }",
    ] {
        pipeline::check(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    }
}

#[test]
fn cli_reports_source_spans_for_lf_and_crlf_without_execution() {
    for newline in ["\n", "\r\n"] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("types.scripta");
        let source = format!("File::write('effect', 'bad');{newline}len(123);{newline}");
        std::fs::write(&file, &source).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .current_dir(dir.path())
            .arg("check")
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert!(!dir.path().join("effect").exists());
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(stderr.contains("error[E0310]"), "{stderr}");
        assert!(stderr.contains("types.scripta:2:5"), "{stderr}");
        let errors = pipeline::check(&source).unwrap_err();
        let span = errors.iter().find(|e| e.code == "E0310").unwrap().span;
        assert_eq!(&source[span.start..span.end], "123");
    }
}

#[test]
fn runtime_still_handles_dynamic_errors_and_catchable_type_failures() {
    let source = "try { len(1); } catch (TypeError $e) { echo 'caught'; }";
    rejects(source, "argument 1");
    let mut output = Vec::new();
    pipeline::run(source, &mut output).unwrap();
    assert_eq!(output, b"caught");
    pipeline::check("echo unknown();").unwrap();
    assert!(pipeline::run("echo unknown();", &mut Vec::new()).is_err());
}

#[test]
fn accepted_flow_and_contract_examples_execute_with_expected_results() {
    for (source, expected) in [
        (
            "function f($x: int): string { $x = 'ok'; return $x; } echo f(1);",
            "ok",
        ),
        (
            "function f($a: int = 1, $b: int = $a, ...$rest: int): int { return $a + $b + len($rest); } echo str(f()); echo str(f(2, 3, 4));",
            "26",
        ),
        (
            "$x = 1; if (type($x) === 'int') { $x = 'changed'; } echo str($x);",
            "changed",
        ),
        (
            "function f(): int { try { return 'ignored'; } finally { return 7; } } echo str(f());",
            "7",
        ),
        (
            "function f(): string { $x = 1; $change = function () { $x = 'changed'; throw 0; }; try { $change(); } catch ($e) { return $x; } return 'unreachable'; } echo f();",
            "changed",
        ),
        (
            "class Box { $n: int = 2; function add($x: int): int { return $this->n + $x; } } $b = new Box(); echo str($b->add(3));",
            "5",
        ),
    ] {
        pipeline::check(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let mut output = Vec::new();
        pipeline::run(source, &mut output).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert_eq!(output, expected.as_bytes(), "{source}");
    }
}

#[test]
fn long_callable_reference_chains_keep_analysis_bounded() {
    let mut source = String::new();
    for index in 0..128 {
        source.push_str(&format!(
            "function f{index}() {{ return f{}; }}\n",
            index + 1
        ));
    }
    source.push_str("function f128(): int { return 1; } f0();");
    pipeline::check(&source).unwrap();
}
