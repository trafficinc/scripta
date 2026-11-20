use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn run(rt: &mut Runtime, source: &str) -> Result<String, scr_runtime::RuntimeError> {
    let mut output = Vec::new();
    rt.execute_source(source, &mut output)?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn interpolation_evaluates_fields_indexes_calls_arithmetic_and_nested_strings_in_order() {
    let source = r#"
class User { $name = "Ada"; }
$user = new User(); $items = ["first"]; $price = 25; $quantity = 4;
$yes = true; $no = false; $ratio = 1.25; $name = "é🦀"; $i = 0;
function identity($value) { return $value; }
$call = identity;
echo "Hello {$user->name}; {$items[0]}; {$price * $quantity}; {$yes}/{$no}; {$ratio}; {$name}\n";
echo "{$i += 1},{$i += 1},{$call("{$name}")},{$items[0] . "}"}\n";
echo '\'{$name}\'' . " $name \{$name} {\$name} {ordinary}\n";
echo "{$call(function () { return "brace }"; })()}";
"#;
    let expected = "Hello Ada; first; 100; true/false; 1.25; é🦀\n1,2,é🦀,first}\n'{$name}' $name {$name} {$name} {ordinary}\nbrace }";
    let formatted = scr_cli::formatter::format(source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    for input in [source, &formatted] {
        let mut rt = Runtime::new();
        assert_eq!(run(&mut rt, input).unwrap(), expected);
        assert_eq!(rt.get_global("i"), Some(&Value::Int(2)));
    }
}

#[test]
fn interpolation_type_errors_preserve_destinations_and_original_error_locations() {
    for expression in ["null", "[]", "['k'=>1]", "function () {}", "new User()"] {
        let mut rt = Runtime::new();
        let source = format!(
            r#"class User {{}} $saved='kept'; $bad={expression}; try {{$saved="value {{$bad}}";}} catch (TypeError $e) {{}}"#
        );
        run(&mut rt, &source).unwrap();
        assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
        let error = run(&mut rt, r#"$saved="{$bad}";"#).unwrap_err();
        assert!(error.message.contains("TypeError"));
    }
    let mut rt = Runtime::new();
    let source = "$saved='kept';\necho \"é {$missing}\";";
    let error = run(&mut rt, source).unwrap_err();
    assert_eq!(&source[error.span.start..error.span.end], "$missing");
    assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
    let source = "$x=1;\necho \"é {$x + }\";";
    let errors = scr_cli::pipeline::check(source).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| source[e.span.start..e.span.end].is_empty()
                && e.span.start == source.find('}').unwrap())
    );
}

#[test]
fn interpolation_checks_exact_byte_limits_and_cancellation_without_partial_assignment() {
    let exact = std::mem::size_of::<Value>() + 6;
    for limit in [exact, exact - 1] {
        let mut rt = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        run(&mut rt, "$saved='kept'; $x='🦀';").unwrap();
        let result = run(&mut rt, r#"$saved="a{$x}b";"#);
        if limit == exact {
            result.unwrap();
            assert_eq!(rt.get_global("saved"), Some(&Value::String("a🦀b".into())));
        } else {
            assert!(result.unwrap_err().message.contains("value byte limit"));
            assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
        }
    }
    let mut rt = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(5),
        ..Default::default()
    });
    run(&mut rt, "$saved='kept'; $x='a';").unwrap();
    assert!(
        run(&mut rt, r#"$saved="{$x}{$x}{$x}{$x}";"#)
            .unwrap_err()
            .message
            .contains("instruction limit")
    );
    assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
    let flag = Arc::new(AtomicBool::new(false));
    let trigger = flag.clone();
    let mut rt = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag),
        ..Default::default()
    });
    rt.register(
        "cancel",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::String),
            move |_, _| {
                trigger.store(true, Ordering::SeqCst);
                Ok(Value::String("cancelled".into()))
            },
        ),
    )
    .unwrap();
    run(&mut rt, "$saved='kept'; $call=cancel;").unwrap();
    assert!(run(&mut rt, r#"$saved="{$call()} next";"#).is_err());
    assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
}

#[test]
fn interpolation_visitors_validate_embedded_function_bodies() {
    let source = r#"$f=function($x){return $x;}; echo "{$f(function($x,$x){})}";"#;
    assert!(scr_cli::pipeline::check(source).is_err());
}

#[test]
fn interpolation_rejects_bytes_and_retains_side_effects_before_failure() {
    let mut rt = Runtime::new();
    rt.set_global("bytes", Value::Bytes(std::rc::Rc::from([0xffu8])))
        .unwrap();
    let error = run(
        &mut rt,
        r#"$i=0; $saved='kept'; $saved="{$i += 1}{$bytes}";"#,
    )
    .unwrap_err();
    assert!(error.message.contains("TypeError"));
    assert_eq!(rt.get_global("i"), Some(&Value::Int(1)));
    assert_eq!(rt.get_global("saved"), Some(&Value::String("kept".into())));
}

#[test]
fn interpolation_in_modules_keeps_lexical_state_source_and_stack_traces() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("messages.scripta");
    let module = r#"$name='module'; function greet() {return "Hello {$name}";} function broken() {return "{$missing}";}"#;
    std::fs::write(&path, module).unwrap();
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: scr_runtime::ContextOptions {
            script_path: Some(directory.path().join("main.scripta")),
            ..Default::default()
        },
        ..Default::default()
    });
    run(
        &mut rt,
        r#"$name='caller'; require 'messages.scripta' as Messages; $message=Messages::greet();"#,
    )
    .unwrap();
    assert_eq!(
        rt.get_global("message"),
        Some(&Value::String("Hello module".into()))
    );
    let error = run(&mut rt, "Messages::broken();").unwrap_err();
    assert_eq!(
        error.source.as_ref().unwrap().path,
        path.canonicalize().unwrap()
    );
    assert_eq!(&module[error.span.start..error.span.end], "$missing");
    assert!(error.trace.iter().any(|frame| frame.function == "broken"));
}
