use super::*;
use crate::{RuntimeOptions, test_support};

fn pair() -> (Runtime, Runtime) {
    let (fast, mut reference) = test_support::runtimes(RuntimeOptions::default());
    reference.disable_structured_method_call_loops = true;
    (fast, reference)
}

fn field(runtime: &Runtime) -> Value {
    let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
        panic!("object")
    };
    let instance = object
        .as_any()
        .unwrap()
        .downcast_ref::<crate::objects::ScriptObject>()
        .unwrap();
    runtime
        .environments
        .scope(instance.scope)
        .variables
        .get("x")
        .unwrap()
        .clone()
}

#[test]
fn structured_calls_keep_aliases_argument_publication_and_thresholds() {
    for body in [
        "$next=$this->x+$v;$this->x=$next;return $next;",
        "$this->x+=$v;if($this->x%2===0){{return $this->x;}}return 0;",
        "$next=$this->x+$v;$this->x=$next;if($next%2===0){return $next;}return 0;",
        "$this->x+=$v;$p=$this->x%3;if($p===0){return $this->x*2;}elseif($p===1){{return $this->x;}}else{return 0;}",
        "$total=$this->x+$v;$this->x=$total;return $total;",
    ] {
        for count in [0, 127, 128, 129] {
            let source = format!(
                "class C{{$x:int=0;function f($v:int):int{{{body}}}}}$c=new C();$alias=$c;$total=0;for($i=0;$i<{count};$i+=1){{$total+=1;$total+=(($alias))->f(($i+$total)%5+1);}}"
            );
            let (mut fast, mut reference) = pair();
            test_support::compare_source(&mut fast, &mut reference, &source).unwrap();
            assert_eq!(fast.method_call_loops_executed, usize::from(count >= 128));
            assert_eq!(reference.method_call_loops_executed, 0);
            assert_eq!(field(&fast), field(&reference));
            assert_eq!(
                fast.environments.scope_entries,
                reference.environments.scope_entries
            );
            assert_eq!(
                fast.prepared_method_calls_executed,
                if count >= 128 { count - 1 } else { 0 }
            );
        }
    }
}

#[test]
fn structured_loop_limits_keep_checkpoints_scopes_partial_state_and_recovery() {
    let setup = "class C{$x:int=0;function f($v:int):int{$next=$this->x+$v;$this->x=$next;if($next%2===0){{return $next;}}return 0;}}$c=new C();$warm=$c->f(0);$total=0;$i=0;";
    let source = "for(;$i<128;$i+=1){$total+=1;$total+=$c->f(($i+$total)%3);}";
    let (mut a, mut b) = pair();
    test_support::compare_source(&mut a, &mut b, setup).unwrap();
    test_support::compare_source(&mut a, &mut b, source).unwrap();
    let exact = a.steps;
    let mut options = Vec::new();
    for steps in (0..85).chain([exact - 1, exact, exact + 1]) {
        options.push(RuntimeOptions {
            instruction_limit: Some(steps),
            ..Default::default()
        });
    }
    for scopes in 0..11 {
        options.push(RuntimeOptions {
            max_scopes: Some(scopes),
            ..Default::default()
        });
    }
    for depth in 0..5 {
        options.push(RuntimeOptions {
            recursion_limit: depth,
            ..Default::default()
        });
    }
    for bytes in [0, 1, 31, 32, 33, 64, 65, 66, 1024] {
        options.push(RuntimeOptions {
            max_value_bytes: Some(bytes),
            ..Default::default()
        });
    }
    for option in options {
        let (mut fast, mut reference) = pair();
        test_support::compare_source(&mut fast, &mut reference, setup).unwrap();
        fast.options = option.clone();
        reference.options = option.clone();
        let _ = test_support::compare_source(&mut fast, &mut reference, source);
        assert_eq!(field(&fast), field(&reference));
        assert_eq!(
            fast.environments.scope_entries,
            reference.environments.scope_entries
        );
        if option.max_value_bytes.is_some() {
            assert_eq!(fast.method_call_loops_executed, 0);
        }
        fast.options = RuntimeOptions::default();
        reference.options = RuntimeOptions::default();
        test_support::compare_source(&mut fast, &mut reference, "$recovered=$c->f(1);").unwrap();
    }
}

#[test]
fn structured_failures_publish_completed_caller_writes_only() {
    for (params, body, arg) in [
        (
            "$v:int",
            "$next=$this->x+$v;$this->x=$next;if($next===125){return 10/0;}return $next;",
            "1",
        ),
        (
            "$v:int",
            "$next=$this->x+1;$this->x=$next;if($next===125){return false;}return $next;",
            "1",
        ),
        (
            "$v:int",
            "$next=$this->x+1;$this->x=$next;if($next===125){$this->x=true;}return $next;",
            "1",
        ),
        (
            "$v:int",
            "$next=$this->x+1;$this->x=$next;return $next+9223372036854775807;",
            "1",
        ),
        (
            "$v:int",
            "$next=$this->x+$v;$this->x=$next;return $next;",
            "1/(125-$i)",
        ),
        (
            "$v:bool",
            "$next=1;$this->x=$next;if($v){return 1;}return 0;",
            "$i",
        ),
        (
            "$v:int",
            "$next=$this->x+1;$this->x=$next;if($next<125){return $next;}",
            "1",
        ),
    ] {
        let source = format!(
            "class C{{$x:int=0;function f({params}):int{{{body}}}}}$c=new C();$i=0;$total=0;for($i=0;$i<128;$i+=1){{$total+=1;$total+=$c->f({arg});}}"
        );
        let (mut fast, mut reference) = pair();
        assert!(
            test_support::compare_with_recovery(
                &mut fast,
                &mut reference,
                &source,
                "$recovered=1;"
            )
            .is_err()
        );
        assert_eq!(fast.method_call_loops_executed, 1, "{body}");
        assert_eq!(reference.method_call_loops_executed, 0);
        assert_eq!(field(&fast), field(&reference));
    }
}

#[test]
fn captured_local_writes_reject_cached_caller_slots() {
    let (mut fast, mut reference) = pair();
    for runtime in [&mut fast, &mut reference] {
        let previous = runtime.environments.enter(0);
        let module = runtime.environments.current;
        runtime.execute_source("$total=0;$out=0;class C{$x=0;function f():int{$this->x+=1;$total=5;if($this->x>0){return $total;}return 0;}}$c=new C();$warm=$c->f();",&mut Vec::new()).unwrap();
        let object = runtime
            .environments
            .scope(module)
            .variables
            .get("c")
            .unwrap()
            .clone();
        runtime.set_global("c", object).unwrap();
        runtime.environments.leave(previous);
    }
    test_support::compare_execution(
        &mut fast,
        &mut reference,
        "captured caller local",
        |runtime, output| {
            let Value::NativeObject(object) = runtime.get_global("c").unwrap() else {
                panic!()
            };
            let module = object
                .as_any()
                .unwrap()
                .downcast_ref::<crate::objects::ScriptObject>()
                .unwrap()
                .class
                .environment;
            runtime.environments.current = module;
            let result =
                runtime.execute_source("for($i=0;$i<128;$i+=1){$total+=1;$out+=$c->f();}", output);
            runtime.environments.current = 0;
            let variables = &runtime.environments.scope(module).variables;
            let seen = Value::list(vec![
                variables.get("total").unwrap().clone(),
                variables.get("out").unwrap().clone(),
            ]);
            runtime.set_global("seen", seen).unwrap();
            result
        },
    )
    .unwrap();
    assert_eq!(fast.method_call_loops_executed, 0);
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::list(vec![Value::Int(5), Value::Int(640)]))
    );
}

#[test]
fn shared_warm_declarations_recheck_local_capture_per_receiver() {
    for collision in [false, true] {
        let (mut fast, mut reference) = pair();
        let mut modules = Vec::new();
        for runtime in [&mut fast, &mut reference] {
            runtime.execute_source("class C{$x=0;function f():int{$temp=5;$this->x+=1;if($this->x>0){return $temp;}return 0;}}$original=new C();$warm=$original->f();",&mut Vec::new()).unwrap();
            let declaration = runtime.environments.scope(0).classes["C"]
                .declaration
                .methods[0]
                .clone();
            let previous = runtime.environments.enter(0);
            let module = runtime.environments.current;
            runtime
                .execute_source("class D{$x=0;function f():int{return 0;}}", &mut Vec::new())
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
            .methods[0] = declaration;
            if collision {
                runtime
                    .environments
                    .scope_mut(module)
                    .variables
                    .insert("temp".into(), Value::Int(99));
            }
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
        test_support::compare_source(
            &mut fast,
            &mut reference,
            "$total=0;for($i=0;$i<128;$i+=1){$total+=$c->f();}",
        )
        .unwrap();
        assert_eq!(fast.method_call_loops_executed, usize::from(!collision));
        assert_eq!(reference.method_call_loops_executed, 0);
        for (runtime, module) in [(&fast, modules[0]), (&reference, modules[1])] {
            assert_eq!(
                runtime.environments.scope(module).variables.get("temp"),
                if collision {
                    Some(&Value::Int(5))
                } else {
                    None
                }
            );
        }
    }
}

#[test]
fn structured_planning_keeps_cold_admission_and_unsupported_fallbacks() {
    for body in [
        "$next=$this->x+1;$this->x=$next;return $next;",
        "$this->x+=1;if($this->x>0){return $this->x;}return 0;",
    ] {
        let mut runtime = Runtime::new();
        runtime
            .execute_source(
                &format!("class C{{$x=0;function f():int{{{body}}}}}$c=new C();"),
                &mut Vec::new(),
            )
            .unwrap();
        let base = Expr {
            kind: ExprKind::Variable("c".into()),
            span: Span::default(),
        };
        assert!(
            runtime.environments.scope(0).classes["C"]
                .declaration
                .methods[0]
                .special_call
                .get()
                .is_none()
        );
        assert!(runtime.integer_loop_method_scope(&base, "f", 0).is_some());
        assert!(
            runtime.environments.scope(0).classes["C"]
                .declaration
                .methods[0]
                .special_call
                .get()
                .is_none()
        );
    }
    for body in [
        "if($this->x===0){$temp=1;}return $temp;",
        "$temp=helper();return $temp;",
        "$temp=0;while($temp<2){$temp+=1;}return $temp;",
    ] {
        let source = format!(
            "function helper():int{{return 1;}}class C{{$x=0;function f():int{{{body}}}}}$c=new C();$total=0;for($i=0;$i<128;$i+=1){{$total+=$c->f();}}"
        );
        let (mut fast, mut reference) = pair();
        let _ = test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            &source,
            "$recovered=1;",
        );
        assert_eq!(fast.method_call_loops_executed, 0);
    }
}
