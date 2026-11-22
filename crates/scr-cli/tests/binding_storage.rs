use scr_runtime::{ContextOptions, Runtime, RuntimeOptions, Value};
use std::fs;

#[test]
fn dynamic_includes_and_aliases_remain_lexical_across_reused_calls() {
    for newline in ["\n", "\r\n"] {
        let directory = tempfile::tempdir().unwrap();
        for (file, source) in [
            (
                "first.scripta",
                "function value(): int {\n    return 1;\n}\n",
            ),
            (
                "second.scripta",
                "function value(): int {\n    return 2;\n}\n",
            ),
            (
                "capture.scripta",
                "$seed += 1;\nfunction read($value = $seed) { return $value; }\n",
            ),
        ] {
            fs::write(
                directory.path().join(file),
                source.replace('\n', newline).as_bytes(),
            )
            .unwrap();
        }
        let mut runtime = Runtime::with_options(RuntimeOptions {
            context: ContextOptions {
                working_directory: directory.path().into(),
                ..Default::default()
            },
            ..Default::default()
        });
        runtime
            .execute_source(
                r#"
            function first() { require 'first.scripta' as Local; return Local::value(); }
            function second() { require 'second.scripta' as Local; return Local::value(); }
            function factory($seed) { require 'capture.scripta'; return read; }
            $a = first(); $b = second();
            $f = factory(10); $g = factory(20);
            for ($i = 0; $i < 100; $i += 1) { first(); second(); }
            $x = $f(); $y = $g();
        "#,
                &mut Vec::new(),
            )
            .unwrap();
        runtime.collect_garbage();
        for (name, expected) in [("a", 1), ("b", 2), ("x", 11), ("y", 21)] {
            assert_eq!(
                runtime.get_global(name),
                Some(&Value::Int(expected)),
                "{name}, newline={newline:?}"
            );
        }
        let error = runtime
            .execute_source("Local::value();", &mut Vec::new())
            .unwrap_err();
        assert!(error.message.contains("undefined function Local::value"));
        runtime
            .execute_source("$result = first() + second();", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(3)));
    }
}

#[test]
fn isolated_modules_keep_native_fallback_and_exclude_caller_functions() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("module.scripta"),
        "function native() { return host(); } function caller() { return callerOnly(); }",
    )
    .unwrap();
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: directory.path().into(),
            ..Default::default()
        },
        ..Default::default()
    });
    runtime
        .register(
            "host",
            scr_runtime::NativeFunction::new(
                scr_runtime::NativeSignature::new(vec![], scr_runtime::NativeType::Int),
                |_, _| Ok(Value::Int(42)),
            ),
        )
        .unwrap();
    runtime.execute_source("function callerOnly() { return 99; } require 'module.scripta' as Module; $result = Module::native();", &mut Vec::new()).unwrap();
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    let error = runtime
        .execute_source("Module::caller();", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("undefined function callerOnly"));
    runtime
        .execute_source("$result = Module::native();", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
}
