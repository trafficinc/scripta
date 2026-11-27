use super::*;
use crate::{NativeFunction, NativeSignature, NativeType, RuntimeOptions, test_support};

fn pair() -> (Runtime, Runtime) {
    let (fast, mut reference) = test_support::runtimes(RuntimeOptions::default());
    reference.disable_captured_calls = true;
    (fast, reference)
}

fn compare(fast: &mut Runtime, reference: &mut Runtime, source: &str) {
    test_support::compare_source(fast, reference, source).unwrap();
}

fn warmed(definition: &str) -> (Runtime, Runtime) {
    let (mut fast, mut reference) = pair();
    compare(&mut fast, &mut reference, definition);
    compare(&mut fast, &mut reference, "$f(1);");
    assert_eq!(fast.captured_calls_executed, 0);
    compare(&mut fast, &mut reference, "$f(2);");
    (fast, reference)
}

#[test]
fn warm_numeric_captures_omit_both_scopes_and_parameter_bindings() {
    let (mut fast, mut reference) = warmed(
        "function make($amount){return function($x:int):int{return $x+$amount;};}$f=make(7);",
    );
    assert_eq!(fast.captured_calls_executed, 1);
    let before_fast = fast.environments.scope_entries;
    let before_reference = reference.environments.scope_entries;
    compare(&mut fast, &mut reference, "$seen=$f(3)+$f(4);");
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(21)));
    assert_eq!(fast.captured_calls_executed, 3);
    assert_eq!(reference.captured_calls_executed, 0);
    assert_eq!(fast.environments.scope_entries, before_fast);
    assert_eq!(reference.environments.scope_entries - before_reference, 4);
}

#[test]
fn shared_captures_remain_live_across_mutation_types_and_caller_shadowing() {
    let (mut fast, mut reference) = pair();
    compare(
        &mut fast,
        &mut reference,
        r#"
        function make($amount){
            return [function($x){return $x+$amount+$amount;},
                    function($next){$amount=$next;}];
        }
        $pair=make(7);$f=$pair[0];$set=$pair[1];
        $f(0);$a=$f(1);$set(10);$b=$f(2);
        $set(1.5);$c=$f(2.5);
        $set("text");try{$f(1);}catch(TypeError $e){$caught=true;}
        $set(4);$d=$f(3);
        function caller($amount,$f){return $f(1);}
        $e=caller(999,$f);
        function shadow($amount){return function($amount){return $amount+1;};}
        $shadow=shadow(100);$shadow(1);$shadow(2);$g=$shadow(3);
    "#,
    );
    for (name, value) in [("a", 15), ("b", 22), ("d", 11), ("e", 9), ("g", 4)] {
        assert_eq!(fast.get_global(name), Some(&Value::Int(value)));
    }
    assert_eq!(fast.get_global("c"), Some(&Value::Float(5.5)));
    assert!(fast.captured_calls_executed >= 5);
}

#[test]
fn capture_lookup_observes_store_promotion_and_the_defining_environment() {
    let (mut fast, mut reference) =
        warmed("function make($amount){return function($x){return $x+$amount;};}$f=make(7);");
    for runtime in [&mut fast, &mut reference] {
        let Value::Callable(callable) = runtime.get_global("f").unwrap() else {
            panic!()
        };
        let crate::Function::User { environment, .. } = callable.0.as_ref() else {
            panic!()
        };
        let mut environment = *environment;
        while runtime
            .environments
            .scope(environment)
            .variables
            .get("amount")
            .is_none()
        {
            environment = runtime.environments.scope(environment).parent.unwrap();
        }
        let variables = &mut runtime.environments.scope_mut(environment).variables;
        for i in 0..40 {
            variables.insert(format!("added{i}"), Value::Int(i));
        }
        *variables.get_mut("amount").unwrap() = Value::Int(9);
        runtime.collect_garbage();
    }
    compare(&mut fast, &mut reference, "$seen=$f(4);");
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(13)));
}

#[test]
fn errors_preserve_operand_order_annotations_spans_stacks_and_destinations() {
    for (expression, annotation, capture, warm, call, hits) in [
        (
            "$x+$amount",
            ":int",
            "1",
            "$f(1);$f(2);",
            "$f(9223372036854775807)",
            true,
        ),
        ("$x+$amount", ":int", "1", "$f(1);$f(2);", "$f(1.0)", true),
        (
            "$x+$amount",
            ":float",
            "1",
            "try{$f(1);}catch(TypeError $e){}try{$f(2);}catch(TypeError $e){}",
            "$f(3)",
            true,
        ),
        (
            "($x/0)+$missing",
            "",
            "1",
            "try{$f(1);}catch(ArithmeticError $e){}try{$f(2);}catch(ArithmeticError $e){}",
            "$f(3)",
            false,
        ),
        ("$x+$missing", "", "1", "", "$f(3)", false),
        (
            "$x<$amount",
            ":bool",
            "1.0",
            "try{$f(1);}catch(TypeError $e){}try{$f(2);}catch(TypeError $e){}",
            "$f(3)",
            true,
        ),
        (
            "($x*2.0)+$amount",
            "",
            "1.0",
            "$f(1.0);$f(2.0);",
            "$f(1e308)",
            true,
        ),
    ] {
        let (mut fast, mut reference) = pair();
        let definition = format!(
            "function make($amount){{return function($x){annotation}{{return {expression};}};}}$f=make({capture});"
        );
        compare(&mut fast, &mut reference, &definition);
        let _ = test_support::compare_source(&mut fast, &mut reference, warm);
        let source = format!("$seen=42;$seen={call};");
        assert!(
            test_support::compare_with_recovery(
                &mut fast,
                &mut reference,
                &source,
                "$recovered=7;",
            )
            .is_err(),
            "{expression}"
        );
        assert_eq!(fast.get_global("seen"), Some(&Value::Int(42)));
        assert_eq!(fast.captured_calls_executed > 0, hits, "{expression}");
    }
}

#[test]
fn comparisons_and_float_signs_use_all_current_numeric_inputs() {
    let (mut fast, mut reference) = pair();
    compare(
        &mut fast,
        &mut reference,
        r#"
        function make($amount){return function($x){return $x+$amount;};}
        $f=make(-0.0);$f(-0.0);$a=$f(-0.0);
        function comparison($amount){return function($x):bool{return $x===$amount;};}
        $g=comparison(1.0);$g(1);$b=$g(1);$c=$g(1.0);
        function less($amount){return function($x):bool{return $x<$amount;};}
        $h=less(3);$h(1);$d=$h(2);$e=$h(3);
    "#,
    );
    let Some(Value::Float(value)) = fast.get_global("a") else {
        panic!()
    };
    assert_eq!(value.to_bits(), (-0.0_f64).to_bits());
    assert_eq!(fast.get_global("b"), Some(&Value::Bool(false)));
    assert_eq!(fast.get_global("c"), Some(&Value::Bool(true)));
    assert_eq!(fast.get_global("d"), Some(&Value::Bool(true)));
    assert_eq!(fast.get_global("e"), Some(&Value::Bool(false)));
    assert_eq!(fast.captured_calls_executed, 5);
}

#[test]
fn unsupported_bodies_defaults_variadics_globals_and_types_fall_back() {
    for source in [
        "function make($a){return function($x=1){return $x+$a;};}$f=make(7);$f();$f();$f();",
        "function make($a){return function(...$xs){return $a;};}$f=make(7);$f();$f();$f();",
        "function make($a){return function($x){$a+=$x;return $a;};}$f=make(7);$f(1);$f(2);$f(3);",
        "function add($x){return $x;}function make($a){return function($x){return add($x)+$a;};}$f=make(7);$f(1);$f(2);$f(3);",
        "function make($a){return function($x):list{return $x+$a;};}$f=make(7);try{$f(1);}catch(TypeError $e){}try{$f(2);}catch(TypeError $e){}",
        "$a=7;$f=function($x){return $x+$a;};$f(1);",
        "function make($a){return function($x,$unused){return $x+$a;};}$f=make(7);$f(1,[]);$f(2,[]);$f(3,[]);",
        "class C{function f($x){return $x+1;}}$c=new C();$c->f(1);$c->f(2);$c->f(3);",
    ] {
        let (mut fast, mut reference) = pair();
        let _ =
            test_support::compare_with_recovery(&mut fast, &mut reference, source, "$recovered=1;");
        assert_eq!(fast.captured_calls_executed, 0, "{source}");
    }
}

#[test]
fn warmed_quotas_instruction_limits_recursion_and_cancellation_match_general_calls() {
    let mut options = Vec::new();
    for limit in 0..35 {
        options.push(RuntimeOptions {
            instruction_limit: Some(limit),
            ..Default::default()
        });
    }
    for limit in 0..8 {
        options.push(RuntimeOptions {
            max_scopes: Some(limit),
            ..Default::default()
        });
    }
    for limit in 0..5 {
        options.push(RuntimeOptions {
            recursion_limit: limit,
            ..Default::default()
        });
    }
    let slot = size_of::<Value>();
    for bytes in [0, slot - 1, slot, slot + 1, 2 * slot, 4096] {
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
        let (mut fast, mut reference) = warmed(
            "function make($amount){return function($x:int):int{return $x+$amount;};}$f=make(7);",
        );
        let before = fast.captured_calls_executed;
        fast.options = option.clone();
        reference.options = option;
        let _ = test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            "$seen=42;$seen=$f(3)+$f(4);",
            "$recovered=1;",
        );
        if fallback {
            assert_eq!(fast.captured_calls_executed, before);
        }
    }
}

#[test]
fn reentrant_invocation_observes_capture_mutation_between_callbacks() {
    let (mut fast, mut reference) = pair();
    for runtime in [&mut fast, &mut reference] {
        runtime
            .register(
                "Host::run",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::Callable],
                        NativeType::List,
                    ),
                    |context, args| {
                        let first = context.invoke(args[0].clone(), &[Value::Int(1)])?;
                        context.invoke(args[1].clone(), &[Value::Int(20)])?;
                        let second = context.invoke(args[0].clone(), &[Value::Int(2)])?;
                        Ok(Value::list(vec![first, second]))
                    },
                ),
            )
            .unwrap();
    }
    compare(
        &mut fast,
        &mut reference,
        r#"
        function make($a){return [function($x){return $x+$a;},function($v){$a=$v;}];}
        $pair=make(7);$f=$pair[0];$set=$pair[1];$f(0);$f(1);
        $seen=Host::run($f,$set);
    "#,
    );
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::list(vec![Value::Int(8), Value::Int(22)]))
    );
    assert_eq!(fast.captured_calls_executed, 3);
}

#[test]
fn host_retained_callbacks_keep_captured_resources_alive_without_plan_owners() {
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct Resource(Rc<Cell<usize>>);
    impl crate::NativeObject for Resource {
        fn type_name(&self) -> &str {
            "Resource"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("missing".into())
        }
    }
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    for disabled in [false, true] {
        let dropped = Rc::new(Cell::new(0));
        let mut runtime = Runtime::new();
        runtime.disable_captured_calls = disabled;
        runtime
            .set_global(
                "resource",
                Value::NativeObject(Rc::new(Resource(dropped.clone()))),
            )
            .unwrap();
        runtime.execute_source(
            "function make($resource){$a=7;return function($x){return $x+$a;};}$f=make($resource);$f(0);$f(1);",
            &mut Vec::new()
        ).unwrap();
        let callback = runtime.get_global("f").unwrap().clone();
        runtime.environments.scope_mut(0).variables.remove("f");
        runtime
            .environments
            .scope_mut(0)
            .variables
            .remove("resource");
        for _ in 0..10 {
            runtime.collect_garbage();
            assert_eq!(dropped.get(), 0);
            assert_eq!(
                runtime
                    .invoke(callback.clone(), &[Value::Int(3)], &mut Vec::new())
                    .unwrap(),
                Value::Int(10)
            );
        }
        drop(callback);
        runtime.collect_garbage();
        assert_eq!(dropped.get(), 1);
    }
}

#[test]
fn input_name_node_and_depth_bounds_fall_back_without_rejecting_valid_programs() {
    for count in [7, 8] {
        let names: Vec<_> = (0..count).map(|i| format!("$c{i}")).collect();
        let definition = format!(
            "function make({}){{return function($x){{return $x+{};}};}}$f=make({});$f(0);$f(1);$seen=$f(2);",
            names.join(","),
            names.join("+"),
            vec!["1"; count].join(","),
        );
        let (mut fast, mut reference) = pair();
        compare(&mut fast, &mut reference, &definition);
        assert_eq!(fast.captured_calls_executed > 0, count == 7);
    }
    for (expression, capture, hits) in [
        (
            format!("$x+{}", "$".to_owned() + &"a".repeat(4096)),
            "a".repeat(4096),
            true,
        ),
        (
            format!("$x+{}", "$".to_owned() + &"a".repeat(4097)),
            "a".repeat(4097),
            false,
        ),
        (
            format!("{}$x+$a{}", "(".repeat(20), ")".repeat(20)),
            "a".into(),
            false,
        ),
        (format!("$a+{}", "$x+".repeat(40) + "0"), "a".into(), false),
    ] {
        let source = format!(
            "function make({}){{return function($x){{return {expression};}};}}$f=make(1);$f(0);$f(1);$seen=$f(2);",
            "$".to_owned() + &capture
        );
        let (mut fast, mut reference) = pair();
        compare(&mut fast, &mut reference, &source);
        assert_eq!(fast.captured_calls_executed > 0, hits);
    }
}

#[test]
fn warm_return_checkpoint_interrupts_and_recovers_without_entering_scopes() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let (mut fast, mut reference) =
        warmed("function make($a){return function($x){return $x+$a;};}$f=make(7);");
    let mut errors = Vec::new();
    for runtime in [&mut fast, &mut reference] {
        let flag = Arc::new(AtomicBool::new(true));
        runtime.options.interrupt_flag = Some(flag.clone());
        let Value::Callable(callback) = runtime.get_global("f").unwrap().clone() else {
            panic!()
        };
        let live = runtime.environments.live;
        let Err(crate::Signal::Interrupted(error)) = runtime.call_captured_callable(
            callback,
            vec![Value::Int(3)],
            crate::Span::default(),
            &mut Vec::new(),
        ) else {
            panic!()
        };
        errors.push(error);
        assert_eq!(runtime.environments.live, live);
        assert_eq!(runtime.environments.current, 0);
        assert!(runtime.stack.is_empty());
        flag.store(false, Ordering::Relaxed);
    }
    assert_eq!(errors[0], errors[1]);
    assert_eq!(fast.captured_calls_executed, 2);
    compare(&mut fast, &mut reference, "$seen=$f(3);");
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(10)));
}

#[test]
fn warmed_calls_still_reject_wrong_arity_and_foreign_runtime_owners() {
    let (mut fast, mut reference) =
        warmed("function make($a){return function($x){return $x+$a;};}$f=make(7);");
    let before = fast.captured_calls_executed;
    for call in ["$f();", "$f(1,2);"] {
        assert!(
            test_support::compare_with_recovery(&mut fast, &mut reference, call, "$recovered=1;")
                .is_err()
        );
    }
    assert_eq!(fast.captured_calls_executed, before);
    let callback = fast.get_global("f").unwrap().clone();
    let error = reference
        .invoke(callback, &[Value::Int(3)], &mut Vec::new())
        .unwrap_err();
    assert!(
        error
            .diagnostic()
            .unwrap()
            .message
            .contains("different runtime")
    );
}

#[test]
fn warmed_capture_errors_keep_definition_source_and_caller_trace() {
    use std::{path::PathBuf, sync::Arc};
    let definition = "function make($a){return function($x){return $x/$a;};}$f=make(0);try{$f(1);}catch(ArithmeticError $e){}try{$f(2);}catch(ArithmeticError $e){}";
    let call = "$seen=42;$seen=$f(3);";
    let mut errors = Vec::new();
    for disabled in [false, true] {
        let mut runtime = Runtime::new();
        runtime.disable_captured_calls = disabled;
        for (path, text) in [("definition.scripta", definition), ("caller.scripta", call)] {
            let program = scr_parser::parse(&scr_lexer::lex(text).unwrap()).unwrap();
            let source = Arc::new(crate::SourceFile {
                path: PathBuf::from(path),
                text: text.into(),
            });
            let result = runtime.execute_program_with_source(&program, source, &mut Vec::new());
            if text == call {
                errors.push(result.unwrap_err());
            } else {
                result.unwrap();
            }
        }
        assert_eq!(runtime.captured_calls_executed > 0, !disabled);
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(42)));
    }
    assert_eq!(errors[0], errors[1]);
    assert_eq!(
        errors[0].source.as_ref().unwrap().path,
        PathBuf::from("definition.scripta")
    );
    assert_eq!(
        errors[0].trace[1].source.as_ref().unwrap().path,
        PathBuf::from("caller.scripta")
    );
}

#[test]
fn borrowed_argument_validation_keeps_the_ordinary_capture_entry() {
    let (mut fast, mut reference) =
        warmed("function make($a){return function($x){return $x+$a;};}$f=make(7);");
    for runtime in [&mut fast, &mut reference] {
        runtime
            .register(
                "Host::borrowed",
                NativeFunction::reentrant(
                    NativeSignature::new(
                        vec![NativeType::Callable, NativeType::Int],
                        NativeType::List,
                    ),
                    |context, args| {
                        let argument = crate::InvocationArgument::new(&args[1]);
                        let first = context.invoke_arguments(args[0].clone(), [&argument])?;
                        let second = context.invoke_arguments(args[0].clone(), [&argument])?;
                        Ok(Value::list(vec![first, second]))
                    },
                ),
            )
            .unwrap();
    }
    let before = fast.captured_calls_executed;
    let scopes = fast.environments.scope_entries;
    compare(&mut fast, &mut reference, "$seen=Host::borrowed($f,3);");
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::list(vec![Value::Int(10), Value::Int(10)]))
    );
    assert_eq!(fast.captured_calls_executed, before);
    assert_eq!(fast.environments.scope_entries - scopes, 4);
    assert_eq!(fast.argument_validations_reused, 1);
}
