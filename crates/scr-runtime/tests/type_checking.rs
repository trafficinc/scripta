use std::{cell::Cell, rc::Rc};

use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, Value};

#[test]
fn checking_uses_host_signatures_without_invocation_or_runtime_mutation() {
    let mut runtime = Runtime::new();
    let invoked = Rc::new(Cell::new(false));
    let observed = invoked.clone();
    runtime
        .register(
            "Host::check",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Int], NativeType::String)
                    .with_optional(vec![NativeType::Bool])
                    .with_variadic(NativeType::String),
                move |_, _| {
                    observed.set(true);
                    Ok(Value::String("value".into()))
                },
            ),
        )
        .unwrap();
    runtime.set_global("preserved", Value::Int(42)).unwrap();
    for (source, rejected) in [
        ("Host::check(1);", false),
        ("Host::check(1, true, 'a', 'b');", false),
        ("Host::check();", true),
        ("Host::check(1.0);", true),
        ("Host::check(1, 'bad');", true),
        ("Host::check(1, true, 1);", true),
        ("Host::check(1) + 1;", true),
        ("$preserved = 0; require 'missing.scripta';", false),
    ] {
        let tokens = scr_lexer::lex(source).unwrap();
        let program = scr_parser::parse(&tokens).unwrap();
        let errors = runtime.check_program(&program);
        assert_eq!(!errors.is_empty(), rejected, "{source}: {errors:?}");
        assert!(errors.iter().all(|error| error.code == "E0310"));
    }
    assert!(!invoked.get());
    assert_eq!(runtime.get_global("preserved"), Some(&Value::Int(42)));
}
