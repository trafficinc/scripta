use super::*;
use scr_runtime::{NativeObject, RuntimeError, RuntimeOptions};
use std::{
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
};

fn execute(
    input: Value,
    options: RuntimeOptions,
    prepared: bool,
) -> std::result::Result<u8, RuntimeError> {
    let mut runtime = Runtime::with_options(options);
    runtime
        .register(
            "Test::root",
            NativeFunction::reentrant(
                NativeSignature::new(vec![], NativeType::Any),
                move |context, _| {
                    if prepared {
                        let Value::List(items) = &input else {
                            unreachable!()
                        };
                        let _arguments = prepare_sort_arguments(context, items, true)?;
                    } else {
                        measure(context, &input, 0)?;
                    }
                    Ok(Value::Int(1))
                },
            ),
        )
        .unwrap();
    runtime.execute_source("$result=42;$result=Test::root();", &mut Vec::new())
}

#[test]
fn argument_preparation_preserves_root_measurement_checkpoints_limits_and_errors() {
    let mut deep = Value::Int(1);
    for _ in 0..129 {
        deep = Value::List(vec![deep]);
    }
    let inputs = [
        Value::List(vec![]),
        Value::List(vec![Value::Int(1), Value::Int(2)]),
        Value::List(vec![Value::Map(vec![(
            MapKey::String("rank".into()),
            Value::String("世界".into()),
        )])]),
        Value::List(vec![deep]),
    ];
    for input in inputs {
        let mut options = vec![RuntimeOptions::default()];
        for limit in 0..30 {
            options.push(RuntimeOptions {
                instruction_limit: Some(limit),
                ..Default::default()
            });
        }
        for limit in [0, 1, size_of::<Value>() - 1, size_of::<Value>(), 200, 1024] {
            options.push(RuntimeOptions {
                max_value_bytes: Some(limit),
                ..Default::default()
            });
        }
        for option in options {
            assert_eq!(
                execute(input.clone(), option.clone(), true),
                execute(input.clone(), option, false)
            );
        }
    }
}

#[derive(Debug)]
struct Cancel(Arc<AtomicBool>);
impl NativeObject for Cancel {
    fn type_name(&self) -> &str {
        "Cancel"
    }
    fn payload_bytes(&self) -> usize {
        self.0.store(true, AtomicOrdering::Relaxed);
        0
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("no property".into())
    }
}
#[test]
fn preparation_keeps_cancellation_after_native_payload_observation() {
    let mut errors = Vec::new();
    for prepared in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let input = Value::List(vec![Value::Map(vec![(
            MapKey::String("object".into()),
            Value::NativeObject(Rc::new(Cancel(flag.clone()))),
        )])]);
        errors.push(
            execute(
                input,
                RuntimeOptions {
                    interrupt_flag: Some(flag),
                    ..Default::default()
                },
                prepared,
            )
            .unwrap_err(),
        );
    }
    assert_eq!(errors[0], errors[1]);
    assert!(errors[0].message.contains("interrupted"));
}
