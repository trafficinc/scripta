use super::*;

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let mut fast = Runtime::with_options(options.clone());
    let mut slow = Runtime::with_options(options);
    slow.environments.disable_empty_reuse = true;
    for text in [source, "$recovered=7;"] {
        let mut a = Vec::new();
        let mut b = Vec::new();
        assert_eq!(
            fast.execute_source(text, &mut a),
            slow.execute_source(text, &mut b),
            "{source}"
        );
        assert_eq!(a, b);
        assert_eq!(fast.steps, slow.steps);
        assert_eq!(fast.environments.live, slow.environments.live);
        assert_eq!(fast.environments.current, 0);
        assert_eq!(slow.environments.current, 0);
        assert!(fast.stack.is_empty());
        assert!(slow.stack.is_empty());
        for (name, value) in &fast.environments.scope(0).variables {
            if !matches!(value, Value::Callable(_) | Value::NativeObject(_)) {
                assert_eq!(Some(value), slow.get_global(name), "{name}");
            }
        }
    }
    fast
}

#[test]
fn recursive_functions_methods_and_block_control_flow_preserve_results() {
    for (source, expected) in [
        (
            "function fib($n:int):int{if($n<2){return $n;}return fib($n-1)+fib($n-2);}$seen=fib(12);",
            Value::Int(144),
        ),
        (
            "function a($n){if($n<2){return $n;}return b($n-1)+b($n-2);}function b($n){if($n<2){return $n;}return a($n-1)+a($n-2);}$seen=a(12);",
            Value::Int(144),
        ),
        (
            "class C{function f($n:int):int{if($n===0){return 1;}return $n*$this->f($n-1);}}$c=new C();$seen=$c->f(10);",
            Value::Int(3628800),
        ),
        (
            "$seen=0;for($i=0;$i<10;$i+=1){{if($i===3){continue;}if($i===8){break;}$seen+=$i;}}",
            Value::Int(25),
        ),
        (
            "function f($n){if($n===0){throw new Error('bad');}return f($n-1);}$seen='';try{f(4);}catch(Error $e){$seen=$e->message;}",
            Value::String("bad".into()),
        ),
    ] {
        let runtime = compare(source, RuntimeOptions::default());
        assert_eq!(runtime.get_global("seen"), Some(&expected));
    }
    for source in [
        "function f($n){if($n===0){return 1/0;}return f($n-1);}f(4);",
        "function f($n):int{if($n===0){return 'bad';}return f($n-1);}f(4);",
        "function f($n){if($n===0){exit(3);}return f($n-1);}f(4);",
    ] {
        compare(source, RuntimeOptions::default());
    }
}

#[test]
fn captured_empty_blocks_and_local_bindings_remain_independent() {
    let runtime = compare(
        "function make($n){return function(){return $n+=1;};}$f=make(10);{}{}$a=$f();$b=$f();\
         function local($n){$x=$n;{function read(){return $x;}return read;}}\
         $g=local(20);{}{}$c=$g();$d=local(30)();\
         function fresh($n){if($n>0){$v=$n;return fresh($n-1)+$v;}return 0;}$sum=fresh(8);",
        RuntimeOptions::default(),
    );
    for (name, expected) in [("a", 11), ("b", 12), ("c", 20), ("d", 30), ("sum", 36)] {
        assert_eq!(runtime.get_global(name), Some(&Value::Int(expected)));
    }
}

#[test]
fn exact_scope_step_depth_and_value_limits_keep_errors_and_recovery() {
    let source =
        "function fib($n:int):int{if($n<2){return $n;}return fib($n-1)+fib($n-2);}$seen=fib(6);";
    for scopes in 0..20 {
        compare(
            source,
            RuntimeOptions {
                max_scopes: Some(scopes),
                ..RuntimeOptions::default()
            },
        );
    }
    for steps in 0..180 {
        compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            },
        );
    }
    for depth in 0..9 {
        compare(
            source,
            RuntimeOptions {
                recursion_limit: depth,
                ..RuntimeOptions::default()
            },
        );
    }
    let slot = std::mem::size_of::<Value>();
    for bytes in [0, slot - 1, slot, slot + 1, slot * 2, 1024] {
        compare(
            source,
            RuntimeOptions {
                max_value_bytes: Some(bytes),
                ..RuntimeOptions::default()
            },
        );
    }
}

#[test]
fn cancellation_during_recursive_reentry_unwinds_reused_blocks() {
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime.environments.disable_empty_reuse = disabled;
        runtime
            .register(
                "reenter",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::Int],
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
        // The first call warms records; the second interrupts in a nested block.
        runtime
            .execute_source(
                "function f($n){if($n>0){return reenter(f,$n-1)+1;}return 0;}$seen=f(4);",
                &mut Vec::new(),
            )
            .unwrap();
        errors.push(
            runtime
                .execute_source(
                    "function g($n){if($n>0){return reenter(g,$n-1);}trip();return 0;}g(4);",
                    &mut Vec::new(),
                )
                .unwrap_err(),
        );
        assert_eq!(runtime.environments.live, 1);
        assert_eq!(runtime.environments.current, 0);
        assert!(runtime.stack.is_empty());
        flag.store(false, Ordering::Relaxed);
        runtime
            .execute_source("$seen=f(5);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(5)));
    }
    assert_eq!(errors[0], errors[1]);
}
