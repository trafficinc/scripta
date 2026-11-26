use crate::{Runtime, RuntimeOptions, Value};

fn warmed(definition: &str, warm: &str) -> (Runtime, Runtime) {
    let mut fast = Runtime::new();
    let mut reference = Runtime::new();
    reference.disable_numeric_setup_elision = true;
    for runtime in [&mut fast, &mut reference] {
        runtime.execute_source(definition, &mut Vec::new()).unwrap();
        runtime.execute_source(warm, &mut Vec::new()).unwrap();
        assert_eq!(runtime.numeric_setup_elisions, 0);
    }
    (fast, reference)
}

#[test]
fn warmed_numeric_callbacks_reuse_plans_without_entering_empty_scopes() {
    use crate::{NativeFunction, NativeSignature, NativeType};
    let (mut fast, mut reference) = warmed("function f($x:int):int{return $x+1;}", "f(0);f(1);");
    for runtime in [&mut fast, &mut reference] {
        runtime
            .register(
                "Host::each",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::List],
                        NativeType::List,
                    ),
                    |context, args| {
                        let Value::List(items) = &args[1] else {
                            unreachable!()
                        };
                        let mut out = Vec::new();
                        for item in items {
                            out.push(context.invoke(args[0].clone(), std::slice::from_ref(item))?);
                        }
                        Ok(Value::List(out))
                    },
                ),
            )
            .unwrap();
    }
    let before_fast = fast.environments.scope_entries;
    let before_reference = reference.environments.scope_entries;
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "$seen=Host::each(f,[1,2,3,4]);echo 'done';",
    )
    .unwrap();
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::List((2..=5).map(Value::Int).collect()))
    );
    assert_eq!(fast.numeric_setup_elisions, 4);
    assert_eq!(reference.numeric_setup_elisions, 0);
    assert!(
        fast.environments.scope_entries - before_fast
            < reference.environments.scope_entries - before_reference
    );
}

#[test]
fn warm_setup_keeps_arithmetic_types_sources_and_recovery_identical() {
    for (definition, warm, call) in [
        (
            "function f($x:int):int{return $x+1;}",
            "f(0);f(1);",
            "$seen=f(9223372036854775807);",
        ),
        (
            "function f($x:int):int{return $x+1;}",
            "f(0);f(1);",
            "$seen=f(1.0);",
        ),
        (
            "function f($x:int):float{return $x;}",
            "try{f(0);}catch(TypeError $e){}try{f(1);}catch(TypeError $e){}",
            "$seen=f(2);",
        ),
        ("function f($x){return 1/$x;}", "f(1);f(2);", "$seen=f(0);"),
        (
            "function f($x){return -$x;}",
            "f(1);f(2);",
            "$seen=f(-9223372036854775808);",
        ),
        (
            "function f($x){return $x*1.0;}",
            "f(0.0);f(1.0);",
            "$seen=f(-0.0);",
        ),
        (
            "function f($x){return $x;}",
            "f(0);f(1);",
            "$seen=f('text');",
        ),
        ("function f($x){return $x;}", "f(0);f(1);", "$seen=f();"),
    ] {
        let (mut fast, mut reference) = warmed(definition, warm);
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            call,
            "$recovered=7;",
        );
    }
}

#[test]
fn quota_and_instruction_boundaries_keep_the_ordinary_setup_contract() {
    let mut options = Vec::new();
    for steps in 0..65 {
        options.push(RuntimeOptions {
            instruction_limit: Some(steps),
            ..Default::default()
        });
    }
    for scopes in 0..8 {
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
    let slot = size_of::<Value>();
    for bytes in [0, slot - 1, slot, slot + 1, 2 * slot] {
        options.push(RuntimeOptions {
            max_value_bytes: Some(bytes),
            ..Default::default()
        });
    }
    for option in options {
        let fallback = option.max_scopes.is_some() || option.max_value_bytes.is_some();
        let (mut fast, mut reference) =
            warmed("function f($x:int):int{return $x+1;}", "f(0);f(1);");
        fast.options = option.clone();
        reference.options = option;
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            "$seen=42;$seen=f(1)+f(2)+f(3);",
            "$recovered=7;",
        );
        if fallback {
            assert_eq!(fast.numeric_setup_elisions, 0);
        }
    }
}

#[test]
fn captures_defaults_variadics_methods_and_general_types_keep_their_setup() {
    for (definition, warm, call) in [
        (
            "function make($amount){return function($x){return $x+$amount;};}$f=make(7);",
            "$f(0);$f(1);",
            "$seen=$f(3);",
        ),
        ("function f($x=1){return $x+1;}", "f();f();", "$seen=f();"),
        (
            "function f(...$rest){return 7;}",
            "f(0);f(1);",
            "$seen=f(2,3);",
        ),
        (
            "class C{function f($x){return $x+1;}}$c=new C();",
            "$c->f(0);$c->f(1);",
            "$seen=$c->f(3);",
        ),
        (
            "function f($x):list{return $x;}",
            "try{f(0);}catch(TypeError $e){}try{f(1);}catch(TypeError $e){}",
            "$seen=f(3);",
        ),
        (
            "function f($x){return $x;}",
            "f(0);f(1);",
            "$seen=f([1,2]);",
        ),
    ] {
        let (mut fast, mut reference) = warmed(definition, warm);
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            call,
            "$recovered=7;",
        );
        assert_eq!(fast.numeric_setup_elisions, 0, "{call}");
    }
}
