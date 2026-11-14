use scr_runtime::{
    ContextOptions, NativeError, NativeFunction, NativeObject, NativeSignature, NativeType,
    PermissionConfig, Runtime, RuntimeOptions, Value,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ffi::OsString,
    io::Write,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, scr_runtime::RuntimeError> {
    let tokens = scr_lexer::lex(&format!("<?scr {source}")).unwrap();
    runtime.execute(&scr_parser::parse(&tokens).unwrap(), &mut Vec::new())
}
fn constant(value: i64) -> NativeFunction {
    NativeFunction::new(
        NativeSignature::new(vec![], NativeType::Int),
        move |_, _| Ok(Value::Int(value)),
    )
}

#[test]
fn registry_supports_namespaces_closures_and_duplicate_detection() {
    let mut runtime = Runtime::new();
    for name in [
        "File::exists",
        "File::read",
        "System::platform",
        "Env::get",
        "Process::run",
    ] {
        runtime.register(name, constant(7)).unwrap();
        run(&mut runtime, &format!("$result = {name}();")).unwrap();
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(7)));
        assert_eq!(
            runtime.register(name, constant(9)).unwrap_err().kind,
            "RegistryError"
        );
        run(&mut runtime, &format!("$result = {name}();")).unwrap();
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(7)));
    }
    for name in [
        "",
        "a::",
        "::b",
        "a:::b",
        "a::b::c",
        "bad-name",
        "$bad",
        "1bad",
        "echo",
        "exit",
        "while",
        "File::return",
    ] {
        assert!(runtime.register(name, constant(0)).is_err(), "{name}");
    }
    run(&mut runtime, "function script() {}").unwrap();
    assert!(runtime.register("script", constant(0)).is_err());
    assert!(run(&mut runtime, "function File() {} function File() {}").is_err());
}

#[test]
fn invalid_arguments_never_reach_callback_and_types_are_strict() {
    let called = Rc::new(Cell::new(0));
    let count = called.clone();
    let mut runtime = Runtime::new();
    runtime
        .register(
            "check",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Int, NativeType::String], NativeType::Bool),
                move |_, _| {
                    count.set(count.get() + 1);
                    Ok(Value::Bool(true))
                },
            ),
        )
        .unwrap();
    for source in [
        "check();",
        "check(1);",
        "check(1, 2);",
        "check(1.0, \"x\");",
        "check(true, \"x\");",
        "check(1, \"x\", 3);",
    ] {
        let error = run(&mut runtime, source).unwrap_err();
        assert!(error.message.starts_with("TypeError:"), "{error}");
        assert_eq!(error.trace.last().unwrap().function, "check");
    }
    assert_eq!(called.get(), 0);
    run(&mut runtime, "$result = check(1, \"x\");").unwrap();
    assert_eq!(called.get(), 1);
    // Native objects cannot masquerade as scalar types by choosing a type name.
    #[derive(Debug)]
    struct FakeInt;
    impl NativeObject for FakeInt {
        fn type_name(&self) -> &str {
            "int"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Ok(Value::Null)
        }
    }
    runtime
        .set_global("fake", Value::NativeObject(Rc::new(FakeInt)))
        .unwrap();
    assert!(run(&mut runtime, "check($fake, \"x\");").is_err());
    assert_eq!(called.get(), 1);
}

#[test]
fn rust_errors_become_typed_exceptions_with_original_call_spans() {
    let mut runtime = Runtime::new();
    runtime
        .register(
            "File::read",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::String], NativeType::String),
                |_, _| {
                    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing file").into())
                },
            ),
        )
        .unwrap();
    let source = "<?scr function inner() { return File::read(\"missing\"); } function outer() { return inner(); } outer();";
    let program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let error = runtime.execute(&program, &mut Vec::new()).unwrap_err();
    assert_eq!(
        &source[error.span.start..error.span.end],
        "File::read(\"missing\")"
    );
    assert!(error.message.contains("IoError: missing file"));
    assert_eq!(
        error
            .trace
            .iter()
            .map(|frame| frame.function.as_str())
            .collect::<Vec<_>>(),
        ["<main>", "outer", "inner", "File::read"]
    );
    run(&mut runtime, r#"$caught = false; $message = ""; try { outer(); } catch (TypeError $e) {} catch (IoError $e) { $caught = true; $message = $e; }"#).unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    assert_eq!(
        runtime.get_global("message"),
        Some(&Value::String("missing file".into()))
    );
    run(
        &mut runtime,
        "try { outer(); } catch (Error $e) {} try { outer(); } catch ($e) {}",
    )
    .unwrap();
    assert!(runtime.get_global("e").is_none());
}

#[test]
fn native_return_contracts_and_host_values_are_validated() {
    for (ty, value, expected) in [
        (
            NativeType::Int,
            Value::String("wrong".into()),
            "returned string, expected int",
        ),
        (NativeType::Float, Value::Float(f64::INFINITY), "finite"),
        (
            NativeType::List,
            Value::List(vec![Value::Float(f64::NAN)]),
            "finite",
        ),
        (
            NativeType::Map,
            Value::Map(vec![
                (scr_runtime::MapKey::Int(1), Value::Null),
                (scr_runtime::MapKey::Int(1), Value::Null),
            ]),
            "duplicate",
        ),
    ] {
        let mut runtime = Runtime::new();
        runtime
            .register(
                "bad",
                NativeFunction::new(NativeSignature::new(vec![], ty), move |_, _| {
                    Ok(value.clone())
                }),
            )
            .unwrap();
        assert!(
            run(&mut runtime, "bad();")
                .unwrap_err()
                .message
                .contains(expected)
        );
        run(
            &mut runtime,
            "$caught = false; try { bad(); } catch (NativeError $e) { $caught = true; }",
        )
        .unwrap();
        assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    }
}

#[derive(Debug)]
struct ReadOnly;
impl NativeObject for ReadOnly {
    fn type_name(&self) -> &str {
        "Result"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "code" => Ok(Value::Int(7)),
            "items" => Ok(Value::List(vec![Value::Int(1)])),
            _ => Err(format!("unknown property {name}")),
        }
    }
}
#[test]
fn native_objects_return_read_only_properties_and_catch_property_errors() {
    let mut runtime = Runtime::new();
    runtime
        .register(
            "Process::run",
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::String, NativeType::List],
                    NativeType::Object("Result"),
                ),
                |_, _| Ok(Value::NativeObject(Rc::new(ReadOnly))),
            ),
        )
        .unwrap();
    run(
        &mut runtime,
        r#"$result = Process::run("test", []); $code = $result->code;"#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("code"), Some(&Value::Int(7)));
    for expression in [
        "$result->code = 9",
        "$result->code += 1",
        "$result->items[0] = 9",
        "$result->items[] = 9",
        "$result->items[1] = 9",
        "$result->missing",
    ] {
        let error = run(&mut runtime, &format!("{expression};")).unwrap_err();
        assert!(error.message.starts_with("PropertyError:"));
        run(&mut runtime, &format!("$caught = false; try {{ {expression}; }} catch (PropertyError $e) {{ $caught = true; }}")).unwrap();
        assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    }
    run(&mut runtime, "$unchanged = $result->code;").unwrap();
    assert_eq!(runtime.get_global("unchanged"), Some(&Value::Int(7)));
    for (expression, kind) in [
        ("$result->items[9]", "IndexError"),
        ("$result->items[9] = 3", "IndexError"),
        ("$result->items[null]", "TypeError"),
    ] {
        run(
            &mut runtime,
            &format!(
                "$caught = false; try {{ {expression}; }} catch ({kind} $e) {{ $caught = true; }}"
            ),
        )
        .unwrap();
        assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    }
}

#[derive(Clone)]
struct Buffer(Rc<RefCell<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn runtime_context_exposes_metadata_env_streams_and_call_location() {
    let mut permissions = PermissionConfig::default();
    permissions.allowed.insert("environment.read".into());
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: PathBuf::from("/host/work"),
            script_path: Some(PathBuf::from("/host/script.scripta")),
            cli_arguments: vec![OsString::from("hello")],
            environment: BTreeMap::from([("SCR_NATIVE_TEST".into(), "value".into())]),
            redacted_values: Vec::new(),
            permissions,
        },
        ..Default::default()
    });
    let stderr = Buffer(Rc::new(RefCell::new(Vec::new())));
    runtime.set_error_stream(stderr.clone());
    runtime
        .register(
            "Host::inspect",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::String),
                |context, _| {
                    assert_eq!(
                        context.working_directory.as_path(),
                        std::path::Path::new("/host/work")
                    );
                    assert_eq!(
                        context.script_path.unwrap(),
                        std::path::Path::new("/host/script.scripta")
                    );
                    assert_eq!(context.cli_arguments, &[OsString::from("hello")]);
                    assert_eq!(context.call_span, scr_diagnostics::Span::new(26, 41));
                    assert_eq!(context.env("SCR_NATIVE_TEST")?.unwrap(), "value");
                    assert!(context.env("ABSENT")?.is_none());
                    context.output.write_all(b"native")?;
                    context.error.write_all(b"diagnostic")?;
                    Ok(Value::String("ok".into()))
                },
            ),
        )
        .unwrap();
    let source = "<?scr echo \"script\"; $v = Host::inspect();";
    let program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let mut output = Vec::new();
    runtime.execute(&program, &mut output).unwrap();
    assert_eq!(output, b"scriptnative");
    assert_eq!(*stderr.0.borrow(), b"diagnostic");
    assert_eq!(runtime.get_global("v"), Some(&Value::String("ok".into())));
}

#[test]
fn permission_denial_stream_errors_and_cancellation_propagate() {
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .remove("environment.read");
    let mut runtime = Runtime::with_options(options);
    runtime
        .register(
            "Env::get",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                |context, _| {
                    context.env("HOME")?;
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    run(
        &mut runtime,
        "$caught = false; try { Env::get(); } catch (PermissionError $e) { $caught = true; }",
    )
    .unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    struct Fail;
    impl Write for Fail {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed stream"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    runtime.set_error_stream(Fail);
    runtime
        .register(
            "write",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                |context, _| {
                    context.error.write_all(b"fail")?;
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    assert!(
        run(&mut runtime, "write();")
            .unwrap_err()
            .message
            .contains("IoError: closed stream")
    );
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    runtime
        .register(
            "stop",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                |context, _| {
                    context
                        .cancellation
                        .as_ref()
                        .unwrap()
                        .store(true, Ordering::Relaxed);
                    context.check_cancelled()?;
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    assert!(
        run(&mut runtime, "try { stop(); } catch (Error $e) {}")
            .unwrap_err()
            .message
            .contains("interrupted")
    );
    flag.store(false, Ordering::Relaxed);
    run(&mut runtime, "$ok = true;").unwrap();
}

#[test]
fn registry_signature_metadata_is_available_to_hosts() {
    let function = constant(1);
    assert!(function.signature().parameters.is_empty());
    assert_eq!(function.signature().returns, NativeType::Int);
    let error = NativeError::type_error("bad argument");
    assert_eq!(error.to_string(), "TypeError: bad argument");
}
