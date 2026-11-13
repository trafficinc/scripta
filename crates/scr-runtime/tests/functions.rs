use scr_runtime::{
    NativeFunction, NativeSignature, NativeType, Runtime, RuntimeError, RuntimeOptions, Value,
};

fn run(runtime: &mut Runtime, body: &str) -> Result<u8, RuntimeError> {
    runtime.execute_source(&format!("<?scr {body}"), &mut Vec::new())
}

fn ints(values: &[i64]) -> Value {
    Value::List(values.iter().copied().map(Value::Int).collect())
}

#[test]
fn defaults_evaluate_per_call_in_parameter_and_lexical_scope() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        function pair($a = 1, $b = $a + 1) { return [$a, $b]; }
        $first = pair(); $second = pair(5); $third = pair(5, 9);
        function nullable($value = 1) { return $value; } $null = nullable(null);
        function factory($seed) {
            function next() { $seed += 1; return $seed; }
            return function ($value = next(), $other = $value + 1) { return [$value, $other]; };
        }
        $f = factory(10); $one = $f(); $supplied = $f(50); $two = $f();
        function order() {
            $events = [];
            function supplied() { $events[] = 1; return 10; }
            function missing() { $events[] = 2; return 20; }
            $f = function ($a, $b = missing()) { return [$a, $b]; };
            $f(supplied()); return $events;
        }
        $order = order();
    "#,
    )
    .unwrap();
    for (name, expected) in [
        ("first", vec![1, 2]),
        ("second", vec![5, 6]),
        ("third", vec![5, 9]),
        ("one", vec![11, 12]),
        ("supplied", vec![50, 51]),
        ("two", vec![12, 13]),
        ("order", vec![1, 2]),
    ] {
        assert_eq!(runtime.get_global(name), Some(&ints(&expected)), "{name}");
    }
    assert_eq!(runtime.get_global("null"), Some(&Value::Null));
}

#[test]
fn mutable_defaults_and_variadic_arguments_have_independent_values() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        function fresh($items = [[1]]) { $items[0][] = 2; return $items; }
        $first = fresh(); $second = fresh(); $original = [[9]]; $copy = fresh($original);
        function extra(...$items: list): list { $items[0][] = 2; return $items; }
        $source = [1]; $extras = extra($source);
        function rest($first = 9, ...$values: int): list { return [$first, $values]; }
        $empty = rest(); $many = rest(1, 2, 3); $noExtras = rest(1);
        function all(...$values): list { return $values; } $all = all(1, "two", null);
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("first"),
        Some(&Value::List(vec![ints(&[1, 2])]))
    );
    assert_eq!(runtime.get_global("second"), runtime.get_global("first"));
    assert_eq!(
        runtime.get_global("original"),
        Some(&Value::List(vec![ints(&[9])]))
    );
    assert_eq!(
        runtime.get_global("copy"),
        Some(&Value::List(vec![ints(&[9, 2])]))
    );
    assert_eq!(runtime.get_global("source"), Some(&ints(&[1])));
    assert_eq!(
        runtime.get_global("extras"),
        Some(&Value::List(vec![ints(&[1, 2])]))
    );
    assert_eq!(
        runtime.get_global("empty"),
        Some(&Value::List(vec![Value::Int(9), ints(&[])]))
    );
    assert_eq!(
        runtime.get_global("many"),
        Some(&Value::List(vec![Value::Int(1), ints(&[2, 3])]))
    );
    assert_eq!(
        runtime.get_global("noExtras"),
        Some(&Value::List(vec![Value::Int(1), ints(&[])]))
    );
    assert_eq!(
        runtime.get_global("all"),
        Some(&Value::List(vec![
            Value::Int(1),
            Value::String("two".into()),
            Value::Null
        ]))
    );
}

#[test]
fn anonymous_callbacks_capture_shared_locals_and_support_recursion() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        function apply($callback: callable, $value: int): int { return $callback($value); }
        $callback = apply(function ($n: int): int { return $n * 2; }, 7);
        $immediate = (function ($n = 4) { return $n; })();
        function counters() {
            $n = 0; $inc = function () { $n += 1; return $n; };
            $get = function () { return $n; }; return [$inc, $get];
        }
        $pair = counters(); $n = $pair[0](); $current = $pair[1]();
        function recursive() {
            $f = function ($n: int = 5): int {
                if ($n === 0) return 1;
                return $n * $f($n - 1);
            }; return $f;
        }
        $factorial = recursive(); $answer = $factorial();
        function named($n = 3) { if ($n === 0) return 0; return 1 + named($n - 1); }
        $named = named();
        $f = function () {}; $same = $f === $f; $different = $f === function () {};
        $nested = function ($f = function ($n = 8) { return $n; }) { return $f(); };
        $nestedResult = $nested();
    "#,
    )
    .unwrap();
    for (name, expected) in [
        ("callback", 14),
        ("immediate", 4),
        ("n", 1),
        ("current", 1),
        ("answer", 120),
        ("named", 3),
        ("nestedResult", 8),
    ] {
        assert_eq!(
            runtime.get_global(name),
            Some(&Value::Int(expected)),
            "{name}"
        );
    }
    assert_eq!(runtime.get_global("same"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("different"), Some(&Value::Bool(false)));
}

#[test]
fn defaults_and_anonymous_functions_preserve_global_and_caller_isolation() {
    for body in [
        "$global = 9; $f = function () { return $global; }; $f();",
        "$global = 9; function f($value = $global) { return $value; } f();",
        "function f($value = $caller) {} function caller() { $caller = 9; f(); } caller();",
    ] {
        let error = run(&mut Runtime::new(), body).unwrap_err();
        assert!(
            error.message.contains("undefined variable"),
            "{body}: {error}"
        );
    }
}

#[test]
fn arity_default_variadic_and_return_annotations_are_catchable() {
    for body in [
        "function f($a, $b = 1) {} f();",
        "function f($a = 1) {} f(1, 2);",
        "function f($a, ...$rest) {} f();",
        "function f($a: int = 'wrong') {} f();",
        "function f($a: int = 1) {} f(null);",
        "function f(...$values: int) {} f(1, 2.0);",
        "$f = function ($a: int = 1): int { return 'wrong'; }; $f();",
        "$f = function ($a: int = 1) {}; $f(true);",
    ] {
        let mut runtime = Runtime::new();
        run(
            &mut runtime,
            &format!(
                "$caught = false; try {{ {body} }} catch (TypeError $e) {{ $caught = true; }}"
            ),
        )
        .unwrap();
        assert_eq!(
            runtime.get_global("caught"),
            Some(&Value::Bool(true)),
            "{body}"
        );
        assert!(runtime.get_global("e").is_none());
        run(&mut runtime, "$recovered = true;").unwrap();
    }
}

#[test]
fn default_failures_retain_definition_spans_and_caller_frames_across_executions() {
    let mut runtime = Runtime::new();
    let definition = "<?scr $f = function ($value = 1 / 0) { return $value; };";
    runtime.execute_source(definition, &mut Vec::new()).unwrap();
    let call = "<?scr $f();";
    let error = runtime.execute_source(call, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with("ArithmeticError:"));
    assert_eq!(error.source.as_ref().unwrap().text, definition);
    assert_eq!(&definition[error.span.start..error.span.end], "1 / 0");
    let frame = error.trace.last().unwrap();
    assert_eq!(frame.function, "<anonymous>");
    assert_eq!(frame.source.as_ref().unwrap().text, call);
    run(&mut runtime, "$result = $f(7);").unwrap();
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(7)));
}

#[test]
fn variadic_wrapping_and_captured_scopes_obey_runtime_limits() {
    let mut bytes = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(8 * std::mem::size_of::<Value>()),
        ..Default::default()
    });
    let error = run(&mut bytes, "function f(...$values) {} $caught = false; try { f(1,2,3,4,5,6,7,8); } catch (Error $e) { $caught = true; }").unwrap_err();
    assert!(error.message.contains("byte limit"));
    assert_eq!(bytes.get_global("caught"), Some(&Value::Bool(false)));
    run(&mut bytes, "f(1);").unwrap();
    let mut depth = Runtime::new();
    let error = run(&mut depth, "$nested = null; for ($i = 0; $i < 128; $i += 1) { $nested = [$nested]; } function f(...$values) {} f($nested);").unwrap_err();
    assert!(error.message.contains("nesting exceeds 128"));
    let mut scopes = Runtime::with_options(RuntimeOptions {
        max_scopes: Some(12),
        ..Default::default()
    });
    let error = run(&mut scopes, "function factory() { return function () {}; } $kept=[]; for ($i = 0; $i < 100; $i += 1) { $kept[]=factory(); }").unwrap_err();
    assert!(error.message.contains("scope limit"));
    let mut recursion = Runtime::with_options(RuntimeOptions {
        recursion_limit: 4,
        ..Default::default()
    });
    let error = run(&mut recursion, "function f($value = f()) {} f();").unwrap_err();
    assert!(error.message.contains("maximum call depth"));
}

#[test]
fn default_evaluation_preserves_cancellation_and_instruction_limits() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    let trigger = flag.clone();
    runtime
        .register(
            "cancel",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                move |_, _| {
                    trigger.store(true, Ordering::Relaxed);
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    let error = run(&mut runtime, "function f($value = cancel()) {} $caught = false; try { f(); } catch (Error $e) { $caught = true; }").unwrap_err();
    assert!(error.message.contains("interrupted"));
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
    flag.store(false, Ordering::Relaxed);
    run(&mut runtime, "f(1);").unwrap();
    let mut runtime = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(30),
        ..Default::default()
    });
    let error = run(&mut runtime, "function spin() { while (true) {} } function f($value = spin()) {} $caught = false; try { f(); } catch (Error $e) { $caught = true; }").unwrap_err();
    assert!(error.message.contains("instruction limit"));
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
    run(&mut runtime, "f(1);").unwrap();
}
