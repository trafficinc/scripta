use super::*;

#[test]
fn explicit_scalar_conversions() {
    for (input, expected) in [
        (Value::Null, "null"),
        (Value::Bool(true), "true"),
        (Value::Bool(false), "false"),
        (Value::Int(-12), "-12"),
        (Value::Float(1.0), "1"),
        (Value::String("é".into()), "é"),
    ] {
        assert_eq!(str_value(&[input]).unwrap(), Value::String(expected.into()));
    }
    for (input, expected) in [
        (Value::Int(12), 12),
        (Value::Bool(true), 1),
        (Value::Float(12.0), 12),
        (Value::String("-12".into()), -12),
        (Value::String(i64::MIN.to_string()), i64::MIN),
    ] {
        assert_eq!(int_value(&[input]).unwrap(), Value::Int(expected));
    }
    for (input, expected) in [
        (Value::Int(12), 12.0),
        (Value::Bool(false), 0.0),
        (Value::Float(1.5), 1.5),
        (Value::String("1.25e2".into()), 125.0),
    ] {
        assert_eq!(float_value(&[input]).unwrap(), Value::Float(expected));
    }
    for (input, expected) in [
        (Value::Bool(true), true),
        (Value::Int(0), false),
        (Value::Int(1), true),
        (Value::Float(0.0), false),
        (Value::Float(1.0), true),
        (Value::String("true".into()), true),
        (Value::String("false".into()), false),
    ] {
        assert_eq!(bool_value(&[input]).unwrap(), Value::Bool(expected));
    }
}

#[test]
fn invalid_conversions_do_not_coerce_or_truncate() {
    for input in [
        Value::Null,
        Value::Float(1.5),
        Value::Float(f64::INFINITY),
        Value::Float(-(i64::MIN as f64)),
        Value::String("12x".into()),
        Value::String(" 12".into()),
        Value::String("1.0".into()),
        Value::String("9223372036854775808".into()),
        Value::List(vec![]),
    ] {
        assert!(int_value(&[input]).is_err());
    }
    for input in [
        Value::Null,
        Value::String("abc".into()),
        Value::String("NaN".into()),
        Value::String("inf".into()),
        Value::String("1e999".into()),
        Value::List(vec![]),
    ] {
        assert!(float_value(&[input]).is_err());
    }
    for input in [
        Value::Null,
        Value::Int(2),
        Value::Float(0.5),
        Value::String("1".into()),
        Value::String("".into()),
        Value::String("TRUE".into()),
        Value::List(vec![]),
    ] {
        assert!(bool_value(&[input]).is_err());
    }
    assert!(str_value(&[Value::List(vec![])]).is_err());
    assert!(str_value(&[Value::Map(vec![])]).is_err());
}

#[test]
fn conversion_arity_is_exactly_one() {
    for convert in [str_value, int_value, float_value, bool_value] {
        assert!(convert(&[]).unwrap_err().contains("exactly one argument"));
        assert!(
            convert(&[Value::Null, Value::Null])
                .unwrap_err()
                .contains("exactly one argument")
        );
    }
}

#[test]
fn lengths_count_unicode_scalars_and_collection_entries() {
    for (value, expected) in [
        (Value::String("".into()), 0),
        (Value::String("é🦀e\u{301}".into()), 4),
        (Value::List(vec![Value::Null, Value::Int(1)]), 2),
        (
            Value::Map(vec![(
                scr_runtime::MapKey::String("x".into()),
                Value::List(vec![Value::Null]),
            )]),
            1,
        ),
    ] {
        assert_eq!(len_value(&[value]).unwrap(), Value::Int(expected));
    }
    for args in [
        vec![],
        vec![Value::Null],
        vec![Value::Int(1)],
        vec![Value::String("x".into()), Value::Null],
    ] {
        assert_eq!(len_value(&args).unwrap_err().kind, "TypeError");
    }
}

#[test]
fn sleep_validates_duration_permissions_and_cancellation() {
    use scr_runtime::ContextOptions;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut options = ContextOptions::default();
    let flag = std::sync::Arc::new(AtomicBool::new(false));
    let mut output = Vec::new();
    let mut error = Vec::new();
    let mut context = RuntimeContext {
        working_directory: &mut options.working_directory,
        script_path: None,
        cli_arguments: &[],
        environment: &mut options.environment,
        input: &mut std::io::Cursor::new(Vec::<u8>::new()),
        output: &mut output,
        error: &mut error,
        cancellation: Some(flag.clone()),
        permissions: &options.permissions,
        call_span: Default::default(),
        max_value_bytes: None,
    };
    assert_eq!(
        sleep_value(&mut context, &[Value::Int(0)]).unwrap(),
        Value::Null
    );
    for args in [
        vec![],
        vec![Value::Int(-1)],
        vec![Value::Float(0.0)],
        vec![Value::String("0".into())],
        vec![Value::Int(0), Value::Int(1)],
    ] {
        assert_eq!(
            sleep_value(&mut context, &args).unwrap_err().kind,
            "TypeError"
        );
    }
    flag.store(true, Ordering::Relaxed);
    assert_eq!(
        sleep_value(&mut context, &[Value::Int(i64::MAX)])
            .unwrap_err()
            .kind,
        "Interrupted"
    );
    flag.store(false, Ordering::Relaxed);
    let denied = scr_runtime::PermissionConfig {
        allowed: Default::default(),
    };
    context.permissions = &denied;
    assert_eq!(
        sleep_value(&mut context, &[Value::Int(0)])
            .unwrap_err()
            .kind,
        "PermissionError"
    );
}

#[test]
fn registration_reports_duplicate_builtins() {
    let mut runtime = Runtime::new();
    register(&mut runtime).unwrap();
    assert_eq!(register(&mut runtime).unwrap_err().kind, "RegistryError");
}
