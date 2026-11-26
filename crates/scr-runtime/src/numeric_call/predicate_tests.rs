use super::*;
use crate::{NativeFunction, NativeSignature, NativeType, RuntimeOptions};

fn runtimes() -> (Runtime, Runtime) {
    let mut fast = Runtime::new();
    fast.force_numeric_calls = true;
    let mut reference = Runtime::new();
    reference.disable_numeric_calls = true;
    (fast, reference)
}

#[test]
fn numeric_predicates_preserve_comparisons_types_operand_order_and_diagnostics() {
    for source in [
        "function f($x:int):bool{return $x%2===0;}$seen=f(4);",
        "function f($x):bool{return (($x%2!==0));}$seen=f(3);",
        "function f($a,$b){return $a<$b;}$seen=f(1,2);",
        "function f($a,$b){return $a<=$b;}$seen=f(2.0,2.0);",
        "function f($a,$b){return $a>$b;}$seen=f(3,2);",
        "function f($a,$b){return $a>=$b;}$seen=f(-0.0,0.0);",
        "function f($a,$b){return $a===$b;}$seen=f(1,1.0);",
        "function f($a,$b){return $a!==$b;}$seen=f(1,1.0);",
        "function f($a,$b){return $a<$b;}$seen=42;$seen=f(1,1.0);",
        "function f($a,$b){return $a+1<$b/0;}$seen=42;$seen=f(9223372036854775807,1);",
        "function f($a,$b){return $a<$b/0;}$seen=42;$seen=f(1,1);",
        "function f($x):int{return $x>0;}$seen=42;$seen=f(1);",
        "function f($x):bool{return $x*2.0>0.0;}try{$seen=f(1e308);}catch(ArithmeticError $e){$seen=7;}",
        "function f($x):bool{return $x>-1.0;}$seen=f(-0.0);",
        "function f($a,$b){return $a<$b;}$i=0;$seen=f($i+=1,$i+=1);",
    ] {
        let (mut fast, mut reference) = runtimes();
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            source,
            "$recovered=7;",
        );
        assert!(fast.numeric_predicates_executed > 0, "{source}");
    }
}

#[test]
fn warmed_predicates_keep_exact_limits_cancellation_and_recovery() {
    let mut options = Vec::new();
    for steps in 0..70 {
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
    options.push(RuntimeOptions {
        interrupt_flag: Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            true,
        ))),
        ..Default::default()
    });
    for option in options {
        let fallback = option.max_scopes.is_some() || option.max_value_bytes.is_some();
        let bytes = option.max_value_bytes.is_some();
        let (mut fast, mut reference) = runtimes();
        crate::test_support::compare_source(
            &mut fast,
            &mut reference,
            "function f($x:int):bool{return $x%2===0;}f(0);f(1);",
        )
        .unwrap();
        fast.numeric_predicates_executed = 0;
        fast.numeric_setup_elisions = 0;
        fast.options = option.clone();
        reference.options = option;
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            "$seen=42;for($i=0;$i<4;$i+=1){$seen=f($i);}",
            "$recovered=7;",
        );
        if fallback {
            assert_eq!(fast.numeric_setup_elisions, 0);
        }
        if bytes {
            assert_eq!(fast.numeric_predicates_executed, 0);
        }
    }
}

#[test]
fn predicate_callbacks_use_the_public_bridge_and_keep_annotation_failures() {
    let (mut fast, mut reference) = runtimes();
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
                            out.push(context.invoke_owned(args[0].clone(), [item.clone()])?);
                        }
                        Ok(Value::List(out))
                    },
                ),
            )
            .unwrap();
    }
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "function f($x:int):bool{return $x%2===0;}$seen=Host::each(f,[1,2,3,4]);",
    )
    .unwrap();
    assert_eq!(fast.numeric_predicates_executed, 4);
    assert_eq!(fast.numeric_setup_elisions, 3);
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::List(vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true)
        ]))
    );
    let _ = crate::test_support::compare_with_recovery(
        &mut fast,
        &mut reference,
        "$seen=42;$seen=Host::each(f,[2,1.0]);",
        "$recovered=7;",
    );
}

#[test]
fn unsupported_predicate_shapes_and_inputs_use_ordinary_evaluation() {
    for source in [
        "function f($x=1){return $x>0;}$seen=f();",
        "function f(...$x){return $x[0]>0;}$seen=f(1);",
        "function make($k){return function($x){return $x>$k;};}$f=make(1);$seen=$f(2);",
        "function f($x){return $x['rank']>0;}$seen=f(['rank'=>1]);",
        "function f($x){return $x>0&&$x<3;}$seen=f(2);",
        "function f($x){return !($x>0);}$seen=f(2);",
        "function f($x){if($x>0)return true;return false;}$seen=f(2);",
        "function f($x){return $x>0;}$seen=f('2');",
        "function f($x){return $x===0;}$seen=f(false);",
        "function f($x){return $x===0;}$seen=f([1]);",
        "function f($x){return $x>0;}$seen=f();",
        "function f($x){return $x>0;}$seen=f(1,2);",
        "class C{function f($x){return $x>0;}}$c=new C();$seen=$c->f(1);",
    ] {
        let (mut fast, mut reference) = runtimes();
        let _ = crate::test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            source,
            "$recovered=7;",
        );
        assert_eq!(fast.numeric_predicates_executed, 0, "{source}");
    }
}

#[test]
fn comparison_plans_cannot_be_used_as_integer_return_proofs() {
    let ast =
        scr_parser::parse(&scr_lexer::lex("function f($x:int):bool{return $x%2===0;}").unwrap())
            .unwrap();
    let StatementKind::Function(ast) = &ast.statements[0].kind else {
        panic!()
    };
    let declaration = UserDeclaration::new(ast.clone());
    assert!(!Plan::compile(&declaration).unwrap().integer_only());
    let (mut fast, mut reference) = runtimes();
    let _ = crate::test_support::compare_with_recovery(
        &mut fast,
        &mut reference,
        "function f($x:int):bool{return $x%2===0;}$seen=42;for($i=0;$i<128;$i+=1){$seen+=f($i);}",
        "$recovered=7;",
    );
}

#[test]
fn predicates_keep_cold_admission_and_bounded_plan_fallback() {
    let mut fast = Runtime::new();
    let mut reference = Runtime::new();
    reference.disable_numeric_calls = true;
    crate::test_support::compare_source(
        &mut fast,
        &mut reference,
        "function f($x:int):bool{return $x%2===0;}$seen=f(1);",
    )
    .unwrap();
    assert_eq!(fast.numeric_predicates_executed, 0);
    crate::test_support::compare_source(&mut fast, &mut reference, "$seen=f(2);").unwrap();
    assert_eq!(fast.numeric_predicates_executed, 1);
    assert_eq!(fast.numeric_setup_elisions, 0);
    crate::test_support::compare_source(&mut fast, &mut reference, "$seen=f(3);").unwrap();
    assert_eq!(fast.numeric_predicates_executed, 2);
    assert_eq!(fast.numeric_setup_elisions, 1);

    // Each side is a balanced 31-node numeric expression. Grouping the full
    // comparison gives exactly 64 nodes; one more grouping forces fallback.
    fn balanced(level: usize) -> String {
        if level == 0 {
            "$x".into()
        } else {
            format!("{}+{}", balanced(level - 1), balanced(level - 1))
        }
    }
    // Construct the balanced AST directly: textual parentheses themselves are
    // counted nodes, so a grouped textual tree would exercise a different bound.
    let mut expression =
        scr_parser::parse(&scr_lexer::lex("function f($x){return ($x===$x);}").unwrap()).unwrap();
    let StatementKind::Function(ast) = &mut expression.statements[0].kind else {
        panic!()
    };
    let StatementKind::Return(Some(root)) = &mut ast.body.statements[0].kind else {
        panic!()
    };
    let ExprKind::Grouped(comparison) = &mut root.kind else {
        panic!()
    };
    let ExprKind::Binary(left, _, right) = &mut comparison.kind else {
        panic!()
    };
    fn tree(depth: usize, leaf: &Expr) -> Expr {
        if depth == 0 {
            return leaf.clone();
        }
        Expr {
            kind: ExprKind::Binary(
                Box::new(tree(depth - 1, leaf)),
                BinaryOp::Add,
                Box::new(tree(depth - 1, leaf)),
            ),
            span: leaf.span,
        }
    }
    **left = tree(4, left);
    **right = tree(4, right);
    assert!(Plan::compile(&UserDeclaration::new(ast.clone())).is_some());
    let StatementKind::Return(Some(root)) = &mut ast.body.statements[0].kind else {
        panic!()
    };
    *root = Expr {
        kind: ExprKind::Grouped(Box::new(root.clone())),
        span: root.span,
    };
    assert!(Plan::compile(&UserDeclaration::new(ast.clone())).is_none());
    // Also execute a large unsupported source to check fallback behavior.
    let source = format!(
        "function f($x):bool{{return {}==={};}}$seen=f(1);",
        balanced(5),
        balanced(5)
    );
    let (mut fast, mut reference) = runtimes();
    crate::test_support::compare_source(&mut fast, &mut reference, &source).unwrap();
    assert_eq!(fast.numeric_predicates_executed, 0);
}
