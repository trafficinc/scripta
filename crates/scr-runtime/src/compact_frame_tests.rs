use super::*;

fn pair() -> (Runtime, Runtime) {
    let (mut fast, mut reference) = crate::test_support::runtimes(RuntimeOptions::default());
    // Exercise call/error dispatch even for shapes production deliberately
    // leaves on the ordinary path after the non-repeating-graph screen.
    fast.force_compact_frames = true;
    reference.disable_compact_calls = true;
    (fast, reference)
}
fn compare(
    fast: &mut Runtime,
    reference: &mut Runtime,
    source: &str,
) -> std::result::Result<u8, RuntimeError> {
    crate::test_support::compare_source(fast, reference, source)
}
const DIVE: &str =
    "function dive($n:int):int{$copy=$n;if($copy===0){return 0;}return 1+dive($copy-1);}";

#[test]
fn self_mutual_and_nested_calls_use_explicit_frames_and_preserve_argument_order() {
    for source in [
        format!("{DIVE}$seen=dive(20);"),
        "function even($n):bool{$x=$n;if($x===0){return true;}return odd($x-1);}function odd($n):bool{$x=$n;if($x===0){return false;}return even($x-1);}$seen=[even(20),odd(20)];".into(),
        "function add($a,$b){return $a+$b;}function work($n){$x=0;return add($x+=1,add($x+=2,$x+=4))+$x;}$seen=work(0);".into(),
        "function bump($x:int):int{return $x+1;}function work($n:int):int{$sum=0;$i=0;while($i<$n){$sum+=bump($i);$i+=1;}return $sum;}$seen=work(20);".into(),
    ] {
        let (mut fast,mut reference)=pair();
        compare(&mut fast,&mut reference,&source).unwrap();
        assert!(fast.compact_frame_calls_executed>0,"{source}");
        assert_eq!(reference.compact_frame_calls_executed,0);
    }
}

#[test]
fn exact_depth_scope_and_instruction_limits_unwind_and_recover() {
    // The AST reference itself needs more native stack for these 64-deep
    // expressions in debug builds. Keep the actual depth assertions intact.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(check_limits)
        .unwrap()
        .join()
        .unwrap();
}
fn check_limits() {
    for limit in [0, 1, 2, 8, 63, 64, 128] {
        for n in [0, 1, 7, 62, 63, 64] {
            let (mut fast, mut reference) = pair();
            compare(&mut fast, &mut reference, DIVE).unwrap();
            fast.options.recursion_limit = limit;
            reference.options.recursion_limit = limit;
            let result = compare(
                &mut fast,
                &mut reference,
                &format!("$kept=9;$kept=dive({n});"),
            );
            assert_eq!(result.is_ok(), n < limit.min(MAX_RECURSION_LIMIT));
        }
    }
    for options in (0..100)
        .map(|n| RuntimeOptions {
            instruction_limit: Some(n),
            ..Default::default()
        })
        .chain((0..16).map(|n| RuntimeOptions {
            max_scopes: Some(n),
            ..Default::default()
        }))
    {
        let (mut fast, mut reference) = pair();
        compare(&mut fast, &mut reference, DIVE).unwrap();
        fast.options = options.clone();
        reference.options = options;
        let _ = compare(&mut fast, &mut reference, "$kept=9;$kept=dive(4);");
        fast.options = RuntimeOptions::default();
        reference.options = RuntimeOptions::default();
        compare(&mut fast, &mut reference, "$seen=dive(4);").unwrap();
        assert!(fast.compact_frame_calls_executed > 0);
    }
}

#[test]
fn vm_reaches_the_call_boundary_on_the_default_test_thread_stack() {
    let mut runtime = Runtime::new();
    runtime.execute_source(DIVE, &mut Vec::new()).unwrap();
    runtime
        .execute_source("$seen=dive(63);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("seen"), Some(&Value::Int(63)));
    assert_eq!(runtime.compact_frame_calls_executed, 63);
    assert!(
        runtime
            .execute_source("$seen=dive(64);", &mut Vec::new())
            .is_err()
    );
    assert_eq!(runtime.environments.live, 1);
    assert!(runtime.stack.is_empty());
}

#[test]
fn existing_recursive_plans_keep_precedence() {
    let mut runtime = Runtime::new();
    runtime.execute_source("function fib($n:int):int{if($n<2){return $n;}return fib($n-1)+fib($n-2);}$seen=fib(10);",&mut Vec::new()).unwrap();
    assert_eq!(runtime.get_global("seen"), Some(&Value::Int(55)));
    assert!(runtime.prepared_calls_executed > 0);
    assert_eq!(runtime.compact_frame_calls_executed, 0);
}

#[test]
fn production_base_return_guard_skips_setup_and_tracks_parameter_aliases() {
    let (mut fast, mut reference) = pair();
    fast.force_compact_frames = false;
    compare(&mut fast, &mut reference, DIVE).unwrap();
    compare(&mut fast, &mut reference, "$seen=dive(0);").unwrap();
    assert_eq!(fast.compact_calls_executed, 0);
    compare(&mut fast, &mut reference, "$seen=dive(4);").unwrap();
    assert_eq!(fast.compact_frame_calls_executed, 4);
    for source in [
        "function f($a,$b){$copy=$a;$a=$b;if($copy===0){return 9;}return f($a-1,$b-1);}$seen=f(0,4);",
        "function f($a,$b){$copy=$a;$a=$b;if($copy===0){return 9;}return f($a-1,$b-1);}$seen=f(2,1);",
        "function f($a:int){$copy=$a;if($copy===0){return 9;}return f($a-1);}$seen=f(true);",
    ] {
        let (mut fast, mut reference) = pair();
        fast.force_compact_frames = false;
        let _ = compare(&mut fast, &mut reference, source);
    }
}

#[test]
fn graph_and_nested_argument_stack_bounds_fall_back_before_instructions() {
    for count in [MAX_RECURSION_LIMIT, MAX_RECURSION_LIMIT + 1] {
        let mut source = "function f0(){$x=0;if(false){return f1();}return 0;}".to_string();
        for index in 1..count {
            let expression = if index + 1 < count {
                format!("f{}()", index + 1)
            } else {
                "0".into()
            };
            source.push_str(&format!("function f{index}(){{$x=0;return {expression};}}"));
        }
        source.push_str("$seen=f0();");
        let (mut fast, mut reference) = pair();
        compare(&mut fast, &mut reference, &source).unwrap();
        assert_eq!(
            fast.compact_calls_executed,
            usize::from(count == MAX_RECURSION_LIMIT)
        );
    }
    for depth in [4, 5] {
        let mut expression = "0".to_string();
        for _ in 0..depth {
            expression = format!("h(0,0,0,0,0,0,0,{expression})");
        }
        let source = format!(
            "function h($a,$b,$c,$d,$e,$f,$g,$h){{$x=0;return $h;}}function work(){{$x=0;return {expression};}}$seen=work();"
        );
        let (mut fast, mut reference) = pair();
        compare(&mut fast, &mut reference, &source).unwrap();
        assert_eq!(fast.compact_frame_calls_executed > 0, depth == 4);
    }
}

#[test]
fn production_gate_preserves_shallow_calls_rebinding_loops_and_mutual_recursion() {
    let (mut fast, mut reference) = pair();
    fast.force_compact_frames = false;
    compare(&mut fast,&mut reference,"function leaf($x){return $x+1;}function work($x){$copy=$x;return leaf($copy);}$seen=work(4);$seen=work(4);").unwrap();
    assert_eq!(fast.compact_frame_calls_executed, 0);
    for runtime in [&mut fast, &mut reference] {
        runtime.environments.scope_mut(0).functions.remove("leaf");
    }
    compare(
        &mut fast,
        &mut reference,
        "function leaf($n){$copy=$n;if($copy===0){return 4;}return 1+leaf($copy-1);}$seen=work(4);",
    )
    .unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(8)));
    assert!(fast.compact_frame_calls_executed > 0);
    for source in [
        "function leaf($x){return $x+1;}function work($n){$i=0;$sum=0;while($i<$n){$sum+=leaf($i);$i+=1;}return $sum;}$seen=work(4);",
        "function a($n){$x=$n;if($x===0){return 1;}return b($x-1);}function b($n){$x=$n;if($x===0){return 0;}return a($x-1);}$seen=a(4);",
    ] {
        let (mut fast, mut reference) = pair();
        fast.force_compact_frames = false;
        compare(&mut fast, &mut reference, source).unwrap();
        assert!(fast.compact_frame_calls_executed > 0);
    }
}

#[test]
fn rejected_graph_opt_out_keeps_later_native_to_script_rebinding_correct() {
    let (mut fast, mut reference) = pair();
    fast.force_compact_frames = false;
    for runtime in [&mut fast, &mut reference] {
        runtime
            .register(
                "leaf",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                    |_, args| Ok(args[0].clone()),
                ),
            )
            .unwrap();
    }
    compare(
        &mut fast,
        &mut reference,
        "function work($n){$x=$n;return leaf($x);}$seen=work(4);$seen=work(4);",
    )
    .unwrap();
    assert_eq!(fast.compact_frame_calls_executed, 0);
    for runtime in [&mut fast, &mut reference] {
        runtime.environments.scope_mut(0).functions.remove("leaf");
    }
    compare(
        &mut fast,
        &mut reference,
        "function leaf($n){$copy=$n;if($copy===0){return 8;}return 1+leaf($copy-1);}$seen=work(4);",
    )
    .unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(12)));
    assert!(fast.compact_frame_calls_executed > 0);
}

#[test]
fn arity_type_arithmetic_and_short_circuit_failures_match_reference_traces() {
    for source in [
        "function g($x:int):int{return $x;}function f(){$x=0;return g(1,2);}$kept=9;$kept=f();",
        "function g($x:int):int{return $x;}function f(){$x=0;return g(1/0,2);}$kept=9;$kept=f();",
        "function g($x:int):int{return $x;}function f(){$x=0;return g(true);}$kept=9;$kept=f();",
        "function g():bool{$x=1;return $x;}function f(){$x=0;return g();}$kept=9;$kept=f();",
        "function g($x){return 1/$x;}function f(){$x=0;return g($x);}$kept=9;$kept=f();",
        "function g(){return 1/0;}function f(){$x=0;return false && g();}$seen=f();",
    ] {
        let (mut fast, mut reference) = pair();
        let _ = compare(&mut fast, &mut reference, source);
        assert!(fast.compact_calls_executed > 0, "{source}");
    }
    // Errors at the call site precede parameter entry, including at depth limits.
    let (mut fast, mut reference) = pair();
    compare(
        &mut fast,
        &mut reference,
        "function g($x){return $x;}function f(){$x=0;return g(1/0);}",
    )
    .unwrap();
    fast.options.recursion_limit = 1;
    reference.options.recursion_limit = 1;
    assert!(compare(&mut fast, &mut reference, "$seen=f();").is_err());
}

#[test]
fn graph_rebinding_sources_and_lifetimes_are_checked_at_each_entry() {
    let (mut fast, mut reference) = pair();
    compare(&mut fast,&mut reference,"function leaf($x){$a=0;return $x+1;}function work($x){$a=0;return leaf($x);}$seen=work(4);").unwrap();
    for runtime in [&mut fast, &mut reference] {
        let Value::Callable(old) = runtime
            .environments
            .scope_mut(0)
            .functions
            .remove("leaf")
            .unwrap()
        else {
            panic!("leaf")
        };
        let weak = Rc::downgrade(&old.0);
        drop(old);
        runtime.collect_garbage();
        assert!(weak.upgrade().is_none());
    }
    compare(
        &mut fast,
        &mut reference,
        "function leaf($x){$a=0;return $x+9;}$seen=work(4);",
    )
    .unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(13)));
    for runtime in [&mut fast, &mut reference] {
        runtime.environments.scope_mut(0).functions.remove("leaf");
    }
    compare(
        &mut fast,
        &mut reference,
        "function leaf($x){$a=0;return 1/0;}",
    )
    .unwrap();
    assert!(compare(&mut fast, &mut reference, "$kept=9;$kept=work(4);").is_err());
}

#[test]
fn native_calls_and_unsupported_transitive_bodies_fall_back_before_execution() {
    for source in [
        "function g($x){return str($x);}function f(){$x=1;return g($x);}$seen=f();",
        "function g($x=1){return $x;}function f(){$x=0;return g();}$seen=f();",
        "function g($x){return [1,$x];}function f(){$x=0;return g($x);}$seen=f();",
        "function f(){$x=0;return missing(1/0);}$seen=f();",
        "function f(){$x=0;return missing();}$seen=f();",
    ] {
        let (mut fast, mut reference) = pair();
        let _ = compare(&mut fast, &mut reference, source);
        assert_eq!(fast.compact_frame_calls_executed, 0, "{source}");
    }
    let (mut fast, mut reference) = pair();
    let mut foreign = Runtime::new();
    foreign
        .execute_source("function alien($x){return $x;}", &mut Vec::new())
        .unwrap();
    let alien = foreign.environments.scope(0).functions["alien"].clone();
    for runtime in [&mut fast, &mut reference] {
        runtime
            .environments
            .scope_mut(0)
            .functions
            .insert("alien".into(), alien.clone());
    }
    assert!(
        compare(
            &mut fast,
            &mut reference,
            "function f(){$x=0;return alien($x);}$seen=f();"
        )
        .is_err()
    );
    assert_eq!(fast.compact_frame_calls_executed, 0);
}

#[test]
fn cancellation_before_vm_entry_preserves_results_and_recovers() {
    let flag = Arc::new(AtomicBool::new(false));
    let (mut fast, mut reference) = pair();
    compare(&mut fast, &mut reference, DIVE).unwrap();
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
                        Ok(Value::Int(4))
                    },
                ),
            )
            .unwrap();
        flag.store(false, Ordering::Relaxed);
        assert!(
            runtime
                .execute_source("$kept=9;$kept=dive(arm());", &mut Vec::new())
                .is_err()
        );
        assert_eq!(runtime.get_global("kept"), Some(&Value::Int(9)));
        assert_eq!(runtime.environments.live, 1);
        assert!(runtime.stack.is_empty());
    }
    flag.store(false, Ordering::Relaxed);
    compare(&mut fast, &mut reference, "$seen=dive(4);").unwrap();
}
