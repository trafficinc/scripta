use crate::{Runtime, RuntimeOptions, Value, test_support};

fn pair() -> (Runtime, Runtime) {
    let (fast, mut old) = test_support::runtimes(RuntimeOptions::default());
    old.disable_method_local_plans = true;
    (fast, old)
}

fn field(runtime: &Runtime) -> (Option<Value>, usize) {
    let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
        panic!("object")
    };
    let instance = object
        .as_any()
        .unwrap()
        .downcast_ref::<crate::objects::ScriptObject>()
        .unwrap();
    (
        runtime
            .environments
            .scope(instance.scope)
            .variables
            .get("x")
            .cloned(),
        object.payload_bytes(),
    )
}

#[test]
fn scalar_locals_reassignments_inputs_fields_and_bound_calls_match() {
    for source in [
        "class C{$x=0;function f():int{$next=$this->x+1;$this->x=$next;return $next;}}$next=99;$c=new C();$bound=$c->f;$a=$bound();$b=$c->f();$seen=[$next,$a,$b,$c->x];",
        "class C{$x=0;$y=2;function f($a:int,$b:int):int{$v=$a+$this->y;$w=$v+$b;$v+=2;$w*=3;$v-=1;$w/=2;$this->x=$v+$w;return $this->x;}}$c=new C();$seen=$c->f(3,4);",
        "class C{$x=0;function f($v:bool):bool{$a=null;$a=$v;$b=$a && true;return $b;}}$c=new C();$seen=[$c->f(true),$c->f(false)];",
        "class C{$x=0;function f():void{$a=1;$this->x=$a;}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f():int{$a=1;return $a;$a=10;$this->x=$a;}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f():int{$next=$this->x+1;$this->x=$next;return $next;}}function run($c):int{return $c->f();}$c=new C();$seen=run($c);",
    ] {
        for hashed in [false, true] {
            let (mut a, mut b) = pair();
            if hashed {
                a.environments.force_hash_bindings();
                b.environments.force_hash_bindings();
            }
            test_support::compare_source(&mut a, &mut b, source).unwrap();
            assert!(a.method_local_plans_executed > 0, "{source}");
            assert_eq!(b.method_local_plans_executed, 0);
            assert_eq!(field(&a), field(&b));
        }
    }
}

#[test]
fn local_failures_preserve_diagnostics_partial_field_writes_and_recovery() {
    for source in [
        "class C{$x=0;function f():int{$a=1;$this->x=$a;$a/=0;return $a;}}$c=new C();$seen='kept';$seen=$c->f();",
        "class C{$x=0;function f():int{$a=9223372036854775807;$this->x=9;$a+=1;return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$a=1;$this->x=9;return true;}}$c=new C();$c->f();",
        "class C{$x:int=0;function f():int{$a=true;$this->x=$a;return 1;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$a=true;$this->x=9;$a+=1;return 1;}}$c=new C();$c->f();",
        "class C{$x=0;function f($v:int):int{$a=$v;$this->x=9;return $a;}}$c=new C();$c->f(false);",
    ] {
        let (mut a, mut b) = pair();
        assert!(
            test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;").is_err()
        );
        assert_eq!(field(&a), field(&b));
        assert!(a.method_local_plans_executed > 0);
    }
}

#[test]
fn exact_local_method_instruction_scope_call_and_byte_limits_match() {
    let mut options = Vec::new();
    for scopes in 0..8 {
        for steps in 0..35 {
            options.push(RuntimeOptions {
                max_scopes: Some(scopes),
                instruction_limit: Some(steps),
                ..RuntimeOptions::default()
            });
        }
    }
    for recursion_limit in 0..4 {
        options.push(RuntimeOptions {
            recursion_limit,
            ..RuntimeOptions::default()
        });
    }
    for bytes in [0, 1, 31, 32, 33, 64, 65, 66, 1024] {
        options.push(RuntimeOptions {
            max_value_bytes: Some(bytes),
            ..RuntimeOptions::default()
        });
    }
    for option in options {
        let (mut a, mut b) = pair();
        test_support::compare_source(&mut a,&mut b,"class C{$x:int=0;function f($v:int):int{$a=$this->x+$v;$a+=1;$this->x=$a;return $a;}}$c=new C();$warm=$c->f(0);").unwrap();
        a.method_local_plans_executed = 0;
        b.method_local_plans_executed = 0;
        a.options = option.clone();
        b.options = option;
        let _ = test_support::compare_source(&mut a, &mut b, "$seen=$c->f(2);$again=$c->f(3);");
        assert_eq!(field(&a), field(&b));
        if a.options.max_value_bytes.is_some() {
            assert_eq!(a.method_local_plans_executed, 0);
        }
        a.options = RuntimeOptions::default();
        b.options = RuntimeOptions::default();
        test_support::compare_source(&mut a, &mut b, "$recovered=1;").unwrap();
    }
}

#[test]
fn captured_names_are_checked_per_receiver_even_with_shared_warm_plans() {
    for removed in [false, true] {
        let (mut a, mut b) = pair();
        test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f():int{$temp=1;$this->x=$temp;return $temp;}}$c=new C();$warm=$c->f();").unwrap();
        let mut modules = Vec::new();
        for runtime in [&mut a, &mut b] {
            let method = runtime.environments.scope(0).classes["C"]
                .declaration
                .methods[0]
                .clone();
            let previous = runtime.environments.enter(0);
            let module = runtime.environments.current;
            runtime
                .execute_source(
                    "$temp=99;class D{$x=0;function f():int{return 0;}}",
                    &mut Vec::new(),
                )
                .unwrap();
            let class = runtime
                .environments
                .scope_mut(module)
                .classes
                .get_mut("D")
                .unwrap();
            std::rc::Rc::get_mut(class).unwrap().declaration.methods[0] = method;
            runtime
                .execute_source("$object=new D();", &mut Vec::new())
                .unwrap();
            let object = runtime
                .environments
                .scope_mut(module)
                .variables
                .remove("object")
                .unwrap();
            if removed {
                runtime
                    .environments
                    .scope_mut(module)
                    .variables
                    .remove("temp");
            }
            runtime.environments.leave(previous);
            runtime.set_global("c", object).unwrap();
            modules.push(module);
            runtime.method_local_plans_executed = 0;
        }
        test_support::compare_source(&mut a, &mut b, "$seen=$c->f();$field=$c->x;").unwrap();
        assert_eq!(a.method_local_plans_executed, usize::from(removed));
        assert_eq!(b.method_local_plans_executed, 0);
        for (runtime, module) in [(&a, modules[0]), (&b, modules[1])] {
            assert_eq!(
                runtime.environments.scope(module).variables.get("temp"),
                if removed { None } else { Some(&Value::Int(1)) }
            );
        }
    }
}

#[test]
fn unsupported_local_reads_writes_and_effects_keep_general_evaluation() {
    for source in [
        "class C{$x=0;function f():int{$a=$a+1;return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$a=$b;$b=1;return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$a+=1;return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f($v:int):int{$v+=1;return $v;}}$c=new C();$c->f(1);",
        "class C{$x=0;function f():int{$this=1;return 1;}}$c=new C();$c->f();",
        "function helper():int{return 1;}class C{$x=0;function f():int{$a=helper();return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():string{$a='text';return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$a=[];return 1;}}$c=new C();$c->f();",
    ] {
        let (mut a, mut b) = pair();
        let _ = test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;");
        assert_eq!(a.method_local_plans_executed, 0, "{source}");
        assert_eq!(b.method_local_plans_executed, 0);
    }
}

#[test]
fn combined_parameter_local_bound_and_caller_loop_scope_stay_bounded() {
    for parameters in [0, 2] {
        for bindings in [8, 9] {
            let params = (0..parameters)
                .map(|i| format!("$p{i}:int"))
                .collect::<Vec<_>>()
                .join(",");
            let assignments = (0..bindings - parameters)
                .map(|i| format!("$v{i}={i};"))
                .collect::<String>();
            let source = format!(
                "class C{{$x=0;function f({params}):int{{{assignments}$this->x=$v0;return $this->x;}}}}$c=new C();$seen=$c->f({});",
                vec!["1"; parameters].join(",")
            );
            let (mut a, mut b) = pair();
            test_support::compare_source(&mut a, &mut b, &source).unwrap();
            assert_eq!(a.method_local_plans_executed, usize::from(bindings == 8));
        }
    }
    let fields = (0..8).map(|i| format!("$f{i}={i};")).collect::<String>();
    let locals = (0..8).map(|i| format!("$v{i}={i};")).collect::<String>();
    let sum = (0..8)
        .map(|i| format!("$this->f{i}"))
        .collect::<Vec<_>>()
        .join("+");
    let source = format!(
        "class C{{{fields}function f():int{{{locals}$this->f7=$v7+{sum};return $this->f7;}}}}$c=new C();$seen=$c->f();"
    );
    let (mut full, mut reference) = pair();
    test_support::compare_source(&mut full, &mut reference, &source).unwrap();
    assert_eq!(full.method_local_plans_executed, 1);
    assert_eq!(full.get_global("seen"), Some(&Value::Int(35)));
    let (mut a, mut b) = pair();
    test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f():int{$a=$this->x+1;$this->x=$a;return $a;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->f();}$field=$c->x;").unwrap();
    assert_eq!(a.method_local_plans_executed, 128);
    assert_eq!(a.method_call_loops_executed, 0);
    assert_eq!(b.method_call_loops_executed, 0);
    assert_eq!(a.get_global("total"), Some(&Value::Int(8256)));
}

#[test]
fn local_method_cancellation_unwinds_scopes_and_keeps_committed_fields() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    for disabled in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let cancel = flag.clone();
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag),
            ..RuntimeOptions::default()
        });
        runtime.disable_method_local_plans = disabled;
        runtime.execute_source("class C{$x=0;function f():int{$a=$this->x+1;$this->x=$a;return $a;}}$c=new C();$i=0;$total=0;",&mut Vec::new()).unwrap();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cancel.store(true, Ordering::Relaxed);
        });
        let error = runtime
            .execute_source(
                "for($i=0;$i<100000000;$i+=1){$total+=$c->f();}",
                &mut Vec::new(),
            )
            .unwrap_err();
        worker.join().unwrap();
        assert!(error.message.contains("interrupted"));
        let Some(Value::Int(i)) = runtime.get_global("i") else {
            panic!("counter")
        };
        let Some(Value::Int(total)) = runtime.get_global("total") else {
            panic!("total")
        };
        assert_eq!(*total, i * (i + 1) / 2);
        let (Some(Value::Int(x)), _) = field(&runtime) else {
            panic!("field")
        };
        assert!(x == *i || x == i + 1);
        assert!(runtime.stack.is_empty());
        assert!(runtime.environments.temporary_roots.is_empty());
        assert_eq!(runtime.environments.live, 2);
        assert_eq!(runtime.method_local_plans_executed > 0, !disabled);
    }
}

#[test]
fn captured_native_resources_release_at_the_same_assignment_or_collection() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Resource(Rc<Cell<usize>>);
    impl NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("none".into())
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    for collision in [false, true] {
        let (mut a, mut b) = pair();
        let mut resources = Vec::new();
        let mut receivers = Vec::new();
        for runtime in [&mut a, &mut b] {
            let drops = Rc::new(Cell::new(0));
            let count = drops.clone();
            runtime
                .register(
                    "probe",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Any),
                        move |_, _| Ok(Value::NativeObject(Rc::new(Resource(count.clone())))),
                    ),
                )
                .unwrap();
            let previous = runtime.environments.enter(0);
            let module = runtime.environments.current;
            let name = if collision { "temp" } else { "resource" };
            runtime.execute_source(&format!("${name}=probe();class C{{$x=0;function f():int{{$temp=1;$this->x=$temp;return $temp;}}}}$object=new C();"),&mut Vec::new()).unwrap();
            let object = runtime
                .environments
                .scope_mut(module)
                .variables
                .remove("object")
                .unwrap();
            let Value::NativeObject(receiver) = &object else {
                panic!("object")
            };
            receivers.push(Rc::downgrade(receiver));
            runtime.environments.leave(previous);
            runtime.set_global("c", object).unwrap();
            runtime.collect_garbage();
            assert_eq!(drops.get(), 0);
            resources.push(drops);
        }
        test_support::compare_source(&mut a, &mut b, "$seen=$c->f();").unwrap();
        assert_eq!(a.method_local_plans_executed, usize::from(!collision));
        for drops in &resources {
            assert_eq!(drops.get(), usize::from(collision));
        }
        test_support::compare_source(&mut a, &mut b, "$c=null;").unwrap();
        for runtime in [&mut a, &mut b] {
            runtime.collect_garbage();
        }
        for drops in &resources {
            assert_eq!(drops.get(), 1);
        }
        for receiver in receivers {
            assert!(receiver.upgrade().is_none());
        }
    }
}

#[test]
fn warmed_local_plans_preserve_native_proxy_observations() {
    use crate::NativeObject;
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Proxy {
        object: Rc<dyn NativeObject>,
        casts: Rc<Cell<usize>>,
    }
    impl NativeObject for Proxy {
        fn type_name(&self) -> &str {
            "Proxy"
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            self.casts.set(self.casts.get() + 1);
            self.object.as_any()
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            panic!("script downcast")
        }
    }
    let (mut a, mut b) = pair();
    test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f():int{$a=$this->x+1;$this->x=$a;return $a;}}$c=new C();$warm=$c->f();").unwrap();
    let mut casts = Vec::new();
    for runtime in [&mut a, &mut b] {
        let Value::NativeObject(object) = runtime.get_global("c").unwrap().clone() else {
            panic!("object")
        };
        let count = Rc::new(Cell::new(0));
        runtime
            .set_global(
                "proxy",
                Value::NativeObject(Rc::new(Proxy {
                    object,
                    casts: count.clone(),
                })),
            )
            .unwrap();
        casts.push(count);
        runtime.method_local_plans_executed = 0;
    }
    test_support::compare_source(&mut a, &mut b, "$seen=$proxy->f();$again=$proxy->f();").unwrap();
    assert_eq!(casts[0].get(), casts[1].get());
    assert!(casts[0].get() > 0);
    assert_eq!(a.method_local_plans_executed, 0);
    assert_eq!(b.method_local_plans_executed, 0);
    assert_eq!(field(&a), field(&b));
}
