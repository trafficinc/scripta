use super::*;

fn pair() -> (Runtime, Runtime) {
    let (fast, mut reference) = crate::test_support::runtimes(RuntimeOptions::default());
    reference.disable_compact_calls = true;
    (fast, reference)
}

#[test]
fn instructions_execute_locals_branches_loops_assignments_and_short_circuits() {
    let (mut fast, mut reference) = pair();
    let source = "function work($n:int):int{
        $sum=0;$i=0;$flag=false;
        while($i<$n){
            if($i%2===0){$sum+=$i;}else{$sum-=$i;}
            $flag=$flag || ($i===3);$i+=1;
        }
        $i=0;while($i<3){$sum+=2;$i+=1;}
        if($flag && true){return $sum;}return -1;
    }
    $seen=[work(10),work(0),work(3)];";
    crate::test_support::compare_source(&mut fast, &mut reference, source).unwrap();
    assert_eq!(fast.compact_calls_executed, 3);
    assert_eq!(reference.compact_calls_executed, 0);
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::List([1, -1, -1].map(Value::Int).into()))
    );
}

#[test]
fn errors_preserve_types_spans_traces_and_failed_assignments() {
    for body in [
        "$x=1;return 1/0;",
        "$x=9223372036854775807;return $x+1;",
        "$x=1;return $x+true;",
        "$x=1;if($x){return 3;}return 0;",
        "$x=1;$x+=null;return $x;",
        "$x=1;return -(-9223372036854775807-1);",
    ] {
        let (mut fast, mut reference) = pair();
        let source = format!("function f(){{{body}}}$kept=9;$kept=f();");
        assert!(crate::test_support::compare_source(&mut fast, &mut reference, &source).is_err());
        assert_eq!(fast.compact_calls_executed, 1, "{body}");
        assert_eq!(fast.get_global("kept"), Some(&Value::Int(9)));
    }
    for source in [
        "function f($x:int){$a=1;return $x;}$seen=f(true);",
        "function f():bool{$a=1;return $a;}$seen=f();",
        "function f():void{$a=1;}$seen=f();",
        "function f(){$a=1;return false && (1/0);}$seen=f();",
        "function f(){$a=1;return true || (1/0);}$seen=f();",
    ] {
        let (mut fast, mut reference) = pair();
        let _ = crate::test_support::compare_source(&mut fast, &mut reference, source);
        assert_eq!(fast.compact_calls_executed, 1, "{source}");
    }
}

#[test]
fn exact_instruction_and_scope_limits_match_and_recover() {
    let definition = "function f($n){$a=0;$i=0;while($i<$n){if($i<2){$a+=1;}else{$a-=1;}$i+=1;}while($i>0){$i-=1;}return $a;}";
    for options in (0..160)
        .map(|limit| RuntimeOptions {
            instruction_limit: Some(limit),
            ..Default::default()
        })
        .chain((0..10).map(|limit| RuntimeOptions {
            max_scopes: Some(limit),
            ..Default::default()
        }))
    {
        let (mut fast, mut reference) = pair();
        crate::test_support::compare_source(&mut fast, &mut reference, definition).unwrap();
        fast.options = options.clone();
        reference.options = options;
        let _ =
            crate::test_support::compare_source(&mut fast, &mut reference, "$kept=7;$kept=f(4);");
        fast.options = RuntimeOptions::default();
        reference.options = RuntimeOptions::default();
        crate::test_support::compare_source(&mut fast, &mut reference, "$seen=f(4);").unwrap();
        assert!(fast.compact_calls_executed > 0);
    }
}

#[test]
fn unsupported_bodies_receivers_captures_and_budgets_fall_back_before_work() {
    for source in [
        "function f(){$a=0;for($i=0;$i<3;$i+=1){$a+=1;}return $a;}$seen=f();",
        "function f($x=1){$a=0;return $x;}$seen=f();",
        "function f(...$xs){$a=0;return 1;}$seen=f(1,2);",
        "function f(){$a=0;return [1,2];}$seen=f();",
        "function f(){$a=0;return missing();}$seen=f();",
        "function f(){if(true){$a=1;}return $a;}$seen=f();",
        "function f(){$a=$a+1;return $a;}$seen=f();",
        "function f(){$a=0;while(true){break;}return $a;}$seen=f();",
        "function f(){$a=0;return 1;}class C{function g(){$a=0;return 1;}}$o=new C();$seen=$o->g();",
        "function outer(){$a=42;return function(){$b=0;return $a;};}$f=outer();$seen=$f();",
        "function f($x){$a=1;return $x;}$seen=f('text');",
    ] {
        let (mut fast, mut reference) = pair();
        let _ = crate::test_support::compare_source(&mut fast, &mut reference, source);
        assert_eq!(fast.compact_calls_executed, 0, "{source}");
    }
    for limit in [
        0,
        std::mem::size_of::<Value>() - 1,
        std::mem::size_of::<Value>(),
        1024,
    ] {
        let (mut fast, mut reference) = pair();
        fast.options.max_value_bytes = Some(limit);
        reference.options.max_value_bytes = Some(limit);
        let _ = crate::test_support::compare_source(
            &mut fast,
            &mut reference,
            "function f(){$a=1;return $a;}$seen=f();",
        );
        assert_eq!(fast.compact_calls_executed, 0);
    }
}

#[test]
fn cancelled_entry_unwinds_and_prepared_plans_do_not_retain_callables() {
    let flag = Arc::new(AtomicBool::new(false));
    let (mut fast, mut reference) = pair();
    for runtime in [&mut fast, &mut reference] {
        runtime.options.interrupt_flag = Some(flag.clone());
        let trigger = flag.clone();
        runtime
            .register(
                "arm",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Int),
                    move |_, _| {
                        trigger.store(true, Ordering::Relaxed);
                        Ok(Value::Int(3))
                    },
                ),
            )
            .unwrap();
    }
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "function f($n){$a=0;while($n>0){$a+=$n;$n-=1;}return $a;}$seen=f(3);",
    )
    .unwrap();
    for runtime in [&mut fast, &mut reference] {
        flag.store(false, Ordering::Relaxed);
        assert!(
            runtime
                .execute_source("$seen=f(arm());", &mut Vec::new())
                .is_err()
        );
        assert_eq!(runtime.environments.current, 0);
        assert!(runtime.stack.is_empty());
        assert_eq!(runtime.environments.live, 1);
    }
    flag.store(false, Ordering::Relaxed);
    crate::test_support::compare_source(&mut fast, &mut reference, "$seen=f(4);").unwrap();
    let Value::Callable(function) = fast.environments.scope(0).functions.get("f").unwrap() else {
        panic!("function")
    };
    let weak = Rc::downgrade(&function.0);
    fast.environments.scope_mut(0).functions.remove("f");
    fast.collect_garbage();
    assert!(weak.upgrade().is_none());
}
