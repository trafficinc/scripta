use super::*;
use crate::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, test_support};
use std::cell::Cell;

fn pair() -> (Runtime, Runtime) {
    let (a, mut b) = test_support::runtimes(RuntimeOptions::default());
    b.disable_empty_method_scopes = true;
    (a, b)
}
fn clean_pool(runtime: &Runtime) {
    let env = &runtime.environments;
    assert!(env.empty_scopes.len() <= MAX_EMPTY_SCOPES);
    assert!(env.spare_scopes.len() <= MAX_SPARE_SCOPES);
    for &id in &env.empty_scopes {
        let scope = env.scope(id);
        assert!(scope.parent.is_none());
        assert!(!scope.function_boundary && !scope.captured);
        assert!(scope.variables.iter().next().is_none());
        assert!(
            scope.functions.is_empty()
                && scope.classes.is_empty()
                && scope.aliases.is_empty()
                && scope.loaded_modules.is_empty()
        );
    }
}
fn field(runtime: &Runtime) -> (Option<Value>, usize) {
    let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
        panic!("object")
    };
    let instance = object
        .as_any()
        .unwrap()
        .downcast_ref::<ScriptObject>()
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
fn cold_warm_local_branch_and_bound_methods_keep_logical_scope_counts() {
    for source in [
        "class C{$x=0;function f($v:int):int{$this->x+=$v;return $this->x;}}$c=new C();$bound=$c->f;$seen=[$c->f(1),$bound(2),$c->f(3)];",
        "class C{$x=0;function f($v:int):int{$a=$v+1;$this->x+=$a;return $this->x;}}$c=new C();$seen=[$c->f(1),$c->f(2),$c->f(3)];",
        "class C{$x=0;function f($v:int):int{$this->x+=$v;if($v>0){return $this->x;}return 0;}}$c=new C();$seen=[$c->f(1),$c->f(0),$c->f(2)];",
    ] {
        let (mut a, mut b) = pair();
        test_support::compare_source(&mut a, &mut b, source).unwrap();
        assert_eq!(a.method_parameter_scopes_reused, 2);
        assert_eq!(a.empty_method_parameter_scopes_released, 3);
        assert_eq!(b.method_parameter_scopes_reused, 0);
        assert_eq!(b.empty_method_parameter_scopes_released, 0);
        assert_eq!(a.environments.scope_entries, b.environments.scope_entries);
        assert_eq!(field(&a), field(&b));
        clean_pool(&a);
        clean_pool(&b);
    }
}

#[test]
fn warm_parameter_reuse_preserves_exact_limits_errors_and_recovery() {
    let mut options = Vec::new();
    for scopes in 0..9 {
        for steps in 0..40 {
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
        test_support::compare_source(&mut a,&mut b,"class C{$x:int=0;function f($v:int):int{$a=$v;$this->x+=$a;if($v>0){{return $this->x;}}return 0;}}$c=new C();$warm=$c->f(1);$again=$c->f(0);").unwrap();
        a.method_parameter_scopes_reused = 0;
        a.empty_method_parameter_scopes_released = 0;
        a.options = option.clone();
        b.options = option;
        let _ = test_support::compare_source(&mut a, &mut b, "$seen=$c->f(2);$other=$c->f(-1);");
        assert_eq!(field(&a), field(&b));
        assert_eq!(a.environments.scope_entries, b.environments.scope_entries);
        clean_pool(&a);
        clean_pool(&b);
        if a.options.max_value_bytes.is_some() {
            assert_eq!(a.method_parameter_scopes_reused, 0);
            assert_eq!(a.empty_method_parameter_scopes_released, 0);
        }
        a.options = RuntimeOptions::default();
        b.options = RuntimeOptions::default();
        test_support::compare_source(&mut a, &mut b, "$recovered=1;").unwrap();
    }
}

#[test]
fn cold_and_warm_admitted_failures_release_only_empty_parameter_records() {
    for source in [
        "class C{$x=0;function f($v:int):int{if($v>0){$this->x=9;return false;}return 0;}}$c=new C();try{$c->f(1);}catch(TypeError $e){}$c->f(1);",
        "class C{$x:int=0;function f($v:int):int{$this->x=9;$this->x=true;return $v;}}$c=new C();try{$c->f(0);}catch(TypeError $e){}$c->f(1);",
        "class C{$x=0;function f($v:int):int{$this->x=9;return 10/$v;}}$c=new C();try{$c->f(0);}catch(ArithmeticError $e){}$c->f(0);",
    ] {
        let (mut a, mut b) = pair();
        assert!(
            test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;").is_err()
        );
        assert!(a.method_parameter_scopes_reused > 0);
        assert!(a.empty_method_parameter_scopes_released >= 2);
        assert_eq!(field(&a), field(&b));
        assert_eq!(a.environments.scope_entries, b.environments.scope_entries);
        clean_pool(&a);
    }
}

#[test]
fn warmed_hint_declines_nonscalar_arguments_without_retaining_resources() {
    #[derive(Debug)]
    struct Resource(Rc<Cell<usize>>);
    impl NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("none".into())
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let (mut a, mut b) = pair();
    let mut resources = Vec::new();
    let mut weak = Vec::new();
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
        runtime.execute_source("class C{$x=0;function f($unused):int{$this->x+=1;return $this->x;}}$c=new C();$warm=$c->f(0);$r=probe();",&mut Vec::new()).unwrap();
        let Value::NativeObject(resource) = runtime.get_global("r").unwrap() else {
            panic!("resource")
        };
        weak.push(Rc::downgrade(resource));
        resources.push(drops);
    }
    let releases = a.empty_method_parameter_scopes_released;
    test_support::compare_source(&mut a, &mut b, "$seen=$c->f($r);$again=$c->f($r);").unwrap();
    assert_eq!(a.method_parameter_scopes_reused, 2);
    assert_eq!(a.empty_method_parameter_scopes_released, releases);
    assert_eq!(field(&a), field(&b));
    for item in &weak {
        assert_eq!(item.strong_count(), 1);
    }
    for drops in &resources {
        assert_eq!(drops.get(), 0);
    }
    clean_pool(&a);
    test_support::compare_source(&mut a, &mut b, "$r=null;").unwrap();
    for drops in resources {
        assert_eq!(drops.get(), 1);
    }
}

#[test]
fn shared_warm_declarations_keep_captured_binding_fallbacks() {
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
        Rc::get_mut(
            runtime
                .environments
                .scope_mut(module)
                .classes
                .get_mut("D")
                .unwrap(),
        )
        .unwrap()
        .declaration
        .methods[0] = method;
        runtime
            .execute_source("$object=new D();", &mut Vec::new())
            .unwrap();
        let object = runtime
            .environments
            .scope_mut(module)
            .variables
            .remove("object")
            .unwrap();
        runtime.environments.leave(previous);
        runtime.set_global("c", object).unwrap();
        modules.push(module);
    }
    let releases = a.empty_method_parameter_scopes_released;
    test_support::compare_source(&mut a, &mut b, "$seen=$c->f();").unwrap();
    assert_eq!(a.method_parameter_scopes_reused, 1);
    assert_eq!(a.empty_method_parameter_scopes_released, releases);
    for (runtime, module) in [(&a, modules[0]), (&b, modules[1])] {
        assert_eq!(
            runtime.environments.scope(module).variables.get("temp"),
            Some(&Value::Int(1))
        );
    }
    assert_eq!(field(&a), field(&b));
    clean_pool(&a);
}

#[test]
fn warmed_proxy_fallback_keeps_observable_downcasts_and_general_cleanup() {
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
        fn get_property(&self, _: &str) -> Result<Value, String> {
            panic!("script downcast")
        }
    }
    let (mut a, mut b) = pair();
    test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f():int{$this->x+=1;if($this->x%2===0){return $this->x;}return 0;}}$c=new C();$warm=$c->f();").unwrap();
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
    }
    let releases = a.empty_method_parameter_scopes_released;
    test_support::compare_source(&mut a, &mut b, "$seen=$proxy->f();$again=$proxy->f();").unwrap();
    assert_eq!(a.method_parameter_scopes_reused, 2);
    assert_eq!(a.empty_method_parameter_scopes_released, releases);
    assert_eq!(casts[0].get(), casts[1].get());
    assert!(casts[0].get() > 0);
    clean_pool(&a);
}

#[test]
fn repeated_method_records_stay_bounded_while_general_closures_keep_their_scopes() {
    let (mut a, mut b) = pair();
    test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f():int{$this->x+=1;return $this->x;}function make($n){return function(){return $n+=1;};}function invoke($fn):int{return $fn();}}$c=new C();$read=$c->make(10);$total=0;for($i=0;$i<5000;$i+=1){$total+=$c->f();}$seen=[$c->invoke($read),$c->invoke($read),$c->x];").unwrap();
    assert!(a.method_parameter_scopes_reused >= 4999);
    assert_eq!(a.environments.scope_entries, b.environments.scope_entries);
    assert!(a.environments.scopes.len() < 32);
    clean_pool(&a);
    clean_pool(&b);
    assert_eq!(
        a.get_global("seen"),
        Some(&Value::list(vec![
            Value::Int(11),
            Value::Int(12),
            Value::Int(5000)
        ]))
    );
    test_support::compare_source(&mut a, &mut b, "$c=null;$read=null;").unwrap();
    for runtime in [&mut a, &mut b] {
        runtime.collect_garbage();
        assert_eq!(runtime.environments.live, 1);
        clean_pool(runtime);
    }
}

#[test]
fn cancellation_keeps_completed_writes_and_detaches_empty_parameter_records() {
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
        runtime.disable_empty_method_scopes = disabled;
        runtime.execute_source("class C{$x=0;function f():int{$this->x+=1;if($this->x%2===0){{return $this->x;}}return 0;}}$c=new C();$i=0;$total=0;",&mut Vec::new()).unwrap();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cancel.store(true, Ordering::Relaxed);
        });
        let error = runtime
            .execute_source(
                "for($i=0;$i<100000000;$i+=1){$total+=$c->f();}",
                &mut Vec::new(),
            )
            .unwrap_err();
        thread.join().unwrap();
        assert!(error.message.contains("interrupted"));
        let Some(Value::Int(i)) = runtime.get_global("i") else {
            panic!("counter")
        };
        let Some(Value::Int(total)) = runtime.get_global("total") else {
            panic!("total")
        };
        let even = i / 2;
        assert_eq!(*total, even * (even + 1));
        let (Some(Value::Int(x)), _) = field(&runtime) else {
            panic!("field")
        };
        assert!(x == *i || x == i + 1);
        assert_eq!(runtime.environments.live, 2);
        assert!(runtime.stack.is_empty());
        assert!(runtime.environments.temporary_roots.is_empty());
        clean_pool(&runtime);
        assert_eq!(runtime.method_parameter_scopes_reused > 0, !disabled);
    }
}

#[test]
fn captured_module_resources_survive_collection_during_warm_scope_failure() {
    #[derive(Debug)]
    struct Resource(Rc<Cell<usize>>);
    impl NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("none".into())
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let (mut a, mut b) = pair();
    let mut drops = Vec::new();
    let mut objects = Vec::new();
    for runtime in [&mut a, &mut b] {
        let count = Rc::new(Cell::new(0));
        let log = count.clone();
        runtime
            .register(
                "probe",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| Ok(Value::NativeObject(Rc::new(Resource(log.clone())))),
                ),
            )
            .unwrap();
        let previous = runtime.environments.enter(0);
        let module = runtime.environments.current;
        runtime.execute_source("$resource=probe();class C{$x=0;function f():int{if(true){$this->x+=1;return $this->x;}return 0;}}$object=new C();",&mut Vec::new()).unwrap();
        let object = runtime
            .environments
            .scope_mut(module)
            .variables
            .remove("object")
            .unwrap();
        let Value::NativeObject(receiver) = &object else {
            panic!("object")
        };
        objects.push(Rc::downgrade(receiver));
        runtime.environments.leave(previous);
        runtime.set_global("c", object).unwrap();
        drops.push(count);
    }
    test_support::compare_source(&mut a, &mut b, "$warm=$c->f();$again=$c->f();").unwrap();
    a.options.max_scopes = Some(5);
    b.options.max_scopes = Some(5);
    assert!(test_support::compare_source(&mut a, &mut b, "$bad=$c->f();").is_err());
    assert!(a.method_parameter_scopes_reused > 0);
    for log in &drops {
        assert_eq!(log.get(), 0);
    }
    assert_eq!(field(&a), field(&b));
    clean_pool(&a);
    a.options.max_scopes = None;
    b.options.max_scopes = None;
    test_support::compare_source(&mut a, &mut b, "$c=null;").unwrap();
    for runtime in [&mut a, &mut b] {
        runtime.collect_garbage();
        assert_eq!(runtime.environments.live, 1);
        clean_pool(runtime);
    }
    for log in drops {
        assert_eq!(log.get(), 1);
    }
    for object in objects {
        assert!(object.upgrade().is_none());
    }
}
