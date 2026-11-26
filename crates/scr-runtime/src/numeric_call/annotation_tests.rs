use super::*;
use crate::RuntimeOptions;
fn compare(source: &str, options: RuntimeOptions) {
    let (mut fast, mut old) = crate::test_support::runtimes(options);
    fast.force_numeric_calls = true;
    old.force_numeric_calls = true;
    old.disable_numeric_annotations = true;
    old.disable_primitive_annotations = true;
    let _ =
        crate::test_support::compare_with_recovery(&mut fast, &mut old, source, "$recovered=1;");
}
#[test]
fn cached_primitive_types_preserve_values_mismatches_and_general_resolution() {
    for source in [
        "function f($x:int):int{return$x;}$seen=f(42);",
        "function f($x:float):float{return$x;}$seen=f(-0.0);",
        "function f($x:float):float{return$x;}$seen=f(5e-324);",
        "function f($x:int):int{return$x;}try{$seen=f(1.0);}catch(TypeError $e){$seen=7;}",
        "function f($x:float):float{return$x;}$seen=f(1);",
        "function f($x:int):float{return$x;}$seen=f(1);",
        "function f($x:float):int{return$x;}$seen=f(1.0);",
        "function f($x){return$x;}$a=f(1);$b=f(1.5);",
        "function f($x):void{return$x;}$seen=f(1);",
        "function f($x:T::int):int{return$x;}$seen=f(1);",
        "class Thing{}function f($x:Thing):int{return$x;}$seen=f(1);",
        "class Thing{$value=7;}function f($x:Thing):Thing{return$x;}$obj=f(new Thing());$seen=$obj->value;",
        "function f($x:Int):int{return$x;}$seen=f(1);",
        "function f($x:bool):int{return$x;}$seen=f(1);",
        "function f($x:int):bool{return$x>0;}$seen=f(1);$seen=f(-1);",
        "function f($x:bool):bool{return$x>0;}$seen=f(1);",
        "function f($x):bool{return$x;}$seen=f(1);",
    ] {
        compare(source, RuntimeOptions::default());
    }
}
#[test]
fn cached_type_checks_keep_budget_error_order_and_recovery() {
    let source =
        "function f($x:int):float{return$x;}$total=0;for($i=0;$i<128;$i+=1){$total+=f($i);}";
    for scopes in 0..7 {
        for steps in 0..28 {
            compare(
                source,
                RuntimeOptions {
                    max_scopes: Some(scopes),
                    instruction_limit: Some(steps),
                    ..RuntimeOptions::default()
                },
            );
        }
    }
    for bytes in [0, 1, std::mem::size_of::<Value>(), 1024] {
        compare(
            source,
            RuntimeOptions {
                max_value_bytes: Some(bytes),
                ..RuntimeOptions::default()
            },
        );
    }
    for depth in 0..4 {
        compare(
            source,
            RuntimeOptions {
                recursion_limit: depth,
                ..RuntimeOptions::default()
            },
        );
    }
}
#[test]
fn primitive_class_names_remain_reserved_for_script_and_host_declarations() {
    for (name, value) in [("int", "1"), ("float", "1.0"), ("bool", "true")] {
        compare(
            &format!(
                "function f($x:{name}):{name}{{return$x;}}$seen=f({value});$seen=f({value});class {name}{{}}"
            ),
            RuntimeOptions::default(),
        );
        let mut program = scr_parser::parse(&scr_lexer::lex("class Allowed {} ").unwrap()).unwrap();
        let StatementKind::Class(class) = &mut program.statements[0].kind else {
            panic!()
        };
        class.name.name = name.into();
        let error = Runtime::new()
            .execute(&program, &mut Vec::new())
            .unwrap_err();
        assert!(
            error
                .message
                .contains("class name conflicts with a built-in type")
        );
    }
}
#[test]
fn native_objects_with_primitive_type_names_keep_generic_compatibility() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::{cell::Cell, rc::Rc};
    #[derive(Debug)]
    struct NativeInt(Rc<Cell<usize>>);
    impl NativeObject for NativeInt {
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            self.0.set(self.0.get() + 1);
            None
        }
        fn type_name(&self) -> &str {
            "int"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Ok(Value::Int(7))
        }
    }
    let mut observations = Vec::new();
    for disabled in [false, true] {
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let mut runtime = Runtime::new();
        runtime.force_numeric_calls = true;
        runtime.disable_numeric_annotations = disabled;
        runtime.disable_primitive_annotations = disabled;
        runtime
            .register(
                "make",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| Ok(Value::NativeObject(Rc::new(NativeInt(c.clone())))),
                ),
            )
            .unwrap();
        runtime
            .execute_source(
                "function f($x:int):int{return$x;}$obj=f(make());$seen=$obj->value;",
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(7)));
        assert_eq!(runtime.numeric_calls_executed, 0);
        observations.push(calls.get());
    }
    assert_eq!(observations[0], observations[1]);
    assert!(observations[0] >= 2);
}
