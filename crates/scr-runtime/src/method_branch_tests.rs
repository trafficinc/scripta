use crate::{Runtime, RuntimeOptions, Value, test_support};

fn pair() -> (Runtime, Runtime) {
    let (a, mut b) = test_support::runtimes(RuntimeOptions::default());
    b.disable_method_branch_plans = true;
    (a, b)
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
fn branches_elseif_blocks_returns_and_initialized_locals_match() {
    for source in [
        "class C{$x=0;function f($n:int):int{if($n<0){return -1;}elseif($n===0){$this->x=10;return 0;}else{$this->x+=$n;}return $this->x;}}$c=new C();$alias=$c;$bound=$c->f;$seen=[$bound(-1),$alias->f(0),$c->f(2),$c->x];",
        "class C{$x=0;function f():int{$a=1;if($a===1){return $a;}return 0;}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f($v:bool):int{$a=1;if($v){$a+=2;{if($a>1){$this->x=$a;return $a;}}}else{$a=5;}$this->x=$a;return $a;}}$c=new C();$seen=[$c->f(true),$c->f(false)];",
        "class C{$x=0;function f():int{if(false && 1/0>0){$this->x=99;}elseif(true || 1/0>0){$this->x=3;}return $this->x;}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f($v:bool):bool{$a=false;if($v)$a=true;else $a=false;return $a;}}$c=new C();$seen=[$c->f(true),$c->f(false)];",
        "class C{$x=0;function f():int{if(false){$this->x=9;}$a=3;return $a;}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f():void{if(true){{$this->x=1;}}else{}}}$c=new C();$seen=$c->f();",
        "class C{$x=0;function f():int{{{$this->x=2;return $this->x;}}return 0;}}$c=new C();$seen=$c->f();",
    ] {
        for hashed in [false, true] {
            let (mut a, mut b) = pair();
            if hashed {
                a.environments.force_hash_bindings();
                b.environments.force_hash_bindings();
            }
            test_support::compare_source(&mut a, &mut b, source).unwrap();
            assert!(a.method_branch_plans_executed > 0, "{source}");
            assert_eq!(b.method_branch_plans_executed, 0);
            assert_eq!(field(&a), field(&b));
        }
    }
}

#[test]
fn strict_conditions_failures_and_partial_writes_match() {
    for source in [
        "class C{$x=0;function f():int{$this->x=9;if(1){return 1;}return 0;}}$c=new C();$seen='kept';$seen=$c->f();",
        "class C{$x=0;function f():int{$this->x=9;if(null){return 1;}return 0;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$a=1;if(true){$this->x=9;$a/=0;}return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{if(true){{$this->x=9;return false;}}return 0;}}$c=new C();$c->f();",
        "class C{$x:int=0;function f():int{if(true){$this->x=true;}return 0;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{$this->x=9;if(1/0>0){return 1;}return 0;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{if(false){return 0;}elseif(1){return 1;}return 2;}}$c=new C();$c->f();",
        "class C{$x=9223372036854775807;function f():int{if(true){$this->x+=1;}return 0;}}$c=new C();$c->f();",
    ] {
        let (mut a, mut b) = pair();
        assert!(
            test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;").is_err()
        );
        assert!(a.method_branch_plans_executed > 0);
        assert_eq!(field(&a), field(&b));
    }
}

#[test]
fn exact_branch_instruction_scope_call_byte_limits_and_unwind_match() {
    let mut options = Vec::new();
    for scopes in 0..9 {
        for steps in 0..45 {
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
        test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f($v:int):int{$a=$v;if($v>0){$this->x+=$a;if($v===2){return $this->x;}}elseif($v===0){$this->x=0;}else{$this->x=-$v;}return $this->x;}}$c=new C();$warm=$c->f(1);").unwrap();
        a.method_branch_plans_executed = 0;
        a.options = option.clone();
        b.options = option;
        let _ = test_support::compare_source(
            &mut a,
            &mut b,
            "$seen=$c->f(2);$zero=$c->f(0);$negative=$c->f(-1);",
        );
        assert_eq!(field(&a), field(&b));
        if a.options.max_value_bytes.is_some() {
            assert_eq!(a.method_branch_plans_executed, 0);
        }
        a.options = RuntimeOptions::default();
        b.options = RuntimeOptions::default();
        test_support::compare_source(&mut a, &mut b, "$recovered=1;").unwrap();
    }
}

#[test]
fn host_statement_block_and_condition_spans_remain_distinct() {
    use scr_ast::StatementKind;
    use scr_diagnostics::Span;
    let mut program = scr_parser::parse(
        &scr_lexer::lex(
            "class C{$x=0;function f($flag):int{if($flag){return 1;}return 0;}}$c=new C();",
        )
        .unwrap(),
    )
    .unwrap();
    let StatementKind::Class(class) = &mut program.statements[0].kind else {
        panic!("class")
    };
    let StatementKind::If { branches, .. } = &mut class.methods[0].body.statements[0].kind else {
        panic!("if")
    };
    branches[0].0.span = Span::new(300, 301);
    branches[0].1.span = Span::new(100, 101);
    let StatementKind::Block(block) = &mut branches[0].1.kind else {
        panic!("block")
    };
    block.span = Span::new(200, 201);
    for scope_limit in [None, Some(4)] {
        let (mut a, mut b) = pair();
        test_support::compare_execution(&mut a, &mut b, "host branch spans", |r, out| {
            r.execute(&program, out)
        })
        .unwrap();
        a.options.max_scopes = scope_limit;
        b.options.max_scopes = scope_limit;
        let result = test_support::compare_source(&mut a, &mut b, "$seen=$c->f(true);");
        if scope_limit.is_some() {
            assert_eq!(result.unwrap_err().span, Span::new(200, 201));
        } else {
            result.unwrap();
            let error = test_support::compare_source(&mut a, &mut b, "$bad=$c->f(1);").unwrap_err();
            assert_eq!(error.span, Span::new(300, 301));
        }
        assert_eq!(field(&a), field(&b));
    }
}

#[test]
fn conditional_declarations_captures_loops_and_effects_keep_general_path() {
    for source in [
        "class C{$x=0;function f():int{if(true){$a=1;return $a;}return 0;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{if(true)$a=1;return $a;}}$c=new C();$c->f();",
        "class C{$x=0;function f():int{if(false){return $a;}$a=1;return $a;}}$c=new C();$c->f();",
        "function helper():int{return 1;}class C{$x=0;function f():int{if(true){return helper();}return 0;}}$c=new C();$c->f();",
        "class C{$x=0;function f($n:int):int{if(true){$n=1;}return $n;}}$c=new C();$c->f(2);",
        "class C{$x=0;function f():int{while(false){$this->x=1;}return $this->x;}}$c=new C();$c->f();",
        "class C{$x=0;function f():string{if(true){return 'x';}return 'y';}}$c=new C();$c->f();",
        "class C extends Error{$x=0;function f():int{if(true){$this->x=1;return 1;}return 0;}}$c=new C('bad');$seen=$c->f();",
        "class C{$x=0;$missing:int;function f():int{$this->x=9;if(false){return $this->missing;}return 0;}}$c=new C();$seen=$c->f();",
    ] {
        let (mut a, mut b) = pair();
        let _ = test_support::compare_with_recovery(&mut a, &mut b, source, "$recovered=1;");
        assert_eq!(a.method_branch_plans_executed, 0, "{source}");
    }
}

#[test]
fn structured_statement_and_scope_stack_bounds_fall_back_safely() {
    for branches in [15, 16] {
        let arms = (0..branches)
            .map(|n| {
                format!(
                    "{}($n==={n})return {n};",
                    if n == 0 { "if" } else { "elseif" }
                )
            })
            .collect::<String>();
        let source =
            format!("class C{{$x=0;function f($n:int):int{{{arms}}}}}$c=new C();$seen=$c->f(14);");
        let (mut a, mut b) = pair();
        test_support::compare_source(&mut a, &mut b, &source).unwrap();
        assert_eq!(a.method_branch_plans_executed, usize::from(branches == 15));
        assert_eq!(a.get_global("seen"), Some(&Value::Int(14)));
    }
    for depth in [16, 17] {
        let source = format!(
            "class C{{$x=0;function f():void{{{}{}}}}}$c=new C();$seen=$c->f();",
            "{".repeat(depth),
            "}".repeat(depth)
        );
        let (mut a, mut b) = pair();
        test_support::compare_source(&mut a, &mut b, &source).unwrap();
        assert_eq!(a.method_branch_plans_executed, usize::from(depth == 16));
        assert_eq!(a.environments.live, 2);
    }
}

#[test]
fn proven_branch_methods_admit_caller_loops_with_either_body_evaluator() {
    let (mut a, mut b) = pair();
    test_support::compare_source(&mut a,&mut b,"class C{$x=0;function f():int{$this->x+=1;if($this->x%2===0){return $this->x;}return 0;}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){$total+=$c->f();}$seen=$c->x;").unwrap();
    assert_eq!(a.method_branch_plans_executed, 128);
    assert_eq!(a.method_call_loops_executed, 1);
    assert_eq!(b.method_call_loops_executed, 1);
    assert_eq!(a.get_global("total"), Some(&Value::Int(4160)));
}

#[test]
fn branch_cancellation_unwinds_blocks_and_keeps_committed_state() {
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
        runtime.disable_method_branch_plans = disabled;
        runtime.execute_source("class C{$x=0;function f():int{$this->x+=1;if($this->x%2===0){{return $this->x;}}return 0;}}$c=new C();$i=0;$total=0;",&mut Vec::new()).unwrap();
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
        let even = i / 2;
        assert_eq!(*total, even * (even + 1));
        let (Some(Value::Int(x)), _) = field(&runtime) else {
            panic!("field")
        };
        assert!(x == *i || x == i + 1);
        assert!(runtime.stack.is_empty());
        assert!(runtime.environments.temporary_roots.is_empty());
        assert_eq!(runtime.environments.current, 0);
        assert_eq!(runtime.environments.live, 2);
        assert_eq!(runtime.method_branch_plans_executed > 0, !disabled);
    }
}

#[test]
fn captured_resources_survive_branch_scope_failures_and_drop_after_receiver_release() {
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
    for limit in [None, Some(6)] {
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
            runtime.execute_source("$resource=probe();class C{$x=0;function f():int{if(true){{$this->x=1;return 1;}}return 0;}}$object=new C();",&mut Vec::new()).unwrap();
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
            resources.push(drops);
            runtime.options.max_scopes = limit;
        }
        let result = test_support::compare_source(&mut a, &mut b, "$seen=$c->f();");
        if limit.is_some() {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        assert!(a.method_branch_plans_executed > 0);
        for drops in &resources {
            assert_eq!(drops.get(), 0);
        }
        for runtime in [&mut a, &mut b] {
            runtime.options.max_scopes = None;
        }
        test_support::compare_source(&mut a, &mut b, "$c=null;").unwrap();
        for runtime in [&mut a, &mut b] {
            runtime.collect_garbage();
        }
        for drops in resources {
            assert_eq!(drops.get(), 1);
        }
        for receiver in receivers {
            assert!(receiver.upgrade().is_none());
        }
    }
}

#[test]
fn warm_branch_plans_preserve_proxy_observations() {
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
        runtime.method_branch_plans_executed = 0;
    }
    test_support::compare_source(&mut a, &mut b, "$seen=$proxy->f();$again=$proxy->f();").unwrap();
    assert_eq!(casts[0].get(), casts[1].get());
    assert!(casts[0].get() > 0);
    assert_eq!(a.method_branch_plans_executed, 0);
    assert_eq!(field(&a), field(&b));
}

#[test]
fn branched_root_locals_preserve_captured_binding_writes() {
    for collision in [false, true] {
        let (mut a, mut b) = pair();
        let mut modules = Vec::new();
        for runtime in [&mut a, &mut b] {
            let previous = runtime.environments.enter(0);
            let module = runtime.environments.current;
            let name = if collision { "temp" } else { "other" };
            runtime.execute_source(&format!("${name}=99;class C{{$x=0;function f():int{{$temp=1;if(true){{$this->x=$temp;return $temp;}}return 0;}}}}$object=new C();"),&mut Vec::new()).unwrap();
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
        test_support::compare_source(&mut a, &mut b, "$seen=$c->f();").unwrap();
        assert_eq!(a.method_branch_plans_executed, usize::from(!collision));
        for (runtime, module) in [(&a, modules[0]), (&b, modules[1])] {
            assert_eq!(
                runtime.environments.scope(module).variables.get("temp"),
                if collision {
                    Some(&Value::Int(1))
                } else {
                    None
                }
            );
        }
        assert_eq!(field(&a), field(&b));
    }
}
