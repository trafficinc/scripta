use super::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let mut fast = Runtime::with_options(options.clone());
    let mut slow = Runtime::with_options(options);
    slow.disable_call_reuse = true;
    let mut a = Vec::new();
    let mut b = Vec::new();
    assert_eq!(
        fast.execute_source(source, &mut a),
        slow.execute_source(source, &mut b),
        "{source}"
    );
    assert_eq!(a, b, "{source}");
    assert_eq!(fast.steps, slow.steps, "{source}");
    assert_eq!(fast.environments.live, slow.environments.live);
    assert!(fast.stack.is_empty());
    assert!(slow.stack.is_empty());
    for (name, value) in &fast.environments.scope(0).variables {
        if !matches!(value, Value::Callable(_) | Value::NativeObject(_)) {
            assert_eq!(Some(value), slow.get_global(name), "{source}: {name}");
        }
    }
    assert!(fast.argument_buffers.len() <= MAX_ARGUMENT_BUFFERS);
    assert!(fast.argument_buffers.iter().all(
        |buffer| buffer.is_empty() && (1..=MAX_ARGUMENT_CAPACITY).contains(&buffer.capacity())
    ));
    fast
}

#[test]
fn defaults_variadics_recursion_methods_and_failure_traces_match_owned_setup() {
    for source in [
        "function f($x){return $x+1;}$observed=f(f(1));",
        "function fib($n){if($n<2){return $n;}return fib($n-1)+fib($n-2);}$observed=fib(12);",
        "function make(){$n=0;return function($x=($n+=1)){return $x;};}$f=make();$a=$f();$b=$f();",
        "function f($x,$y=$x+1,...$rest){return [$x,$y,$rest];}$a=f(1);$b=f(1,3);$c=f(1,3,4,5);",
        "function f(...$rest){return $rest;}$a=f();$b=f(1,2,3);$copy=$b;$b[0]=9;",
        "function f($x:int,...$rest:int){return $rest;}$observed=f(1,2,'bad');",
        "function f($x){return $x;}$observed=f();",
        "function f($x){return $x;}$observed=f(1,2);",
        "function f($x:int){return $x;}function g(){return f('bad');}g();",
        "function f($x):int{return 'bad';}f(1);",
        "function f($x=10/0){return $x;}f();",
        "function f($x){return 1/0;}try{f(1);}catch(ArithmeticError $e){$caught=true;}",
        "class C{$n=0;function next(){return $this->n+=1;}}$c=new C();$m=$c->next;$c=null;$a=$m();$b=$m();",
        "function make($x){return function(){return $x+=1;};}$f=make(10);$a=$f();$b=$f();",
        "function f($x){return $x;}function side(){echo 'side';return 1;}$observed=f(...[side(),side()]);",
        "function f($x){exit(3);}f(1);",
    ] {
        compare(source, RuntimeOptions::default());
    }
}

#[test]
fn exact_budgets_scope_byte_and_recursion_limits_keep_recovery() {
    let source = "function f($x,$y=2,...$rest){return [$x,$y,$rest];}$observed=f(1,...[3,4]);";
    for limit in 0..30 {
        compare(
            source,
            RuntimeOptions {
                instruction_limit: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    for limit in 1..=6 {
        compare(
            source,
            RuntimeOptions {
                max_scopes: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    let slot = std::mem::size_of::<Value>();
    for limit in [
        0,
        slot - 1,
        slot,
        slot * 2 - 1,
        slot * 2,
        slot * 3 - 1,
        slot * 3,
        slot * 3 + 1,
        4096,
    ] {
        compare(
            source,
            RuntimeOptions {
                max_value_bytes: Some(limit),
                ..RuntimeOptions::default()
            },
        );
    }
    for limit in 0..=4 {
        compare(
            "function f($x){if($x>0){return f($x-1);}return 1;}$observed=f(3);",
            RuntimeOptions {
                recursion_limit: limit,
                ..RuntimeOptions::default()
            },
        );
    }
    let mut runtime = compare(
        "function f($x){return $x/0;}f(1);",
        RuntimeOptions::default(),
    );
    runtime
        .execute_source("$observed=7;", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("observed"), Some(&Value::Int(7)));
}

#[test]
fn recursive_reentrant_calls_cannot_overwrite_suspended_arguments() {
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.disable_call_reuse = disabled;
        runtime
            .register(
                "reenter",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::Int],
                        NativeType::Int,
                    ),
                    |context, args| {
                        let before = args[1].clone();
                        let result = context.invoke(args[0].clone(), &args[1..]);
                        assert_eq!(args[1], before);
                        result
                    },
                ),
            )
            .unwrap();
        runtime.execute_source("function step($x){if($x>0){return reenter(step,$x-1)+1;}return 0;}$observed=reenter(step,8);",&mut Vec::new()).unwrap();
        assert_eq!(runtime.get_global("observed"), Some(&Value::Int(8)));
        assert!(runtime.argument_buffers.iter().all(Vec::is_empty));
    }
}

#[derive(Debug)]
struct Probe(Rc<Cell<usize>>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
impl NativeObject for Probe {
    fn type_name(&self) -> &str {
        "Probe"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("no properties".into())
    }
}

#[test]
fn pooled_arguments_never_retain_values_after_success_failure_or_cancellation() {
    for source in [
        "sink(make());",
        "function f($x,$y){}f(make(),1/0);",
        "function bad($x:int){}bad(make());",
        "function bad($x){return 1/0;}bad(make());",
        "trip(make());",
        "function bad(...$xs:int){}bad(make());",
    ] {
        let count = Rc::new(Cell::new(0));
        let probe = Rc::new(Probe(count.clone()));
        let weak = Rc::downgrade(&probe);
        let holder = Rc::new(RefCell::new(Some(probe)));
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag),
            ..RuntimeOptions::default()
        });
        runtime
            .register(
                "make",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Object("Probe")),
                    move |_, _| Ok(Value::NativeObject(holder.borrow_mut().take().unwrap())),
                ),
            )
            .unwrap();
        runtime
            .register(
                "sink",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::Any], NativeType::Null),
                    |_, _| Ok(Value::Null),
                ),
            )
            .unwrap();
        runtime
            .register(
                "trip",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::Any], NativeType::Null),
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
        let result = runtime.execute_source(source, &mut Vec::new());
        assert_eq!(result.is_ok(), source == "sink(make());");
        assert_eq!(count.get(), 1, "{source}");
        assert!(weak.upgrade().is_none());
        assert!(runtime.argument_buffers.iter().all(Vec::is_empty));
        assert!(runtime.stack.is_empty());
        assert_eq!(runtime.environments.live, 1);
    }
}

#[test]
fn retained_variadics_and_captured_parameters_keep_values_until_collection() {
    for source in [
        "function keep($x){return function(){return $x;};}$f=keep(make());",
        "function keep(...$xs){return function(){return $xs[0];};}$f=keep(make());",
    ] {
        let count = Rc::new(Cell::new(0));
        let probe = Rc::new(Probe(count.clone()));
        let weak = Rc::downgrade(&probe);
        let holder = Rc::new(RefCell::new(Some(probe)));
        let mut runtime = Runtime::new();
        runtime
            .register(
                "make",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Object("Probe")),
                    move |_, _| Ok(Value::NativeObject(holder.borrow_mut().take().unwrap())),
                ),
            )
            .unwrap();
        runtime.execute_source(source, &mut Vec::new()).unwrap();
        assert!(weak.upgrade().is_some());
        let function = runtime.get_global("f").unwrap().clone();
        runtime.environments.scope_mut(0).variables.remove("f");
        runtime.collect_garbage();
        assert!(weak.upgrade().is_some());
        assert!(matches!(
            runtime
                .invoke(function.clone(), &[], &mut Vec::new())
                .unwrap(),
            Value::NativeObject(_)
        ));
        drop(function);
        runtime.collect_garbage();
        assert_eq!(count.get(), 1);
        assert!(weak.upgrade().is_none());
    }
}

#[test]
fn shared_frames_release_with_callable_but_diagnostic_snapshots_remain_owned() {
    let mut runtime = Runtime::new();
    runtime
        .execute_source("function fail($parameter){return 1/0;}", &mut Vec::new())
        .unwrap();
    let function = runtime.get_callable("fail").unwrap();
    let Value::Callable(callable) = &function else {
        panic!()
    };
    let Function::User { declaration, .. } = callable.0.as_ref() else {
        panic!()
    };
    let declaration_weak = Rc::downgrade(declaration);
    let error = runtime
        .invoke(function.clone(), &[Value::Int(1)], &mut Vec::new())
        .unwrap_err();
    runtime.environments.scope_mut(0).functions.remove("fail");
    drop(function);
    runtime.collect_garbage();
    assert!(declaration_weak.upgrade().is_none());
    assert_eq!(
        error.diagnostic().unwrap().trace.last().unwrap().function,
        "fail"
    );
}

#[test]
fn buffer_pool_is_bounded_and_discards_oversized_allocations() {
    let mut runtime = Runtime::new();
    runtime
        .execute_source(
            "function recurse($n){if($n>0){return recurse($n-1);}return 0;}$x=recurse(24);",
            &mut Vec::new(),
        )
        .unwrap();
    assert_eq!(runtime.argument_buffers.len(), 25);
    runtime.options.recursion_limit = 25;
    let error = runtime
        .execute_source("recurse(25);", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("maximum call depth"));
    assert!(runtime.argument_buffers.iter().all(Vec::is_empty));
    // Exercise the pool ceiling directly without depending on the host test
    // thread's stack size for a 64-level general AST recursion.
    runtime
        .argument_buffers
        .resize_with(MAX_ARGUMENT_BUFFERS, || Vec::with_capacity(1));
    let Value::Callable(function) = runtime.get_callable("recurse").unwrap() else {
        panic!()
    };
    runtime
        .call_callable(
            function,
            vec![Value::Int(0)],
            Span::default(),
            &mut Vec::new(),
        )
        .ok()
        .unwrap();
    assert_eq!(runtime.argument_buffers.len(), MAX_ARGUMENT_BUFFERS);
    runtime
        .register(
            "sink",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null).with_variadic(NativeType::Any),
                |_, _| Ok(Value::Null),
            ),
        )
        .unwrap();
    let source = format!("sink({});", vec!["1"; 64].join(","));
    runtime.execute_source(&source, &mut Vec::new()).unwrap();
    assert!(
        runtime
            .argument_buffers
            .iter()
            .all(|buffer| buffer.is_empty() && buffer.capacity() <= MAX_ARGUMENT_CAPACITY)
    );
}

#[test]
fn short_variadic_tails_do_not_retain_large_or_pooled_argument_buffers() {
    let mut runtime = Runtime::new();
    let parameters = (0..63)
        .map(|i| format!("$a{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let arguments = vec!["1"; 64].join(",");
    let source =
        format!("function tail({parameters},...$rest){{return $rest;}}$result=tail({arguments});");
    runtime.execute_source(&source, &mut Vec::new()).unwrap();
    let Some(Value::List(rest)) = runtime.get_global("result") else {
        panic!()
    };
    assert_eq!(rest, &vec![Value::Int(1)]);
    assert!(rest.capacity() <= 4);
    runtime
        .register(
            "prime",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null).with_variadic(NativeType::Any),
                |_, _| Ok(Value::Null),
            ),
        )
        .unwrap();
    let source = format!(
        "prime({});function small(...$xs){{return $xs;}}$result=small(7);",
        vec!["1"; 16].join(",")
    );
    runtime.execute_source(&source, &mut Vec::new()).unwrap();
    let Some(Value::List(rest)) = runtime.get_global("result") else {
        panic!()
    };
    assert_eq!(rest, &vec![Value::Int(7)]);
    assert!(rest.capacity() <= 4);
}
