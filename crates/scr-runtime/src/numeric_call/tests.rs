use super::*;
use crate::{NativeFunction, NativeSignature, NativeType, RuntimeOptions};

fn compare(source: &str, options: RuntimeOptions) -> Runtime {
    let mut fast = Runtime::with_options(options.clone());
    fast.force_numeric_calls = true;
    let mut slow = Runtime::with_options(options);
    slow.disable_numeric_calls = true;
    for text in [source, "$recovered=1;"] {
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
        assert_eq!(fast.stack.len(), slow.stack.len());
        for (name, value) in &fast.environments.scope(0).variables {
            if matches!(value, Value::Callable(_) | Value::NativeObject(_)) {
                continue;
            }
            if let Value::Float(a) = value {
                let Some(Value::Float(b)) = slow.get_global(name) else {
                    panic!("{name}");
                };
                assert_eq!(a.to_bits(), b.to_bits());
            } else {
                assert_eq!(Some(value), slow.get_global(name), "{name}");
            }
        }
    }
    fast
}

#[test]
fn numeric_calls_preserve_values_strict_types_error_order_and_argument_effects() {
    for source in [
        "function f($x:int):int{return($x%97)*3+1;}$result=f(123);",
        "function f($x,$y){return -($x/$y)+$x%$y;}$result=f(9.5,2.0);",
        "function f($x){return $x;}$result=f(-0.0);",
        "function f(){return 5e-324*0.5;}$result=f();",
        "function f($x){return $x+1;}$result=f(1.0);",
        "function f($x:int):int{return$x+1;}$result=f(1.0);",
        "function f($x:int):float{return$x+1;}$result=f(1);",
        "function f($x){return$x+1;}$result=f(9223372036854775807);",
        "function f($x){return-$x;}$result=f(-9223372036854775807-1);",
        "function f($x){return$x%-1;}$result=f(-9223372036854775807-1);",
        "function f($x){return$x/0.0;}$result=f(1.0);",
        "function f($x){return($x*2.0)+(1.0/0.0);}$result=f(1e308);",
        "function f($x){return$x*2.0;}try{$result=f(1e308);}catch(ArithmeticError $e){$result=7;}",
        "function f($a,$b){return$a*10+$b;}$i=0;$result=f($i+=1,$i+=1);",
        "function f($x){return$x+1;}$f=f;$result=$f(2);",
    ] {
        assert!(
            compare(source, RuntimeOptions::default()).numeric_calls_executed > 0,
            "{source}"
        );
    }
}

#[test]
fn exact_instruction_scope_value_and_call_limits_match_and_recover() {
    for source in [
        "function f($x:int):int{return$x+1;}$total=0;for($i=0;$i<4;$i+=1){$total+=f($i);}",
        "function f($x){return$x/0.0;}$total=3.0;$total=f(1.0);",
    ] {
        for scope in 0..7 {
            for instructions in 0..55 {
                compare(
                    source,
                    RuntimeOptions {
                        max_scopes: Some(scope),
                        instruction_limit: Some(instructions),
                        ..RuntimeOptions::default()
                    },
                );
            }
        }
        for bytes in [0, 1, std::mem::size_of::<Value>(), 128, 1024] {
            assert_eq!(
                compare(
                    source,
                    RuntimeOptions {
                        max_value_bytes: Some(bytes),
                        ..RuntimeOptions::default()
                    }
                )
                .numeric_calls_executed,
                0
            );
        }
        for depth in 0..5 {
            compare(
                source,
                RuntimeOptions {
                    recursion_limit: depth,
                    ..RuntimeOptions::default()
                },
            );
        }
    }
}

#[test]
fn unsupported_signatures_bodies_objects_and_nonnumeric_arguments_keep_fallback() {
    for source in [
        "function f($x=1){return$x+1;}$result=f();",
        "function f(...$x){return 1;}$result=f(1,2);",
        "function f($x){return$x;}$result=f([1,2]);$result[]=3;",
        "function f($x){return$x;}$result=f('value');",
        "function f($x){return$x;}$result=f(true);",
        "function f($x){return$x+1;}$result=f();",
        "function f($x){return$x+1;}$result=f(1,2);",
        "function make(){$x=5;return function($n){return$n+$x;};}$f=make();$result=$f(3);",
        "function f($x){if($x>0)return$x;return 0;}$result=f(1);",
        "function f($x){$x+=1;return$x;}$result=f(1);",
        "class C{function f($x){return$x+1;}}$c=new C();$result=$c->f(1);",
    ] {
        assert_eq!(
            compare(source, RuntimeOptions::default()).numeric_calls_executed,
            0,
            "{source}"
        );
    }
}

#[test]
fn declaration_identity_and_host_reentrancy_do_not_reuse_stale_plans() {
    compare(
        "function make($k){if($k){return function($x){return$x+1;};}return function($x){return$x*2;};}$a=make(true);$b=make(false);$x=$a(5);$y=$b(5);",
        RuntimeOptions::default(),
    );
    let mut runtime = Runtime::new();
    runtime
        .execute_source("function f($x){return$x+1;}$f=f;", &mut Vec::new())
        .unwrap();
    let callable = runtime.get_global("f").unwrap().clone();
    runtime
        .register(
            "host",
            NativeFunction::reentrant(
                NativeSignature::new(vec![NativeType::Any], NativeType::Any),
                |ctx, args| ctx.invoke(args[0].clone(), &[Value::Int(4)]),
            ),
        )
        .unwrap();
    runtime.set_global("f", callable.clone()).unwrap();
    runtime
        .execute_source("$result=host($f);$result=host($f);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(5)));
    let mut other = Runtime::new();
    assert!(
        other
            .invoke(callable, &[Value::Int(4)], &mut Vec::new())
            .is_err()
    );
}

#[test]
fn plan_lifetime_follows_declaration_and_bounds_reject_oversized_bodies() {
    let mut runtime = Runtime::new();
    runtime
        .execute_source(
            "function f($x){return$x+1;}$result=f(1);$result=f(2);",
            &mut Vec::new(),
        )
        .unwrap();
    let Value::Callable(f) = runtime.function("f", Span::default()).ok().unwrap() else {
        panic!()
    };
    let crate::Function::User { declaration, .. } = f.0.as_ref() else {
        panic!()
    };
    assert!(declaration.numeric_call.get().unwrap().is_some());
    let weak = std::rc::Rc::downgrade(declaration);
    drop(f);
    runtime.environments.scope_mut(0).functions.remove("f");
    runtime.collect_garbage();
    assert!(weak.upgrade().is_none());
    for count in [8, 9] {
        let params = (0..count)
            .map(|i| format!("$p{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let args = vec!["1"; count].join(",");
        let source = format!("function f({params}){{return$p0+1;}}$result=f({args});");
        assert_eq!(
            compare(&source, RuntimeOptions::default()).numeric_calls_executed,
            usize::from(count == 8)
        );
    }
    let source = format!(
        "function f($x){{return{};}}$result=f(1);",
        vec!["$x"; 40].join("+")
    );
    assert_eq!(
        compare(&source, RuntimeOptions::default()).numeric_calls_executed,
        0
    );
}

#[test]
fn exact_node_and_depth_boundaries_and_nonfinite_host_literals_fall_back() {
    fn leaf() -> Expr {
        Expr {
            kind: ExprKind::Variable("x".into()),
            span: Span::default(),
        }
    }
    fn unary(inner: Expr) -> Expr {
        Expr {
            kind: ExprKind::Unary(UnaryOp::Positive, Box::new(inner)),
            span: Span::default(),
        }
    }
    fn balanced(depth: usize) -> Expr {
        if depth == 0 {
            return leaf();
        }
        Expr {
            kind: ExprKind::Binary(
                Box::new(balanced(depth - 1)),
                BinaryOp::Add,
                Box::new(balanced(depth - 1)),
            ),
            span: Span::default(),
        }
    }
    let mut depth16 = leaf();
    for _ in 0..16 {
        depth16 = unary(depth16);
    }
    for (expression, expected) in [
        (unary(balanced(5)), 1),
        (unary(unary(balanced(5))), 0),
        (depth16.clone(), 1),
        (unary(depth16), 0),
        (
            Expr {
                kind: ExprKind::Literal(Literal::Float(f64::NAN)),
                span: Span::default(),
            },
            0,
        ),
    ] {
        let mut program =
            scr_parser::parse(&scr_lexer::lex("function f($x){return$x;}$result=f(1);").unwrap())
                .unwrap();
        let StatementKind::Function(declaration) = &mut program.statements[0].kind else {
            panic!()
        };
        declaration.body.statements[0].kind = StatementKind::Return(Some(expression));
        let mut fast = Runtime::new();
        fast.force_numeric_calls = true;
        let mut slow = Runtime::new();
        slow.disable_numeric_calls = true;
        assert_eq!(
            fast.execute(&program, &mut Vec::new()),
            slow.execute(&program, &mut Vec::new())
        );
        assert_eq!(fast.get_global("result"), slow.get_global("result"));
        assert_eq!(fast.numeric_calls_executed, expected);
    }
}

#[test]
fn host_held_numeric_closure_keeps_its_environment_during_return_checkpoint_gc() {
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Probe(Rc<Cell<usize>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    impl crate::NativeObject for Probe {
        fn type_name(&self) -> &str {
            "Probe"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("none".into())
        }
    }
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.disable_numeric_calls = disabled;
        runtime.force_numeric_calls = !disabled;
        let drops = Rc::new(Cell::new(0));
        let probe = drops.clone();
        runtime
            .register(
                "probe",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| Ok(Value::NativeObject(Rc::new(Probe(probe.clone())))),
                ),
            )
            .unwrap();
        runtime
            .execute_source(
                "function make(){$resource=probe();return function($x){return$x+1;};}$held=make();",
                &mut Vec::new(),
            )
            .unwrap();
        let Value::Callable(held) = runtime.get_global("held").unwrap().clone() else {
            panic!()
        };
        runtime.set_global("held", Value::Null).unwrap();
        for _ in 0..512 {
            let Value::Callable(dead) = Value::native_function(
                "dead",
                NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
                    Ok(Value::Null)
                }),
            ) else {
                panic!()
            };
            runtime.environments.track_function(&dead.0);
        }
        let result = runtime
            .call_callable(
                held.clone(),
                vec![Value::Int(4)],
                Span::default(),
                &mut Vec::new(),
            )
            .ok()
            .unwrap();
        assert_eq!(result, Value::Int(5));
        assert_eq!(drops.get(), 0);
        runtime.collect_garbage();
        assert_eq!(drops.get(), 0);
        drop(held);
        runtime.collect_garbage();
        assert_eq!(drops.get(), 1);
        assert_eq!(runtime.environments.live, 1);
    }
}

#[test]
fn cancellation_at_numeric_return_matches_original_diagnostics_and_cleanup() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime.disable_numeric_calls = disabled;
        runtime.force_numeric_calls = !disabled;
        runtime
            .execute_source("function f($x){return$x+1;}", &mut Vec::new())
            .unwrap();
        let Value::Callable(f) = runtime.function("f", Span::default()).ok().unwrap() else {
            panic!()
        };
        flag.store(true, Ordering::Relaxed);
        let Err(crate::Signal::Interrupted(error)) =
            runtime.call_callable(f, vec![Value::Int(1)], Span::default(), &mut Vec::new())
        else {
            panic!()
        };
        errors.push(error);
        assert!(runtime.stack.is_empty());
        assert_eq!(runtime.environments.live, 1);
        flag.store(false, Ordering::Relaxed);
        runtime
            .execute_source("$result=f(2);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(3)));
    }
    assert_eq!(errors[0], errors[1]);
}

#[test]
fn cold_calls_defer_compilation_and_reused_declarations_admit_once() {
    let mut runtime = Runtime::new();
    runtime
        .execute_source("function f($x){return$x+1;}$result=f(1);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.numeric_calls_executed, 0);
    let Value::Callable(f) = runtime.function("f", Span::default()).ok().unwrap() else {
        panic!()
    };
    let crate::Function::User { declaration, .. } = f.0.as_ref() else {
        panic!()
    };
    assert!(declaration.numeric_seen.get());
    assert!(declaration.numeric_call.get().is_none());
    runtime
        .execute_source("$result=f(2);$result=f(3);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.numeric_calls_executed, 2);
    assert!(declaration.numeric_call.get().unwrap().is_some());
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(4)));
}
