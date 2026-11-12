use scr_runtime::{Runtime, RuntimeOptions, Value};
fn run(runtime: &mut Runtime, body: &str) -> Result<String, scr_runtime::RuntimeError> {
    let mut output = Vec::new();
    runtime.execute_source(&format!("<?scr {body}"), &mut output)?;
    Ok(String::from_utf8(output).unwrap())
}
#[test]
fn optimized_reads_writes_and_concat_preserve_snapshots_and_evaluation_order() {
    let mut runtime = Runtime::new();
    runtime
        .register(
            "str",
            scr_runtime::NativeFunction::new(
                scr_runtime::NativeSignature::new(
                    vec![scr_runtime::NativeType::Int],
                    scr_runtime::NativeType::String,
                ),
                |_, args| {
                    let Value::Int(value) = args[0] else {
                        unreachable!()
                    };
                    Ok(Value::String(value.to_string()))
                },
            ),
        )
        .unwrap();
    let result=run(&mut runtime, r#"
        $a=[10,20]; $b=$a; $a[0]=99; echo str($b[0]) . ":" . str(($a)[0]);
        function snapshot() { $a=[10]; function index() { $a[0]=99; return 0; } return $a[index()]; }
        function concat() { $s="a"; function change() { $s="b"; return "c"; } $s .= change(); return $s; }
        echo ":" . str(snapshot()) . ":" . concat();
        $s="x"; $copy=$s; $s .= "y"; $result=($s .= "z"); echo ":" . $copy . ":" . $result . ":" . $s;
    "#).unwrap();
    assert_eq!(result, "10:99:10:ac:x:xyz:xyz");
}
#[test]
fn runtime_value_depth_is_bounded_and_failure_does_not_poison_the_runtime() {
    let mut runtime = Runtime::new();
    let error = run(
        &mut runtime,
        "$a=null; for ($i=0;$i<300;$i+=1) { $a=[$a]; }",
    )
    .unwrap_err();
    assert!(error.message.contains("nesting exceeds 128"));
    assert_eq!(run(&mut runtime, "echo 'recovered';").unwrap(), "recovered");
}
#[test]
fn configured_bytes_scopes_steps_and_source_limits_are_enforced() {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(std::mem::size_of::<Value>() + 8),
        ..Default::default()
    });
    assert!(
        run(&mut runtime, r#"$s="12345678"; $s .= "x";"#)
            .unwrap_err()
            .message
            .contains("byte limit")
    );
    assert_eq!(
        runtime.get_global("s"),
        Some(&Value::String("12345678".into()))
    );
    let mut scopes = Runtime::with_options(RuntimeOptions {
        max_scopes: Some(3),
        ..Default::default()
    });
    assert!(
        run(&mut scopes, "{ { {} } }")
            .unwrap_err()
            .message
            .contains("scope limit")
    );
    assert_eq!(run(&mut scopes, "echo 'ok';").unwrap(), "ok");
    let mut steps = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(20),
        ..Default::default()
    });
    assert!(
        run(&mut steps, "while (true) {}")
            .unwrap_err()
            .message
            .contains("instruction limit")
    );
    let mut source = Runtime::with_options(RuntimeOptions {
        max_source_bytes: 8,
        ..Default::default()
    });
    assert!(
        run(&mut source, "echo 'too long';")
            .unwrap_err()
            .message
            .contains("source byte limit")
    );
}
#[test]
fn list_growth_obeys_value_budgets_without_partial_mutation() {
    for assignment in ["$items[] = 3;", "$items[2] = 3;"] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(3 * std::mem::size_of::<Value>()),
            ..Default::default()
        });
        run(&mut runtime, "$items = [1]; $items[] = 2;").unwrap();
        let error = run(&mut runtime, assignment).unwrap_err();
        assert!(error.message.contains("byte limit"));
        assert_eq!(
            runtime.get_global("items"),
            Some(&Value::List(vec![Value::Int(1), Value::Int(2)]))
        );
    }
}

#[test]
fn list_append_obeys_nesting_limits_without_partial_mutation() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        "$nested = []; for ($i = 0; $i < 128; $i += 1) { $nested = [$nested]; } $items = [];",
    )
    .unwrap();
    let error = run(&mut runtime, "$items[] = $nested;").unwrap_err();
    assert!(error.message.contains("nesting exceeds 128"));
    assert_eq!(runtime.get_global("items"), Some(&Value::List(vec![])));
}

#[test]
fn retained_closure_scopes_can_be_limited() {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_scopes: Some(12),
        instruction_limit: Some(1000),
        ..Default::default()
    });
    let error=run(&mut runtime,"function factory() { function inner() { return 1; } return inner; } for ($i=0;$i<100;$i+=1) { $f=factory(); }").unwrap_err();
    assert!(error.message.contains("scope limit"));
}
#[test]
fn diagnostics_redact_known_credentials_literal_excerpts_and_debug_source_text() {
    let mut options = RuntimeOptions::default();
    options
        .context
        .environment
        .insert("API_TOKEN".into(), "sensitive-value-123".into());
    let mut runtime = Runtime::with_options(options);
    let error = run(&mut runtime, "throw 'sensitive-value-123';").unwrap_err();
    for rendered in [
        error.to_string(),
        format!("{error:?}"),
        error.render("test.scr", "<?scr throw 'sensitive-value-123';"),
    ] {
        assert!(!rendered.contains("sensitive-value-123"), "{rendered}");
    }
    let error = run(&mut runtime, "$x='unmarked-sensitive-literal'; echo 1;").unwrap_err();
    assert!(
        !error
            .render("test.scr", "unused")
            .contains("unmarked-sensitive-literal")
    );
}

#[test]
fn native_resources_in_uncaptured_scopes_drop_after_exceptions() {
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Resource(Rc<Cell<usize>>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl scr_runtime::NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("unknown property".into())
        }
    }
    let dropped = Rc::new(Cell::new(0));
    let counter = dropped.clone();
    let mut runtime = Runtime::new();
    runtime
        .register(
            "resource",
            scr_runtime::NativeFunction::new(
                scr_runtime::NativeSignature::new(
                    vec![],
                    scr_runtime::NativeType::Object("Resource"),
                ),
                move |_, _| Ok(Value::NativeObject(Rc::new(Resource(counter.clone())))),
            ),
        )
        .unwrap();
    run(
        &mut runtime,
        "function f() { $r=resource(); throw 'boom'; } try { f(); } catch ($e) {}",
    )
    .unwrap();
    assert_eq!(dropped.get(), 1);
    run(
        &mut runtime,
        "function g() { $r=resource(); 1 / 0; } try { g(); } catch (ArithmeticError $e) {}",
    )
    .unwrap();
    assert_eq!(dropped.get(), 2);
}

#[test]
fn literal_temporaries_and_native_properties_also_obey_value_limits() {
    use std::rc::Rc;
    #[derive(Debug)]
    struct Big;
    impl scr_runtime::NativeObject for Big {
        fn type_name(&self) -> &str {
            "Big"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Ok(Value::String("x".repeat(1024)))
        }
    }
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(128),
        ..Default::default()
    });
    let source = format!("echo \"{}\";", "x".repeat(1024));
    assert!(
        run(&mut runtime, &source)
            .unwrap_err()
            .message
            .contains("byte limit")
    );
    runtime
        .set_global("big", Value::NativeObject(Rc::new(Big)))
        .unwrap();
    assert!(
        run(&mut runtime, "echo $big->text;")
            .unwrap_err()
            .message
            .contains("byte limit")
    );
}
