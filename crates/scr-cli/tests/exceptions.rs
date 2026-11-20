use scr_cli::{formatter, pipeline};
use scr_runtime::{ContextOptions, Runtime, RuntimeOptions, Value};
use std::fs;

#[test]
fn qualified_exception_classes_preserve_module_identity_and_definition_scopes() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("a.scripta"),
        r#"
        class Detail {}
        class Problem extends Error {
            $detail: Detail = new Detail();
            function getDetail(): Detail { return $this->detail; }
        }
        function fail() { throw new Problem("module failure"); }
    "#,
    )
    .unwrap();
    fs::write(
        dir.path().join("b.scripta"),
        "class Problem extends Error {}",
    )
    .unwrap();
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: dir.path().into(),
            ..Default::default()
        },
        ..Default::default()
    });
    rt.execute_source(
        r#"
        require_once "a.scripta" as A;
        require_once "a.scripta" as Same;
        require "b.scripta" as B;
        class Child extends A::Problem {}
        $matched = ""; $saved = null;
        try { A::fail(); }
        catch (B::Problem $e) { $matched = "wrong"; }
        catch (Same::Problem $e) { $matched = $e->name; $saved = $e; }
        $child = new Child("child");
        $detail = $child->getDetail();
        function accept($value: A::Problem): Error { return $value; }
        $same = accept($child) === $child;
    "#,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        rt.get_global("matched"),
        Some(&Value::String("Problem".into()))
    );
    assert_eq!(rt.get_global("same"), Some(&Value::Bool(true)));
    rt.collect_garbage();
    rt.execute_source("$again = $child->getDetail();", &mut Vec::new())
        .unwrap();
    assert_eq!(rt.get_global("again"), rt.get_global("detail"));
    let error = rt
        .execute_source("throw $saved;", &mut Vec::new())
        .unwrap_err();
    assert!(error.source.as_ref().unwrap().path.ends_with("a.scripta"));
}

#[test]
fn formatting_and_execution_support_exception_syntax_with_lf_and_crlf() {
    let source = "class Problem extends Error {function describe():string{return $this->name . ': ' . $this->message;}}\ntry{throw new Problem('bad');}catch(Problem $e){echo $e->describe();}\n";
    for bytes in [
        source.as_bytes().to_vec(),
        source.replace('\n', "\r\n").into_bytes(),
    ] {
        let input = String::from_utf8(bytes).unwrap();
        pipeline::check(&input).unwrap();
        let formatted = formatter::format(&input).unwrap();
        assert!(formatted.contains("class Problem extends Error"));
        assert!(formatted.contains("$this->message"));
        assert_eq!(formatter::format(&formatted).unwrap(), formatted);
        let mut output = Vec::new();
        pipeline::run(&formatted, &mut output).unwrap();
        assert_eq!(output, b"Problem: bad");
    }
}

#[test]
fn inherited_contract_failures_report_the_method_definition_source() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("base.scripta"),
        "class Base extends Error {function describe(): string {return 'base';}}",
    )
    .unwrap();
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: dir.path().into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let error = rt.execute_source("require 'base.scripta' as B; interface I {function describe(): int;} class Child extends B::Base implements I {}", &mut Vec::new()).unwrap_err();
    assert!(error.message.contains("does not match interface"));
    let source = error.source.as_ref().unwrap();
    assert!(source.path.ends_with("base.scripta"));
    assert_eq!(&source.text[error.span.start..error.span.end], "describe");
}

#[test]
fn method_exceptions_compose_with_interpolation_rethrows_and_explicit_causes() {
    let source = r#"
class ValidationError extends Error {
    $details = ["field" => "é🦀"];
    function describe(): string { return "{$this->details["field"]}: {$this->message}"; }
}
class MissingField extends ValidationError {}
class Wrapped extends Error {}
class Validator {
    function check() { throw new MissingField("required"); }
    function render(): string { return "result: {$this->check()}"; }
}
$validator = new Validator();
$saved = "unchanged"; $original = null; $wrapped = null;
$line = null; $trace = null; $same = false; $sameLine = false; $sameTrace = false; $causeSame = false;
try {
    try { $saved = "prefix {$validator->render()} suffix"; }
    catch (ValidationError $e) {
        $original = $e; $line = $e->line; $trace = $e->trace;
        echo "{$e->name}: {$e->describe()}\n";
        throw $e;
    }
} catch (MissingField $e) {
    $same = $e === $original; $sameLine = $line === $e->line; $sameTrace = $trace === $e->trace;
    $wrapped = new Wrapped("context {$e->details["field"]}", $e);
}
try { throw $wrapped; }
catch (Error $e) {
    echo "{$e->message}: {$e->cause->name}: {$e->cause->details["field"]}\n";
    $causeSame = $e->cause === $original;
}
echo $saved;
"#;
    for input in [source.to_owned(), source.replace('\n', "\r\n")] {
        pipeline::check(&input).unwrap();
        let formatted = formatter::format(&input).unwrap();
        assert_eq!(formatter::format(&formatted).unwrap(), formatted);
        for source in [&input, &formatted] {
            let mut rt = Runtime::new();
            let mut output = Vec::new();
            rt.execute_source(source, &mut output).unwrap();
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "MissingField: é🦀: required\ncontext é🦀: MissingField: é🦀\nunchanged"
            );
            for name in ["same", "sameLine", "sameTrace", "causeSame"] {
                assert_eq!(rt.get_global(name), Some(&Value::Bool(true)), "{name}");
            }
            rt.collect_garbage();
            let error = rt
                .execute_source("throw $original;", &mut Vec::new())
                .unwrap_err();
            assert_eq!(error.source.as_ref().unwrap().text, *source);
            assert_eq!(
                &source[error.span.start..error.span.end],
                "new MissingField(\"required\")"
            );
            assert!(error.trace.iter().any(|frame| frame.function == "check"));
            assert!(error.trace.iter().any(|frame| frame.function == "render"));
        }
    }
}

#[test]
fn failed_interpolated_method_access_retains_module_origin_when_wrapped_and_rethrown() {
    for newline in ["\n", "\r\n"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("renderer.scripta");
        let module = [
            "class Renderer {",
            "    $items = [\"ok\"];",
            "    function render(): string { return \"é🦀 {$this->items[1]}\"; }",
            "}",
            "function create() { return new Renderer(); }",
            "",
        ]
        .join(newline);
        fs::write(&path, &module).unwrap();
        let mut rt = Runtime::with_options(RuntimeOptions {
            context: ContextOptions {
                working_directory: directory.path().into(),
                ..Default::default()
            },
            ..Default::default()
        });
        rt.execute_source(
            r#"
require "renderer.scripta" as Module;
$renderer = Module::create(); $saved = "unchanged"; $wrapped = null;
try { $saved = "{$renderer->render()}"; }
catch (IndexError $e) { $wrapped = new Error("render failed", $e); }
$originLine = $wrapped->cause->line;
$originColumn = $wrapped->cause->column;
"#,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            rt.get_global("saved"),
            Some(&Value::String("unchanged".into()))
        );
        assert_eq!(rt.get_global("originLine"), Some(&Value::Int(3)));
        let start = module.find("$this->items[1]").unwrap();
        let line_start = module[..start].rfind('\n').unwrap() + 1;
        let column = module[line_start..start].chars().count() + 1;
        assert_eq!(
            rt.get_global("originColumn"),
            Some(&Value::Int(column as i64))
        );
        rt.collect_garbage();
        let error = rt
            .execute_source("throw $wrapped->cause;", &mut Vec::new())
            .unwrap_err();
        let source = error.source.as_ref().unwrap();
        assert_eq!(source.path, path.canonicalize().unwrap());
        assert_eq!(source.text, module);
        assert_eq!(&module[error.span.start..error.span.end], "$this->items[1]");
        assert!(error.trace.iter().any(|frame| frame.function == "render"));
    }
}
