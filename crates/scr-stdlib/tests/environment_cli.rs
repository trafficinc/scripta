use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use std::{collections::BTreeMap, ffi::OsString, io::Cursor};
fn runtime(arguments: &[&str]) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            cli_arguments: arguments.iter().map(OsString::from).collect(),
            environment: BTreeMap::new(),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str, output: &mut Vec<u8>) -> Result<u8, RuntimeError> {
    let source = format!("<?scr {source}");
    runtime.execute(
        &scr_parser::parse(&scr_lexer::lex(&source).unwrap()).unwrap(),
        output,
    )
}
fn error(runtime: &mut Runtime, source: &str, kind: &str) {
    let failure = run(runtime, source, &mut Vec::new()).unwrap_err();
    assert!(
        failure.message.starts_with(&format!("{kind}:")),
        "{source}: {failure}"
    );
}
#[test]
fn environment_roundtrip_removal_and_snapshot_are_runtime_local() {
    let original = std::env::var_os("SCR_PHASE9_LOCAL_TEST");
    let mut first = runtime(&[]);
    let mut second = runtime(&[]);
    run(&mut first, r#"$missing = Env::get("SCR_PHASE9_LOCAL_TEST"); Env::set("SCR_PHASE9_LOCAL_TEST", "世界"); $has = Env::has("SCR_PHASE9_LOCAL_TEST"); $value = Env::get("SCR_PHASE9_LOCAL_TEST"); $all = Env::all(); Env::remove("SCR_PHASE9_LOCAL_TEST"); Env::remove("SCR_PHASE9_LOCAL_TEST"); $removed = Env::has("SCR_PHASE9_LOCAL_TEST");"#, &mut Vec::new()).unwrap();
    assert_eq!(first.get_global("missing"), Some(&Value::Null));
    assert_eq!(first.get_global("has"), Some(&Value::Bool(true)));
    assert_eq!(
        first.get_global("value"),
        Some(&Value::String("世界".into()))
    );
    assert_eq!(first.get_global("removed"), Some(&Value::Bool(false)));
    let Some(Value::Map(all)) = first.get_global("all") else {
        panic!();
    };
    assert_eq!(all.len(), 1);
    run(
        &mut second,
        r#"$other = Env::has("SCR_PHASE9_LOCAL_TEST");"#,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(second.get_global("other"), Some(&Value::Bool(false)));
    assert_eq!(std::env::var_os("SCR_PHASE9_LOCAL_TEST"), original);
}
#[test]
fn environment_invalid_names_values_types_and_permissions() {
    let mut runtime = runtime(&[]);
    for source in [
        r#"Env::set("", "value");"#,
        r#"Env::has("a=b");"#,
        "Env::get();",
        "Env::set(1, 2);",
        "Env::all(1);",
    ] {
        error(&mut runtime, source, "TypeError");
    }
    runtime
        .set_global("nul", Value::String("bad\0value".into()))
        .unwrap();
    error(&mut runtime, "Env::set(\"valid\", $nul);", "TypeError");
    error(&mut runtime, "Env::get($nul);", "TypeError");
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .remove("environment.read");
    options
        .context
        .permissions
        .allowed
        .remove("environment.write");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    for source in [
        r#"Env::get("HOME");"#,
        r#"Env::set("X", "y");"#,
        "Env::all();",
        r#"Env::remove("X");"#,
    ] {
        error(&mut denied, source, "PermissionError");
    }
}
#[test]
fn cli_args_are_zero_based_verbatim_and_out_of_range_is_null() {
    let mut runtime = runtime(&["one", "世界", "", "--help"]);
    run(&mut runtime, "$args = Cli::args(); $first = Cli::arg(0); $empty = Cli::arg(2); $last = Cli::arg(3); $missing = Cli::arg(4);", &mut Vec::new()).unwrap();
    assert_eq!(
        runtime.get_global("args"),
        Some(&Value::List(
            ["one", "世界", "", "--help"]
                .map(|value| Value::String(value.into()))
                .to_vec()
        ))
    );
    assert_eq!(
        runtime.get_global("first"),
        Some(&Value::String("one".into()))
    );
    assert_eq!(
        runtime.get_global("empty"),
        Some(&Value::String(String::new()))
    );
    assert_eq!(runtime.get_global("missing"), Some(&Value::Null));
    error(&mut runtime, "Cli::arg(-1);", "TypeError");
    error(&mut runtime, "Cli::arg(1.0);", "TypeError");
}
#[test]
fn cli_options_support_equals_separate_values_repeats_and_terminator() {
    let mut runtime = runtime(&[
        "positional",
        "--output",
        "old",
        "--flag",
        "--output=new",
        "-n=3",
        "--empty=",
        "--count=-1",
        "--",
        "--ignored=yes",
    ]);
    run(&mut runtime, r#"$has = Cli::has("flag"); $prefixed = Cli::has("--flag"); $value = Cli::option("output"); $short = Cli::option("n"); $flag = Cli::option("flag"); $empty = Cli::option("empty"); $negative = Cli::option("count"); $ignored = Cli::has("ignored"); $missing = Cli::option("absent");"#, &mut Vec::new()).unwrap();
    for name in ["has", "prefixed"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Bool(true)));
    }
    for (name, value) in [
        ("value", "new"),
        ("short", "3"),
        ("empty", ""),
        ("negative", "-1"),
    ] {
        assert_eq!(runtime.get_global(name), Some(&Value::String(value.into())));
    }
    for name in ["flag", "missing"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Null));
    }
    assert_eq!(runtime.get_global("ignored"), Some(&Value::Bool(false)));
    error(&mut runtime, "Cli::has(\"--\");", "TypeError");
}
#[test]
fn prompts_use_injected_input_trim_line_endings_and_handle_eof() {
    let mut runtime = runtime(&[]);
    runtime.set_input_stream(Cursor::new("世界\r\n\nlast".as_bytes().to_vec()));
    let mut output = Vec::new();
    run(&mut runtime, r#"$first = Cli::prompt("Name: "); $empty = Cli::prompt("Again: "); $last = Cli::prompt(""); $eof = Cli::prompt("");"#, &mut output).unwrap();
    assert_eq!(output, b"Name: Again: ");
    assert_eq!(
        runtime.get_global("first"),
        Some(&Value::String("世界".into()))
    );
    assert_eq!(
        runtime.get_global("empty"),
        Some(&Value::String(String::new()))
    );
    assert_eq!(
        runtime.get_global("last"),
        Some(&Value::String("last".into()))
    );
    assert_eq!(runtime.get_global("eof"), Some(&Value::Null));
    runtime.set_input_stream(Cursor::new(vec![0xff, b'\n']));
    error(&mut runtime, r#"Cli::prompt("");"#, "EncodingError");
    runtime.set_input_stream(Cursor::new(vec![b'x'; 65537]));
    error(&mut runtime, r#"Cli::prompt("");"#, "InputLimitError");
}
#[test]
fn prompt_permissions_and_io_failures_are_exceptions() {
    struct Fail;
    impl std::io::BufRead for Fail {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            Err(std::io::Error::other("closed input"))
        }
        fn consume(&mut self, _: usize) {}
    }
    impl std::io::Read for Fail {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed input"))
        }
    }
    let mut rt = runtime(&[]);
    rt.set_input_stream(Fail);
    error(&mut rt, r#"Cli::prompt("");"#, "IoError");
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.remove("cli.prompt");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    error(&mut denied, r#"Cli::prompt("blocked");"#, "PermissionError");
}
#[cfg(unix)]
#[test]
fn non_utf8_environment_and_arguments_raise_encoding_errors() {
    use std::os::unix::ffi::OsStringExt;
    let mut options = RuntimeOptions::default();
    options
        .context
        .environment
        .insert("INVALID".into(), OsString::from_vec(vec![0xff]));
    options
        .context
        .cli_arguments
        .push(OsString::from_vec(vec![0xff]));
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).unwrap();
    for source in [
        r#"Env::get("INVALID");"#,
        "Env::all();",
        "Cli::args();",
        "Cli::arg(0);",
    ] {
        error(&mut runtime, source, "EncodingError");
    }
}
#[cfg(windows)]
#[test]
fn windows_environment_names_are_ascii_case_insensitive() {
    let mut runtime = runtime(&[]);
    run(&mut runtime, r#"Env::set("MixedName", "one"); Env::set("MIXEDNAME", "two"); $value = Env::get("mixedname"); $all = Env::all(); Env::remove("mixedNAME"); $has = Env::has("MixedName");"#, &mut Vec::new()).unwrap();
    assert_eq!(
        runtime.get_global("value"),
        Some(&Value::String("two".into()))
    );
    assert_eq!(runtime.get_global("has"), Some(&Value::Bool(false)));
    let Some(Value::Map(all)) = runtime.get_global("all") else {
        panic!();
    };
    assert_eq!(all.len(), 1);
}

#[test]
fn read_lines_preserve_unicode_empty_lines_bom_and_bare_cr_without_output() {
    let mut rt = runtime(&[]);
    rt.set_input_stream(std::io::BufReader::with_capacity(
        2,
        Cursor::new("\u{feff}é🦀\r\n\nlast\r".as_bytes().to_vec()),
    ));
    let mut output = Vec::new();
    run(&mut rt, "$a = Cli::readLine(); $b = Cli::readLine(); $c = Cli::readLine(); $d = Cli::readLine(); $e = Cli::readLine();", &mut output).unwrap();
    for (name, value) in [("a", "\u{feff}é🦀"), ("b", ""), ("c", "last\r")] {
        assert_eq!(rt.get_global(name), Some(&Value::String(value.into())));
    }
    assert_eq!(rt.get_global("d"), Some(&Value::Null));
    assert_eq!(rt.get_global("e"), Some(&Value::Null));
    assert!(output.is_empty());
}

#[test]
fn read_line_limits_encoding_and_failure_state() {
    for input in [b"abc\nnext\n".to_vec(), vec![b'x'; 1024 * 1024 + 1]] {
        let mut rt = runtime(&[]);
        rt.set_input_stream(Cursor::new(input));
        error(&mut rt, "Cli::readLine(3);", "ResourceLimitError");
        error(&mut rt, "Cli::readLine();", "InputError");
        error(&mut rt, "Cli::prompt(\"\");", "InputError");
    }
    let mut rt = runtime(&[]);
    rt.set_input_stream(Cursor::new(vec![0xff, b'\n', b'x']));
    error(&mut rt, "Cli::readLine();", "EncodingError");
    error(&mut rt, "Cli::readLine();", "InputError");
    let mut rt = runtime(&[]);
    rt.set_input_stream(Cursor::new(b"ab\n".to_vec()));
    for script in ["Cli::readLine(0);", "Cli::readLine(-1);"] {
        error(&mut rt, script, "RangeError");
    }
    for script in [
        "Cli::readLine(null);",
        "Cli::readLine(1.0);",
        "Cli::readLine(1, 2);",
    ] {
        error(&mut rt, script, "TypeError");
    }
    run(&mut rt, "$value = Cli::readLine(3);", &mut Vec::new()).unwrap();
    assert_eq!(rt.get_global("value"), Some(&Value::String("ab".into())));
}

#[test]
fn read_line_value_budget_and_separate_permission() {
    for input in ["é🦀\n", "é🦀\r\n", "é🦀"] {
        for enough in [true, false] {
            let mut rt = Runtime::with_options(RuntimeOptions {
                max_value_bytes: Some(
                    std::mem::size_of::<Value>() + "é🦀".len() - usize::from(!enough),
                ),
                ..Default::default()
            });
            scr_stdlib::register(&mut rt).unwrap();
            rt.set_input_stream(std::io::BufReader::with_capacity(
                2,
                Cursor::new(input.as_bytes().to_vec()),
            ));
            if enough {
                run(&mut rt, "$x = Cli::readLine();", &mut Vec::new()).unwrap();
                assert_eq!(rt.get_global("x"), Some(&Value::String("é🦀".into())));
            } else {
                error(&mut rt, "Cli::readLine();", "ResourceLimitError");
            }
        }
    }
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.remove("cli.read");
    let mut rt = Runtime::with_options(options);
    scr_stdlib::register(&mut rt).unwrap();
    rt.set_input_stream(Cursor::new(b"ok\n".to_vec()));
    error(&mut rt, "Cli::readLine();", "PermissionError");
    run(&mut rt, "$x = Cli::prompt(\"\");", &mut Vec::new()).unwrap();
    assert_eq!(rt.get_global("x"), Some(&Value::String("ok".into())));
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.remove("cli.prompt");
    let mut rt = Runtime::with_options(options);
    scr_stdlib::register(&mut rt).unwrap();
    rt.set_input_stream(Cursor::new(b"ok\n".to_vec()));
    run(&mut rt, "$x = Cli::readLine();", &mut Vec::new()).unwrap();
}

#[test]
fn read_line_io_failure_and_mid_read_cancellation_disable_input() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Reader {
        flag: Option<Arc<AtomicBool>>,
    }
    impl std::io::Read for Reader {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            unreachable!()
        }
    }
    impl std::io::BufRead for Reader {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if let Some(flag) = &self.flag {
                flag.store(true, Ordering::Relaxed);
                Ok(b"partial")
            } else {
                Err(std::io::Error::other("injected input failure"))
            }
        }
        fn consume(&mut self, _: usize) {}
    }
    let mut rt = runtime(&[]);
    rt.set_input_stream(Reader { flag: None });
    error(&mut rt, "Cli::readLine();", "IoError");
    error(&mut rt, "Cli::readLine();", "InputError");
    let flag = Arc::new(AtomicBool::new(false));
    let mut rt = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    scr_stdlib::register(&mut rt).unwrap();
    rt.set_input_stream(Reader {
        flag: Some(flag.clone()),
    });
    let failure = run(&mut rt, "Cli::readLine();", &mut Vec::new()).unwrap_err();
    assert!(failure.message.contains("interrupt"), "{failure}");
    flag.store(false, Ordering::Relaxed);
    error(&mut rt, "Cli::readLine();", "InputError");
}

#[test]
fn read_line_default_limit_and_shared_prompt_cursor() {
    let mut rt = runtime(&[]);
    rt.set_input_stream(Cursor::new(vec![b'x'; 1024 * 1024 + 1]));
    error(&mut rt, "Cli::readLine();", "ResourceLimitError");
    let mut rt = runtime(&[]);
    rt.set_input_stream(Cursor::new(b"first\nsecond\r\nthird".to_vec()));
    run(
        &mut rt,
        "$a = Cli::readLine(); $b = Cli::prompt(\"\"); $c = Cli::readLine();",
        &mut Vec::new(),
    )
    .unwrap();
    for (name, text) in [("a", "first"), ("b", "second"), ("c", "third")] {
        assert_eq!(rt.get_global(name), Some(&Value::String(text.into())));
    }
}
