use scr_runtime::{
    NativeFunction, NativeObject, NativeSignature, NativeType, Runtime, RuntimeError,
    RuntimeOptions, Value,
};
use std::{cell::RefCell, rc::Rc};

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, RuntimeError> {
    runtime.execute_source(source, &mut Vec::new())
}

#[test]
fn direct_and_escaped_methods_preserve_receivers_defaults_spreads_and_identity() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        interface Counter { function next($add: int = 1, ...$rest: int): int; }
        class Count implements Counter {
            $value = 0;
            $callback;
            function next($add: int = 1, ...$rest: int): int {
                $this->value += $add;
                foreach ($rest as $amount) { $this->value += $amount; }
                return $this->value;
            }
            function capture(): callable { return function () { return $this->value; }; }
            function identity(): Count { return $this; }
        }
        $a = new Count(); $b = new Count();
        $method = $a->next; $alias = $method;
        $same = $method === $alias; $different = $method !== $a->next;
        $otherReceiver = $method !== $b->next;
        $a->callback = $b->next;
        $first = $a->next(); $second = $method(...[2, 3, 4]);
        $third = ($a->next)(5); $field = $a->callback(7);
        $closure = $a->capture(); $held = $a->identity(); $a = null;
        $last = $method(); $captured = $closure();
    "#,
    )
    .unwrap();
    for (name, value) in [
        ("first", 1),
        ("second", 10),
        ("third", 15),
        ("field", 7),
        ("last", 16),
        ("captured", 16),
    ] {
        assert_eq!(runtime.get_global(name), Some(&Value::Int(value)), "{name}");
    }
    for name in ["same", "different", "otherReceiver"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Bool(true)), "{name}");
    }
    assert_eq!(run(&mut runtime, "$held->next(0);"), Ok(0));
}

#[derive(Debug)]
struct NativeReceiver(Rc<RefCell<Vec<&'static str>>>);
impl NativeObject for NativeReceiver {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.0.borrow_mut().push("downcast");
        None
    }
    fn type_name(&self) -> &str {
        "NativeReceiver"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        self.0.borrow_mut().push("property");
        let events = self.0.clone();
        Ok(Value::native_function(
            "nativeMethod",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                move |_, args| {
                    events.borrow_mut().push("call");
                    Ok(args[0].clone())
                },
            ),
        ))
    }
}

#[test]
fn native_properties_and_effectful_receivers_are_evaluated_once_before_arguments() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut runtime = Runtime::new();
    for (name, event) in [("receiver", "receiver"), ("argument", "argument")] {
        let events = events.clone();
        runtime
            .register(
                name,
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| {
                        events.borrow_mut().push(event);
                        Ok(if name == "receiver" {
                            Value::NativeObject(Rc::new(NativeReceiver(events.clone())))
                        } else {
                            Value::Int(9)
                        })
                    },
                ),
            )
            .unwrap();
    }
    run(&mut runtime, "$result = receiver()->method(argument());").unwrap();
    assert_eq!(
        *events.borrow(),
        ["receiver", "downcast", "property", "argument", "call"]
    );
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(9)));

    run(
        &mut runtime,
        r#"
        class Holder { $callback; function method($a, $b) { return [$a, $b]; } }
        function exercise() {
            $events = []; $holder = new Holder();
            $holder->callback = function ($v) { $events[] = 4; return $v; };
            function receiver() { $events[] = 1; return $holder; }
            function argument() { $events[] = 2; return 7; }
            $result = receiver()->method(argument(), argument());
            $field = receiver()->callback(argument());
            return [$result, $field, $events];
        }
        $order = exercise();
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("order"),
        Some(&Value::list(vec![
            Value::list(vec![Value::Int(7); 2]),
            Value::Int(7),
            Value::List([1, 2, 2, 1, 2, 4].map(Value::Int).into()),
        ]))
    );
}

#[test]
fn failed_resolution_precedes_arguments_and_failed_calls_preserve_destinations() {
    for callee in [
        "$object->missing",
        "($object->missing)",
        "$object->field",
        "($object->field)",
    ] {
        let mut runtime = Runtime::new();
        run(&mut runtime, "class Object { $field = 9; } $object = new Object(); $events = 0; $result = 'unchanged';").unwrap();
        let error = run(&mut runtime, &format!("$result = {callee}($events += 1);")).unwrap_err();
        assert!(
            error.message.contains("unknown method")
                || error.message.contains("requires a callable"),
            "{error}"
        );
        assert_eq!(runtime.get_global("events"), Some(&Value::Int(0)));
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
    for callee in ["$object->method", "($object->method)"] {
        let mut runtime = Runtime::new();
        run(&mut runtime, "class Object { function method($value: int): int { return $value; } } $object = new Object(); $events = 0; $result = 'unchanged';").unwrap();
        let error = run(
            &mut runtime,
            &format!("$result = {callee}($events += 1, $events += 1);"),
        )
        .unwrap_err();
        assert!(
            error
                .message
                .starts_with("TypeError: method() expects 1 arguments, got 2")
        );
        assert_eq!(error.trace.last().unwrap().function, "method");
        assert_eq!(runtime.get_global("events"), Some(&Value::Int(2)));
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
}

#[test]
fn direct_and_bound_calls_keep_definition_locations_across_executions() {
    let definition = "class Object { function method($value = 1 / 0): int { return $value; } } $object = new Object(); $method = $object->method;";
    let mut runtime = Runtime::new();
    run(&mut runtime, definition).unwrap();
    runtime.collect_garbage();
    for call in ["$object->method()", "$method()"] {
        let source = format!("$result = {call};");
        let error = run(&mut runtime, &source).unwrap_err();
        assert!(error.message.starts_with("ArithmeticError:"));
        assert_eq!(error.source.as_ref().unwrap().text, definition);
        assert_eq!(&definition[error.span.start..error.span.end], "1 / 0");
        let frame = error.trace.last().unwrap();
        assert_eq!(frame.function, "method");
        assert_eq!(frame.source.as_ref().unwrap().text, source);
        assert_eq!(&source[frame.span.start..frame.span.end], call);
    }
    run(&mut runtime, "$result = $object->method(7);").unwrap();
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(7)));
}

#[test]
fn foreign_receivers_and_bound_methods_are_rejected_before_invocation() {
    let mut owner = Runtime::new();
    run(&mut owner, "class Object { function method($value) { return $value; } } $object = new Object(); $method = $object->method;").unwrap();
    let mut other = Runtime::new();
    for name in ["object", "method"] {
        other
            .set_global(name, owner.get_global(name).unwrap().clone())
            .unwrap();
    }
    run(&mut other, "$events = 0; $result = 'unchanged';").unwrap();
    let error = run(&mut other, "$result = $object->method($events += 1);").unwrap_err();
    assert!(
        error
            .message
            .contains("object belongs to a different runtime")
    );
    assert_eq!(other.get_global("events"), Some(&Value::Int(0)));
    let error = run(&mut other, "$result = $method($events += 1);").unwrap_err();
    assert!(
        error
            .message
            .contains("user callable belongs to a different runtime")
    );
    assert_eq!(other.get_global("events"), Some(&Value::Int(1)));
    owner.collect_garbage();
    other.collect_garbage();
    run(&mut owner, "$result = $method(42);").unwrap();
    assert_eq!(owner.get_global("result"), Some(&Value::Int(42)));
    assert_eq!(
        other.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
}

#[test]
fn direct_and_bound_recursion_obey_identical_scope_and_call_limits() {
    for recursion_limit in [1, 2, 3, 4, 8] {
        for max_scopes in [3, 4, 5, 6, 7, 12, 20] {
            let mut outcomes = Vec::new();
            for call in ["$this->method", "($this->method)"] {
                let mut runtime = Runtime::with_options(RuntimeOptions {
                    recursion_limit,
                    max_scopes: Some(max_scopes),
                    ..Default::default()
                });
                let source = format!(
                    "class Object {{ function method($depth: int): int {{ if ($depth === 0) return 7; return {call}($depth - 1); }} }} $object = new Object(); $result = 'unchanged'; $result = $object->method(2);"
                );
                let outcome = run(&mut runtime, &source).map_err(|error| error.message);
                outcomes.push((outcome, runtime.get_global("result").cloned()));
            }
            assert_eq!(
                outcomes[0], outcomes[1],
                "recursion={recursion_limit}, scopes={max_scopes}"
            );
        }
    }
}

#[test]
fn cancellation_in_arguments_defaults_and_bodies_unwinds_direct_and_bound_calls() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for callee in ["$object->method", "($object->method)"] {
        for (method, arguments) in [
            ("function method($value) { return $value; }", "cancel()"),
            ("function method($value = cancel()) { return $value; }", ""),
            ("function method() { cancel(); }", ""),
        ] {
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
            run(&mut runtime, &format!("class Object {{ {method} }} $object = new Object(); $result = 'unchanged'; $caught = false;")).unwrap();
            let error = run(&mut runtime, &format!("try {{ $result = {callee}({arguments}); }} catch (Error $e) {{ $caught = true; }}")).unwrap_err();
            assert!(error.message.contains("execution interrupted"));
            assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
            assert_eq!(
                runtime.get_global("result"),
                Some(&Value::String("unchanged".into()))
            );
            flag.store(false, Ordering::Relaxed);
            run(&mut runtime, "$recovered = 42;").unwrap();
            assert_eq!(runtime.get_global("recovered"), Some(&Value::Int(42)));
        }
    }
}

#[test]
fn direct_and_bound_calls_preserve_instruction_and_exact_value_budgets() {
    for instruction_limit in [0, 1, 5, 10, 20, 30, 100] {
        let mut outcomes = Vec::new();
        for callee in ["$object->method", "($object->method)"] {
            let mut runtime = Runtime::with_options(RuntimeOptions {
                instruction_limit: Some(instruction_limit),
                ..Default::default()
            });
            let source = format!(
                "class Object {{ function method($value: int): int {{ $value += 1; return $value; }} }} $object = new Object(); $result = 0; for ($i = 0; $i < 5; $i += 1) {{ $result = {callee}(...[$i]); }}"
            );
            outcomes.push((
                run(&mut runtime, &source).map_err(|error| error.message),
                runtime.get_global("result").cloned(),
            ));
        }
        assert_eq!(
            outcomes[0], outcomes[1],
            "instruction limit {instruction_limit}"
        );
    }
    let exact = 3 * std::mem::size_of::<Value>();
    for limit in [exact - 1, exact, exact + 1] {
        for callee in ["$object->method", "($object->method)"] {
            let mut runtime = Runtime::with_options(RuntimeOptions {
                max_value_bytes: Some(limit),
                ..Default::default()
            });
            run(&mut runtime, "class Object { function method(): list { return [1, 2]; } } $object = new Object(); $result = 'unchanged';").unwrap();
            let outcome = run(&mut runtime, &format!("$result = {callee}();"));
            if limit < exact {
                assert!(outcome.unwrap_err().message.contains("value byte limit"));
                assert_eq!(
                    runtime.get_global("result"),
                    Some(&Value::String("unchanged".into()))
                );
            } else {
                outcome.unwrap();
                assert_eq!(
                    runtime.get_global("result"),
                    Some(&Value::list(vec![Value::Int(1), Value::Int(2)]))
                );
            }
        }
    }
}
