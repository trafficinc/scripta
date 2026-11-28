use super::*;
use crate::{RuntimeOptions, test_support};

fn outcome(result: ops::Result<Value>) -> std::result::Result<Value, (&'static str, &'static str)> {
    result.map_err(|error| match error {
        ops::Error::Type(message) => ("type", message),
        ops::Error::Index(message) => ("index", message),
        ops::Error::Arithmetic(message) => ("arithmetic", message),
        ops::Error::Fatal(message) => ("fatal", message),
    })
}

#[test]
fn scalar_operations_match_general_types_errors_and_integer_boundaries() {
    let mut values = vec![Scalar::Null, Scalar::Bool(false), Scalar::Bool(true)];
    values.extend(
        [
            i64::MIN,
            i64::MIN + 1,
            -97,
            -2,
            -1,
            0,
            1,
            2,
            97,
            i64::MAX - 1,
            i64::MAX,
        ]
        .map(Scalar::Int),
    );
    for &left in &values {
        assert_eq!(
            outcome(left.boolean().map(Value::Bool)),
            outcome(ops::boolean(left.value()).map(Value::Bool))
        );
        for op in [UnaryOp::Positive, UnaryOp::Negative, UnaryOp::Not] {
            assert_eq!(
                outcome(left.unary(op).map(Scalar::value)),
                outcome(ops::unary(op, left.value())),
                "{left:?} {op:?}"
            );
        }
        for &right in &values {
            for op in [
                BinaryOp::Add,
                BinaryOp::Subtract,
                BinaryOp::Multiply,
                BinaryOp::Divide,
                BinaryOp::Remainder,
                BinaryOp::Equal,
                BinaryOp::NotEqual,
                BinaryOp::Less,
                BinaryOp::LessEqual,
                BinaryOp::Greater,
                BinaryOp::GreaterEqual,
                BinaryOp::Concat,
                BinaryOp::And,
                BinaryOp::Or,
                BinaryOp::In,
            ] {
                assert_eq!(
                    outcome(left.binary(op, right).map(Scalar::value)),
                    outcome(ops::binary(op, left.value(), right.value())),
                    "{left:?} {op:?} {right:?}"
                );
            }
        }
    }
}

fn pair() -> (Runtime, Runtime) {
    let (fast, mut reference) = test_support::runtimes(RuntimeOptions::default());
    reference.disable_method_plans = true;
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
fn scalar_method_expressions_keep_strict_results_spans_and_partial_state() {
    for expression in [
        "$a+$b", "$a-$b", "$a*$b", "$a/$b", "$a%$b", "-$a", "+$a", "!$a", "$a===$b", "$a!==$b",
        "$a<$b", "$a<=$b", "$a>$b", "$a>=$b", "$a&&$b", "$a||$b",
    ] {
        for (a, b) in [
            ("7", "2"),
            ("-7", "2"),
            ("true", "false"),
            ("null", "0"),
            ("9223372036854775807", "1"),
            ("$min", "-1"),
            ("1", "0"),
        ] {
            let (mut fast, mut reference) = pair();
            let source = format!(
                "$min=-9223372036854775807-1;class C{{$x=0;function f($a,$b){{$this->x+=1;$v={expression};return $v;}}}}$c=new C();$seen=42;$seen=$c->f({a},{b});"
            );
            let _ = test_support::compare_with_recovery(
                &mut fast,
                &mut reference,
                &source,
                "$recovered=1;",
            );
            assert!(fast.method_plans_executed > 0, "{expression}");
            assert_eq!(field(&fast), field(&reference));
            assert_eq!(
                fast.environments.scope_entries,
                reference.environments.scope_entries
            );
        }
    }
}

#[test]
fn short_circuiting_skips_failures_and_validates_selected_conditions() {
    for expression in [
        "false&&(1/0===0)",
        "true||(1/0===0)",
        "true&&(1/0===0)",
        "false||(1/0===0)",
        "false&&1",
        "true||null",
        "true&&1",
        "false||null",
        "!(1===true)",
        "null===false",
    ] {
        let (mut fast, mut reference) = pair();
        let source = format!(
            "class C{{$x=0;function f(){{$this->x+=1;$v={expression};if($v){{$this->x+=2;return true;}}return false;}}}}$c=new C();$seen=42;$seen=$c->f();"
        );
        let _ = test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            &source,
            "$recovered=1;",
        );
        assert!(fast.method_plans_executed > 0);
        assert_eq!(field(&fast), field(&reference));
    }
}

#[test]
fn scalar_slots_preserve_limits_unwind_and_recovery() {
    let mut options = Vec::new();
    for steps in 0..75 {
        options.push(RuntimeOptions {
            instruction_limit: Some(steps),
            ..Default::default()
        });
    }
    for scopes in 0..9 {
        options.push(RuntimeOptions {
            max_scopes: Some(scopes),
            ..Default::default()
        });
    }
    for depth in 0..4 {
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
        test_support::compare_source(&mut fast,&mut reference,"class C{$x:int=1;function f($v:int):int{$n=($this->x+$v)*2;if($n%3===0){$this->x=$n/3;return $n;}$this->x=$n;return $this->x;}}$c=new C();$warm=$c->f(1);").unwrap();
        fast.options = option.clone();
        reference.options = option.clone();
        let _ = test_support::compare_source(
            &mut fast,
            &mut reference,
            "function read($c){return $c->f(2);}$seen=read($c);$again=$c->f(0);",
        );
        assert_eq!(field(&fast), field(&reference));
        assert_eq!(
            fast.environments.scope_entries,
            reference.environments.scope_entries
        );
        fast.options = RuntimeOptions::default();
        reference.options = RuntimeOptions::default();
        test_support::compare_source(&mut fast, &mut reference, "$recovered=$c->f(1);").unwrap();
    }
}

#[test]
fn full_parameter_local_field_slots_and_bound_results_stay_independent() {
    for width in [8, 64] {
        let fields = (0..width)
            .map(|i| format!("$f{i}={i};"))
            .collect::<String>();
        let updates = (0..8)
            .map(|i| format!("$this->f{i}=$l{}+{i};", i % 4))
            .collect::<String>();
        let sum = (0..8)
            .map(|i| format!("$this->f{i}"))
            .collect::<Vec<_>>()
            .join("+");
        let source = format!(
            "class C{{{fields}function f($a,$b,$c,$d):int{{$l0=$a+$b;$l1=$b+$c;$l2=$c+$d;$l3=$d+$a;{updates}return {sum};}}}}$c=new C();$bound=$c->f;$first=$c->f(1,2,3,4);$second=$bound(4,3,2,1);$seen=[$first,$second];"
        );
        let stored = (0..8)
            .map(|i| format!("$c->f{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("{source}$stored=[{stored}];");
        let (mut fast, mut reference) = pair();
        test_support::compare_source(&mut fast, &mut reference, &source).unwrap();
        assert_eq!(fast.method_plans_executed, 2);
        assert_eq!(
            fast.environments.scope_entries,
            reference.environments.scope_entries
        );
    }
}

#[test]
fn scalar_field_materialization_preserves_annotations_and_failed_destinations() {
    for body in [
        "$this->x+=1;$v=null;$this->x=$v;return 0;",
        "$this->x+=1;return false;",
        "$this->x+=1;$v=0;$this->x/=$v;return $this->x;",
        "$this->x+=1;$v=-9223372036854775807-1;return -$v;",
    ] {
        let source = format!(
            "class C{{$x:int=0;function f():int{{{body}}}}}$c=new C();$seen=42;$seen=$c->f();"
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
        assert!(fast.method_plans_executed > 0);
        assert_eq!(field(&fast), field(&reference));
        assert_eq!(fast.get_global("seen"), Some(&Value::Int(42)));
    }
}
