use super::*;

fn compare(
    source: &str,
    options: RuntimeOptions,
) -> (Runtime, std::result::Result<u8, RuntimeError>) {
    let mut fast = Runtime::with_options(options.clone());
    let mut old = Runtime::with_options(options);
    fast.disable_numeric_calls = true;
    old.disable_numeric_calls = true;
    old.disable_primitive_annotations = true;
    old.disable_shared_parameter_names = true;
    if source.contains("$binary") {
        for runtime in [&mut fast, &mut old] {
            runtime
                .set_global("binary", Value::Bytes(Rc::from(&b"\0\xff"[..])))
                .unwrap();
        }
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
        assert!(fast.stack.is_empty());
        assert!(old.stack.is_empty());
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
fn general_primitive_checks_accept_values_and_preserve_mismatch_diagnostics() {
    for (kind, value) in [
        ("int", "7"),
        ("float", "1.25"),
        ("string", "'é'"),
        ("bool", "true"),
        ("bytes", "$binary"),
        ("list", "[1,2]"),
        ("map", "['a'=>1]"),
        ("callable", "function(){return 7;}"),
    ] {
        let source = format!(
            "function f($x:{kind}):{kind}{{if(false){{return $x;}}return $x;}}$seen=f({value});"
        );
        let (runtime, result) = compare(&source, RuntimeOptions::default());
        assert_eq!(result.unwrap(), 0);
        assert!(runtime.get_global("seen").is_some());
    }
    for source in [
        "function f($x:int):int{if(false){return 0;}return $x;}$seen=f(1.0);",
        "function f($x:float):float{if(false){return 0.0;}return $x;}$seen=f(1);",
        "function f($x:string):int{if(false){return 0;}return $x;}$seen=f('bad');",
        "function f($x:bool):bool{if(false){return true;}return $x;}$seen=f(1);",
        "function f($x:void){return $x;}$seen=f(null);",
        "function f($x:T::int){return $x;}$seen=f(1);",
        "class Thing{}function f($x:Thing){return $x;}$seen=f(1);",
    ] {
        let (runtime, result) = compare(source, RuntimeOptions::default());
        assert!(
            result
                .unwrap_err()
                .message
                .contains("type annotation requires")
        );
        assert!(runtime.get_global("seen").is_none());
    }
    let (_, result) = compare(
        "function f():void{if(false){return;}return null;}f();",
        RuntimeOptions::default(),
    );
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn parameter_storage_handles_defaults_promotions_receivers_and_escaped_captures() {
    for (source, expected) in [
        (
            "function f($a,$b=($scratch=7),$c=$a+$scratch){return [$a,$b,$c];}$seen=f(1);",
            Value::List(vec![Value::Int(1), Value::Int(7), Value::Int(8)]),
        ),
        (
            "function f($n){if($n<2){return $n;}return f($n-1)+f($n-2);}$seen=f(12);",
            Value::Int(144),
        ),
        (
            "function factory($n){return function(){return $n+=1;};}$f=factory(10);$a=$f();$seen=$f();",
            Value::Int(12),
        ),
        (
            "function f($a,...$rest){return [$a,$rest];}$seen=f(1,2,3);",
            Value::List(vec![
                Value::Int(1),
                Value::List(vec![Value::Int(2), Value::Int(3)]),
            ]),
        ),
        (
            "class C{function f($n){if($n===0){return 1;}return $n*$this->f($n-1);}}$c=new C();$seen=$c->f(8);",
            Value::Int(40320),
        ),
        (
            "class C{function f($a,$b=$this){return $b;}}$c=new C();$f=$c->f;$seen=$f(1)===$c;",
            Value::Bool(true),
        ),
        (
            "function f($a,$b,$c,$d,$e,$f,$g,$h){$a+=1;return $a+$h;}$seen=f(1,2,3,4,5,6,7,8);",
            Value::Int(10),
        ),
        (
            "class C{function f($a,$b,$c,$d,$e,$f,$g,$h){return $a+$h;}}$c=new C();$seen=$c->f(1,2,3,4,5,6,7,8);",
            Value::Int(9),
        ),
        (
            "function f($a,$b,$c,$d,$e,$f,$g,$h,$i){return $a+$i;}$seen=f(1,2,3,4,5,6,7,8,9);",
            Value::Int(10),
        ),
    ] {
        let (runtime, result) = compare(source, RuntimeOptions::default());
        assert_eq!(result.unwrap(), 0, "{source}");
        assert_eq!(runtime.get_global("seen"), Some(&expected));
    }
}

#[test]
fn recursive_setup_preserves_exact_limits_errors_and_recovery() {
    let source = "function f($n:int):int{if($n<2){return $n;}return f($n-1)+f($n-2);}$seen=f(6);";
    for steps in 0..180 {
        let _ = compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            },
        );
    }
    for scopes in 0..20 {
        let _ = compare(
            source,
            RuntimeOptions {
                max_scopes: Some(scopes),
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
fn native_objects_named_like_primitives_keep_downcasts_and_generic_compatibility() {
    use std::cell::Cell;
    #[derive(Debug)]
    struct Primitive {
        kind: &'static str,
        calls: Rc<Cell<usize>>,
    }
    impl NativeObject for Primitive {
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            self.calls.set(self.calls.get() + 1);
            None
        }
        fn type_name(&self) -> &str {
            self.kind
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Ok(Value::Int(7))
        }
    }
    for kind in [
        "int", "float", "string", "bytes", "bool", "list", "map", "callable", "void",
    ] {
        let mut observations = Vec::new();
        for disabled in [false, true] {
            let calls = Rc::new(Cell::new(0));
            let c = calls.clone();
            let mut runtime = Runtime::new();
            runtime.disable_primitive_annotations = disabled;
            runtime
                .register(
                    "make",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Any),
                        move |_, _| {
                            Ok(Value::NativeObject(Rc::new(Primitive {
                                kind,
                                calls: c.clone(),
                            })))
                        },
                    ),
                )
                .unwrap();
            runtime.execute_source(&format!("function f($x:{kind}):{kind}{{if(false){{return $x;}}return $x;}}$obj=f(make());$seen=$obj->value;"),&mut Vec::new()).unwrap();
            assert_eq!(runtime.get_global("seen"), Some(&Value::Int(7)));
            observations.push(calls.get());
        }
        assert_eq!(observations[0], observations[1], "{kind}");
        assert!(observations[0] >= 2);
    }
}

#[test]
fn cancellation_releases_shared_parameter_values_and_allows_recovery() {
    use std::cell::RefCell;
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime.disable_shared_parameter_names = disabled;
        runtime.disable_primitive_annotations = disabled;
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
        errors.push(runtime.execute_source(
            "function f($n:int,$payload):int{if($n===0){trip();return 0;}return f($n-1,$payload);}$seen=f(6,make());",
            &mut Vec::new(),
        ).unwrap_err());
        assert!(weak.upgrade().is_none());
        assert!(runtime.argument_buffers.iter().all(Vec::is_empty));
        assert_eq!(runtime.environments.live, 1);
        assert!(runtime.stack.is_empty());
        flag.store(false, Ordering::Relaxed);
        runtime.execute_source("$seen=7;", &mut Vec::new()).unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(7)));
    }
    assert_eq!(errors[0], errors[1]);
}
