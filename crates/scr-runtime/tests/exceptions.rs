use scr_runtime::{
    ContextOptions, NativeError, NativeFunction, NativeSignature, NativeType, Runtime,
    RuntimeOptions, Value,
};

// Exception messages must not depend on secrets configured on the test host.
// Redaction tests inject their own environment values below.
fn options() -> RuntimeOptions {
    RuntimeOptions {
        context: ContextOptions {
            environment: Default::default(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn run(rt: &mut Runtime, source: &str) -> Result<u8, scr_runtime::RuntimeError> {
    rt.execute_source(source, &mut Vec::new())
}
fn string(rt: &Runtime, name: &str, expected: &str) {
    assert_eq!(rt.get_global(name), Some(&Value::String(expected.into())));
}

#[test]
fn core_and_native_failures_have_structured_read_only_properties() {
    let mut rt = Runtime::with_options(options());
    rt.register(
        "fail",
        NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
            Err(NativeError::new("CustomNativeError", "native failure"))
        }),
    )
    .unwrap();
    run(&mut rt, r#"
        $saved = null; $name = null; $message = null; $code = null; $file = null; $line = null; $column = null; $trace = null; $cause = null; $native = null; $nativeName = null;
        function inner() { 1 / 0; }
        try { inner(); } catch (ArithmeticError $e) {
            $saved = $e; $name = $e->name; $message = $e->message;
            $code = $e->code; $file = $e->file; $line = $e->line; $column = $e->column;
            $trace = $e->trace; $cause = $e->cause;
        }
        $readonly = 0;
        try { $saved->message = "changed"; } catch (PropertyError $e) { $readonly += 1; }
        try { $saved->trace[] = 7; } catch (PropertyError $e) { $readonly += 1; }
        try { fail(); } catch (CustomNativeError $e) { $native = $e->message; $nativeName = $e->name; }
    "#).unwrap();
    string(&rt, "name", "ArithmeticError");
    string(&rt, "message", "division by zero");
    string(&rt, "code", "E0401");
    string(&rt, "native", "native failure");
    string(&rt, "nativeName", "CustomNativeError");
    assert_eq!(rt.get_global("readonly"), Some(&Value::Int(2)));
    assert_eq!(rt.get_global("cause"), Some(&Value::Null));
    assert!(matches!(rt.get_global("line"), Some(Value::Int(n)) if *n > 0));
    assert!(matches!(rt.get_global("trace"), Some(Value::List(frames)) if frames.len() == 2));
}

#[test]
fn exception_classes_inherit_fields_methods_constructors_and_annotations() {
    let mut rt = Runtime::with_options(options());
    run(
        &mut rt,
        r#"
        interface Described { function describe(): string; }
        class ValidationError extends Error implements Described {
            $field: string = "email";
            function __construct($message: string) { $this->field = "address"; }
            function describe(): string { return $this->field . ": " . $this->message; }
        }
        class MissingEmail extends ValidationError {}
        function accept($e: ValidationError): Error { return $e; }
        function described($e: Described): string { return $e->describe(); }
        $original = new MissingEmail("required");
        $before = $original->line; $beforeTrace = $original->trace; $code = $original->code;
        $description = described($original);
        $selected = "none"; $same = false;
        try { throw accept($original); }
        catch (IoError $e) { $selected = "wrong"; }
        catch (ValidationError $e) { $selected = $e->name; $same = $e === $original; }
        catch (MissingEmail $e) { $selected = "late"; }
    "#,
    )
    .unwrap();
    string(&rt, "selected", "MissingEmail");
    string(&rt, "description", "address: required");
    assert_eq!(rt.get_global("same"), Some(&Value::Bool(true)));
    assert_eq!(rt.get_global("before"), Some(&Value::Null));
    assert_eq!(rt.get_global("code"), Some(&Value::Null));
    assert_eq!(rt.get_global("beforeTrace"), Some(&Value::List(vec![])));
}

#[test]
fn rethrow_preserves_identity_origin_and_trace_and_wrapping_preserves_cause() {
    let mut rt = Runtime::with_options(options());
    let source = r#"
        class Wrapped extends Error {}
        $saved = null; $line = null; $trace = null; $same = false; $sameLine = false; $sameTrace = false; $wrapped = null; $wrappedCode = null;
        function fail() { 1 / 0; }
        try {
            try { fail(); } catch (ArithmeticError $e) {
                $saved = $e; $line = $e->line; $trace = $e->trace;
                throw $e;
            }
        } catch (ArithmeticError $e) {
            $same = $saved === $e; $sameLine = $line === $e->line; $sameTrace = $trace === $e->trace;
            $wrapped = new Wrapped("outer", $e);
        }
        $causeSame = $wrapped->cause === $saved;
        $causeMessage = $wrapped->cause->message;
        try { throw $wrapped; } catch (Error $e) { $wrappedCode = $e->code; }
    "#;
    run(&mut rt, source).unwrap();
    for name in ["same", "sameLine", "sameTrace", "causeSame"] {
        assert_eq!(rt.get_global(name), Some(&Value::Bool(true)), "{name}");
    }
    string(&rt, "causeMessage", "division by zero");
    assert_eq!(rt.get_global("wrappedCode"), Some(&Value::Null));
    let error = run(&mut rt, "throw $saved;").unwrap_err();
    assert_eq!(&source[error.span.start..error.span.end], "1 / 0");
    assert_eq!(error.source.as_ref().unwrap().text, source);
}

#[test]
fn invalid_hierarchies_members_and_constructor_arguments_are_rejected() {
    for source in [
        "class A extends B {} class B extends A {}",
        "class Plain {} class Invalid extends Plain {}",
        "class Invalid extends Missing {}",
        "class Invalid extends Error {$message;}",
        "class Invalid extends Error {function trace() {}}",
        "class Base extends Error {$field;} class Child extends Base {$field;}",
        "class Error {}",
        "new Error();",
        "new Error(1);",
        "new Error('x', 1);",
        "new Error('x', null, 3);",
    ] {
        assert!(
            run(&mut Runtime::with_options(options()), source).is_err(),
            "{source}"
        );
    }
    let source = (0..70)
        .map(|i| format!("class E{i} extends E{} {{}} ", i + 1))
        .collect::<String>()
        + "class E70 extends Error {}";
    assert!(
        run(&mut Runtime::with_options(options()), &source)
            .unwrap_err()
            .message
            .contains("64")
    );
}

#[test]
fn arbitrary_values_keep_legacy_catches_and_finally_keeps_control_flow() {
    let mut rt = Runtime::with_options(options());
    run(
        &mut rt,
        r#"
        $order = ""; $number = null; $raw = null; $message = null; $cause = null;
        try { try { throw 42; } catch (string $e) { $order = "wrong"; } }
        catch (int $e) { $number = $e; } finally { $order .= "f"; }
        try { throw "raw"; } catch (Error $e) { $raw = $e; }
        try { try { throw new Error("first"); } finally { throw new TypeError("replacement"); } }
        catch (TypeError $e) { $message = $e->message; $cause = $e->cause; }
    "#,
    )
    .unwrap();
    string(&rt, "order", "f");
    string(&rt, "raw", "raw");
    string(&rt, "message", "replacement");
    assert_eq!(rt.get_global("number"), Some(&Value::Int(42)));
    assert_eq!(rt.get_global("cause"), Some(&Value::Null));
}

#[test]
fn messages_and_locations_are_redacted_and_cause_depth_is_bounded() {
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            redacted_values: vec!["private-token".into()],
            environment: [("SCR_TEST_SECRET".into(), "root".into())].into(),
            ..Default::default()
        },
        ..options()
    });
    run(
        &mut rt,
        "$e = new Error('private-token'); $message = $e->message;",
    )
    .unwrap();
    let Some(Value::String(message)) = rt.get_global("message") else {
        panic!()
    };
    assert!(!message.contains("private-token"));
    run(&mut rt, "$e = new Error('root'); $message = $e->message;").unwrap();
    string(&rt, "message", "[REDACTED]");
    run(
        &mut rt,
        "$e = new Error('root'); $i = 0; while ($i < 128) {$e = new Error('next', $e); $i += 1;}",
    )
    .unwrap();
    assert!(
        run(&mut rt, "new Error('too deep', $e);")
            .unwrap_err()
            .message
            .contains("128")
    );
}

#[test]
fn exact_value_budget_and_budget_plus_one_preserve_fatal_boundaries() {
    let limit = 2 * std::mem::size_of::<Value>() + 1; // value, cause handle, message byte
    let mut exact = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(limit),
        ..options()
    });
    run(&mut exact, "$e = new Error('x');").unwrap();
    let mut short = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(limit - 1),
        ..options()
    });
    let error = run(&mut short, "$saved = 'kept'; $caught = false; $cleanup = false; try {$saved = new Error('x');} catch (Error $e) {$caught = true;} finally {$cleanup = true;}").unwrap_err();
    assert!(error.message.contains("value byte limit"));
    string(&short, "saved", "kept");
    assert_eq!(short.get_global("caught"), Some(&Value::Bool(false)));
    assert_eq!(short.get_global("cleanup"), Some(&Value::Bool(false)));
    // First throw adds diagnostic/trace payload and must also respect the budget.
    assert!(
        run(&mut exact, "throw $e;")
            .unwrap_err()
            .message
            .contains("value byte limit")
    );
    run(&mut exact, "$line = $e->line;").unwrap();
    assert_eq!(exact.get_global("line"), Some(&Value::Null));
}

#[test]
fn causes_and_user_field_cycles_are_collected_without_losing_host_roots() {
    use std::{
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    #[derive(Debug)]
    struct Resource(Arc<AtomicUsize>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl scr_runtime::NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("no properties".into())
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let counter = drops.clone();
    let mut rt = Runtime::with_options(options());
    rt.register(
        "resource",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Any),
            move |_, _| Ok(Value::NativeObject(Rc::new(Resource(counter.clone())))),
        ),
    )
    .unwrap();
    run(&mut rt, "class LinkedError extends Error {$link; $resource = resource();} $root = new LinkedError('root'); $wrapper = new Error('wrapper', $root); $root->link = $wrapper;").unwrap();
    let host = rt.get_global("wrapper").unwrap().clone();
    run(&mut rt, "$root = null; $wrapper = null;").unwrap();
    rt.collect_garbage();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    rt.set_global("restored", host.clone()).unwrap();
    run(&mut rt, "$message = $restored->cause->message;").unwrap();
    string(&rt, "message", "root");
    run(&mut rt, "$restored = null;").unwrap();
    drop(host);
    rt.collect_garbage();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn inheritance_accepts_the_exact_depth_and_rejects_one_more() {
    let mut rt = Runtime::with_options(options());
    let source = (0..62)
        .map(|i| format!("class E{i} extends E{} {{}} ", i + 1))
        .collect::<String>()
        + "class E62 extends Error {} $e = new E0('ok');";
    run(&mut rt, &source).unwrap();
    assert!(
        run(&mut rt, "class TooDeep extends E0 {}")
            .unwrap_err()
            .message
            .contains("64")
    );
}

#[test]
fn cancellation_during_cause_validation_bypasses_catches_and_finally() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut rt = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..options()
    });
    let trigger = flag.clone();
    rt.register(
        "cancel",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Any], NativeType::Any),
            move |_, args| {
                trigger.store(true, Ordering::SeqCst);
                Ok(args[0].clone())
            },
        ),
    )
    .unwrap();
    let error = run(&mut rt, "$e = new Error('root'); $caught = false; $cleanup = false; try {new Error('wrapper', cancel($e));} catch (Error $e) {$caught = true;} finally {$cleanup = true;}").unwrap_err();
    assert!(error.message.contains("interrupted"));
    assert_eq!(rt.get_global("caught"), Some(&Value::Bool(false)));
    assert_eq!(rt.get_global("cleanup"), Some(&Value::Bool(false)));
    flag.store(false, Ordering::SeqCst);
    run(&mut rt, "$message = $e->message;").unwrap();
    string(&rt, "message", "root");
}
