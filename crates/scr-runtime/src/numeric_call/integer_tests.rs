use super::*;
use crate::RuntimeOptions;

fn compare(source: &str, options: RuntimeOptions, warm: bool) -> Runtime {
    let mut fast = Runtime::new();
    let mut old = Runtime::new();
    old.disable_integer_numeric_calls = true;
    if warm {
        for runtime in [&mut fast, &mut old] {
            runtime
                .execute_source(
                    "function f($x:int):int{return($x%97)*3+1;}f(0);f(1);",
                    &mut Vec::new(),
                )
                .unwrap();
            runtime.integer_numeric_calls_executed = 0;
        }
    } else {
        fast.force_numeric_calls = true;
        old.force_numeric_calls = true;
    }
    fast.options = options.clone();
    old.options = options;
    let _ =
        crate::test_support::compare_with_recovery(&mut fast, &mut old, source, "$recovered=1;");
    fast
}

#[test]
fn integer_leaf_operators_preserve_values_overflow_and_first_failure() {
    for expression in [
        "+$x",
        "-$x",
        "$x+$y",
        "$x-$y",
        "$x*$y",
        "$x/$y",
        "$x%$y",
        "(-$x/$y)+($x%$y)",
        "($x+1)+(1/0)",
        "(1/0)+($x+1)",
    ] {
        for (x, y) in [
            (0, 0),
            (9, 2),
            (-9, 2),
            (9, -2),
            (i64::MIN, -1),
            (i64::MAX, 1),
        ] {
            let x = if x == i64::MIN {
                "(-9223372036854775807-1)".to_owned()
            } else {
                x.to_string()
            };
            let source = format!(
                "function f($x:int,$y:int):int{{return {expression};}}$seen=17;$seen=f({x},{y});"
            );
            let runtime = compare(&source, RuntimeOptions::default(), false);
            assert!(runtime.integer_numeric_calls_executed > 0, "{source}");
        }
    }
    for source in [
        "function f($x:int):float{return$x+1;}$seen=f(1);",
        "function f($x:int,$y:float){return$x+$y;}$seen=f(1,2);",
        "function f($x){return$x+1;}$seen=f(1,2);",
        "function f($x){return$x+1;}$seen=f();",
        "function f($a,$b){return$a*10+$b;}$i=0;$seen=f($i+=1,$i+=1);",
    ] {
        compare(source, RuntimeOptions::default(), false);
    }
}

#[test]
fn integer_leaf_guards_preserve_float_bits_mixed_errors_and_cold_admission() {
    for source in [
        "function f($x){return$x;}$seen=f(-0.0);",
        "function f($x){return-$x;}$seen=f(-0.0);",
        "function f($x){return$x*2;}$seen=f(2.0);",
        "function f($x){return$x+0.5;}$seen=f(2);",
        "function f($unused){return 7;}$seen=f(1.0);",
        "function f($x){return$x;}$seen=f('text');",
        "function f($x){return$x;}$seen=f([1,2]);",
        "function f(){return 5e-324*0.5;}$seen=f();",
    ] {
        assert_eq!(
            compare(source, RuntimeOptions::default(), false).integer_numeric_calls_executed,
            0,
            "{source}"
        );
    }
    let mut runtime = Runtime::new();
    runtime
        .execute_source("function f($x){return$x+$x;}f(1);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.integer_numeric_calls_executed, 0);
    runtime.execute_source("f(1.0);", &mut Vec::new()).unwrap();
    assert_eq!(runtime.integer_numeric_calls_executed, 0);
    runtime
        .execute_source("$seen=f(2);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.integer_numeric_calls_executed, 1);
    assert_eq!(runtime.get_global("seen"), Some(&Value::Int(4)));
    runtime
        .execute_source("$seen=f(2.0);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.integer_numeric_calls_executed, 1);
    assert_eq!(runtime.get_global("seen"), Some(&Value::Float(4.0)));
}

#[test]
fn warmed_integer_leaves_preserve_exact_limits_cancellation_and_recovery() {
    let source = "$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}";
    for scopes in 0..8 {
        for steps in (0..70).chain(500..570) {
            compare(
                source,
                RuntimeOptions {
                    max_scopes: Some(scopes),
                    instruction_limit: Some(steps),
                    ..RuntimeOptions::default()
                },
                true,
            );
        }
    }
    for depth in 0..5 {
        compare(
            source,
            RuntimeOptions {
                recursion_limit: depth,
                ..RuntimeOptions::default()
            },
            true,
        );
    }
    for bytes in [0, 1, 32, 1024] {
        assert_eq!(
            compare(
                source,
                RuntimeOptions {
                    max_value_bytes: Some(bytes),
                    ..RuntimeOptions::default()
                },
                true
            )
            .integer_numeric_calls_executed,
            0
        );
    }
    compare(
        source,
        RuntimeOptions {
            interrupt_flag: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                true,
            ))),
            ..RuntimeOptions::default()
        },
        true,
    );
    assert!(compare(source, RuntimeOptions::default(), true).integer_numeric_calls_executed > 0);
}
