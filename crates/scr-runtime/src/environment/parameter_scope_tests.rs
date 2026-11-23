use super::*;
use crate::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeError, RuntimeOptions};

fn reserved(env: &Environments) -> impl Iterator<Item = usize> + '_ {
    env.spare_scopes.iter().filter_map(|entry| match entry {
        SpareScope::Reserved(id) => Some(*id),
        SpareScope::Storage(_) => None,
    })
}

fn compare(source: &str, options: RuntimeOptions) -> (Runtime, Result<u8, RuntimeError>) {
    let mut fast = Runtime::new();
    let mut old = Runtime::new();
    old.environments.disable_parameter_reuse = true;
    // Warm a general call before applying limits, so failure checks exercise a
    // reserved record as well as the usual body-scope pool.
    for runtime in [&mut fast, &mut old] {
        runtime
            .execute_source(
                "function warm($x){if(false){return 0;}return $x;}warm(1);warm(2);",
                &mut Vec::new(),
            )
            .unwrap();
        runtime.options = options.clone();
    }
    let mut first = Ok(0);
    for (index, text) in [source, "$recovered=7;"].into_iter().enumerate() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let result = fast.execute_source(text, &mut a);
        assert_eq!(result, old.execute_source(text, &mut b), "{source}");
        assert_eq!(a, b);
        assert_eq!(fast.steps, old.steps);
        assert_eq!(fast.environments.live, old.environments.live);
        assert_eq!(fast.environments.current, 0);
        assert_eq!(old.environments.current, 0);
        assert!(fast.stack.is_empty() && old.stack.is_empty());
        assert!(fast.argument_buffers.iter().all(Vec::is_empty));
        for (name, value) in &fast.environments.scope(0).variables {
            if !matches!(value, Value::Callable(_) | Value::NativeObject(_)) {
                assert_eq!(Some(value), old.get_global(name), "{name}");
            }
        }
        if index == 0 {
            first = result;
        }
    }
    (fast, first)
}

#[test]
fn recursion_defaults_methods_and_captures_keep_current_bindings() {
    for (source, expected) in [
        (
            "function fib($n:int):int{if($n<2){return $n;}return fib($n-1)+fib($n-2);}$seen=fib(12);",
            Value::Int(144),
        ),
        (
            "function f($a=1,$b=($a+=2),...$rest){return [$a,$b,$rest];}$seen=f();",
            Value::List(vec![Value::Int(3), Value::Int(3), Value::List(vec![])]),
        ),
        (
            "function make($x,$read=function(){return $x;}){return $read;}$f=make(19);warm(3);warm(4);$seen=$f();",
            Value::Int(19),
        ),
        (
            "function make($x){return function($d=1){return $x+=$d;};}$f=make(10);$g=make(100);$a=$f();$b=$g();$seen=$f(2);",
            Value::Int(13),
        ),
        (
            "class Base extends Error{function f($n:int):int{if($n===0){return 1;}return $n*$this->f($n-1);}}class C extends Base{}$c=new C('counter');$f=$c->f;$seen=$f(8);",
            Value::Int(40320),
        ),
        (
            "function big($a,$b,$c,$d,$e,$f,$g,$h,$i){return $a+$i;}$a=big(1,2,3,4,5,6,7,8,9);$seen=warm(11);",
            Value::Int(11),
        ),
        (
            "function f($a,$b=($scratch=7),$c=$a+$scratch){return [$a,$b,$c];}$seen=f(1);",
            Value::List(vec![Value::Int(1), Value::Int(7), Value::Int(8)]),
        ),
    ] {
        let (runtime, result) = compare(source, RuntimeOptions::default());
        assert_eq!(result.unwrap(), 0, "{source}");
        assert_eq!(runtime.get_global("seen"), Some(&expected));
    }
    for source in [
        "function f($x:int){return $x;}$seen=f('bad');",
        "function f($x):int{return 'bad';}$seen=f(1);",
        "function f($x){return $x;}f();",
        "function f($x=1/0){return $x;}f();",
        "$global=7;function f($n){if($n>0){return f($n-1);}return $global;}f(4);",
    ] {
        assert!(
            compare(source, RuntimeOptions::default()).1.is_err(),
            "{source}"
        );
    }
}

#[test]
fn warm_records_preserve_scope_step_value_and_recursion_limits() {
    let source = "function f($n:int):int{if($n<2){return $n;}return f($n-1)+f($n-2);}$seen=f(6);";
    for scopes in 0..20 {
        let _ = compare(
            source,
            RuntimeOptions {
                max_scopes: Some(scopes),
                ..RuntimeOptions::default()
            },
        );
    }
    for steps in 0..180 {
        let _ = compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            },
        );
    }
    for depth in 0..9 {
        let _ = compare(
            source,
            RuntimeOptions {
                recursion_limit: depth,
                ..RuntimeOptions::default()
            },
        );
    }
    let slot = std::mem::size_of::<Value>();
    for bytes in [0, slot - 1, slot, slot + 1, slot * 2, 1024] {
        let _ = compare(
            source,
            RuntimeOptions {
                max_value_bytes: Some(bytes),
                ..RuntimeOptions::default()
            },
        );
    }
}

#[test]
fn parameter_records_are_bounded_clear_values_and_survive_collection() {
    let mut env = Environments::default();
    let mut previous = Vec::new();
    let mut values = Vec::new();
    for _ in 0..=MAX_SPARE_SCOPES {
        previous.push(env.enter(env.current));
        let payload: Rc<[u8]> = Rc::from(&b"payload"[..]);
        values.push(Rc::downgrade(&payload));
        let scope = env.scope_mut(env.current);
        scope.function_boundary = true;
        scope
            .variables
            .insert("argument".into(), Value::Bytes(payload));
    }
    for parent in previous.into_iter().rev() {
        env.collect();
        env.leave_parameter(parent);
    }
    assert_eq!(env.live, 1);
    assert_eq!(reserved(&env).count(), MAX_SPARE_SCOPES);
    assert!(values.iter().all(|value| value.upgrade().is_none()));
    for _ in 0..10 {
        assert_eq!(env.collect(), 0);
        for id in reserved(&env) {
            let scope = env.scope(id);
            assert!(scope.parent.is_none());
            assert!(!scope.captured && !scope.function_boundary);
            assert!(scope.variables.is_empty());
            assert!(!env.free.contains(&id));
        }
        let previous = env.enter(0);
        assert_eq!(env.scope(env.current).parent, Some(0));
        assert_eq!(env.live, 2);
        env.leave_parameter(previous);
    }
    assert_eq!(env.live, 1);
}

#[test]
fn reserved_parameter_records_do_not_keep_captured_parents_alive() {
    let mut env = Environments::default();
    let outer = env.enter(0);
    let parent = env.current;
    env.scope_mut(parent).captured = true;
    let previous = env.enter(parent);
    env.leave_parameter(previous);
    env.leave(outer);
    assert_eq!(env.collect(), 1);
    assert!(env.scopes[parent].is_none());
    assert_eq!(env.live, 1);
    assert_eq!(reserved(&env).count(), 1);
}

#[test]
fn exact_capacity_and_namespace_storage_bound_reserved_records() {
    for capacity in [MAX_SPARE_SCOPE_SLOTS, MAX_SPARE_SCOPE_SLOTS + 1] {
        let mut env = Environments::default();
        let previous = env.enter(0);
        let id = env.current;
        env.scope_mut(id).variables =
            crate::bindings::Bindings::Slots(Vec::with_capacity(capacity));
        env.scope_mut(id)
            .variables
            .insert("argument".into(), Value::Int(1));
        env.leave_parameter(previous);
        assert_eq!(
            reserved(&env).count(),
            usize::from(capacity == MAX_SPARE_SCOPE_SLOTS)
        );
        assert_eq!(env.scopes[id].is_some(), capacity == MAX_SPARE_SCOPE_SLOTS);
        assert_eq!(env.live, 1);
    }
    for namespace in 0..4 {
        let mut env = Environments::default();
        let previous = env.enter(0);
        let id = env.current;
        let scope = env.scope_mut(id);
        match namespace {
            0 => scope.functions = HashMap::with_capacity(1),
            1 => scope.classes = HashMap::with_capacity(1),
            2 => scope.aliases = HashMap::with_capacity(1),
            _ => scope.loaded_modules = HashSet::with_capacity(1),
        }
        env.leave_parameter(previous);
        assert!(reserved(&env).next().is_none());
        assert!(env.scopes[id].is_none());
        assert_eq!(env.live, 1);
    }
}

#[test]
fn reentrant_cancellation_releases_arguments_and_recovers() {
    use std::{
        cell::RefCell,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime.environments.disable_parameter_reuse = disabled;
        runtime
            .execute_source(
                "function warm($x){if(false){return 0;}return $x;}warm(1);",
                &mut Vec::new(),
            )
            .unwrap();
        let payload: Rc<[u8]> = Rc::from(&b"payload"[..]);
        let weak = Rc::downgrade(&payload);
        let holder = RefCell::new(Some(payload));
        runtime
            .register(
                "make",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| Ok(Value::Bytes(holder.borrow_mut().take().unwrap())),
                ),
            )
            .unwrap();
        runtime
            .register(
                "reenter",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::Int, NativeType::Any],
                        NativeType::Int,
                    ),
                    |context, args| context.invoke(args[0].clone(), &args[1..]),
                ),
            )
            .unwrap();
        runtime
            .register(
                "trip",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Null),
                    |context, _| {
                        context
                            .cancellation
                            .as_ref()
                            .unwrap()
                            .store(true, Ordering::Relaxed);
                        Ok(Value::Null)
                    },
                ),
            )
            .unwrap();
        errors.push(runtime.execute_source("function f($n:int,$data):int{if($n===0){trip();return 0;}return reenter(f,$n-1,$data);}$seen=f(5,make());",&mut Vec::new()).unwrap_err());
        assert!(weak.upgrade().is_none());
        assert!(runtime.argument_buffers.iter().all(Vec::is_empty));
        assert_eq!(runtime.environments.live, 1);
        assert!(runtime.stack.is_empty());
        flag.store(false, Ordering::Relaxed);
        runtime
            .execute_source("$seen=warm(7);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(7)));
    }
    assert_eq!(errors[0], errors[1]);
}
