use crate::{MapKey, MapKeyText, NativeObject, Runtime, RuntimeOptions, Value};
use std::{cell::Cell, collections::HashMap, rc::Rc};

#[test]
fn shared_text_preserves_content_hashing_debug_and_owned_string_independence() {
    let text = MapKeyText::from("é世界");
    let copied = text.clone();
    assert_eq!(text.as_ptr(), copied.as_ptr());
    assert_eq!(format!("{text:?}"), "\"é世界\"");
    assert_eq!(format!("{text}"), "é世界");
    let mut index = HashMap::new();
    index.insert(text.clone(), 7);
    assert_eq!(index.get("é世界"), Some(&7));
    assert_eq!(text, "é世界".to_owned());
    let mut owned = copied.into_string();
    owned.push('!');
    assert_eq!(text.as_str(), "é世界");
    assert_eq!(owned, "é世界!");
    assert_ne!(MapKey::Int(1), MapKey::String("1".into()));
}

#[test]
fn logical_key_slot_preserves_the_owned_string_layout() {
    // Freeze the old representation independently of the new charge constant.
    enum OriginalKey {
        Int(i64),
        String(String),
    }
    let originals = [OriginalKey::Int(1), OriginalKey::String("é".into())];
    for value in originals {
        match value {
            OriginalKey::Int(value) => assert_eq!(value, 1),
            OriginalKey::String(value) => assert_eq!(value, "é"),
        }
    }
    assert_eq!(MapKey::LOGICAL_BYTES, size_of::<OriginalKey>());
    let value = Value::map(vec![
        (MapKey::Int(1), Value::Int(3)),
        (MapKey::String("é".into()), Value::Int(4)),
    ]);
    let exact = 3 * size_of::<Value>() + 2 * size_of::<OriginalKey>() + "é".len();
    assert_eq!(value.measure(0, None).unwrap(), exact);
    for limit in [exact - 1, exact, exact + 1] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        runtime.set_global("destination", Value::Int(42)).unwrap();
        assert_eq!(
            runtime.set_global("destination", value.clone()).is_ok(),
            limit >= exact
        );
        if limit < exact {
            assert_eq!(runtime.get_global("destination"), Some(&Value::Int(42)));
        }
    }
}

#[derive(Debug)]
struct Object {
    drops: Rc<Cell<usize>>,
    bytes: Cell<usize>,
}
impl NativeObject for Object {
    fn type_name(&self) -> &str {
        "Object"
    }
    fn payload_bytes(&self) -> usize {
        self.bytes.get()
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no property".into())
    }
}
impl Drop for Object {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn opaque_map_copies_share_only_key_text_and_keep_fresh_payload_checks() {
    let drops = Rc::new(Cell::new(0));
    let object = Rc::new(Object {
        drops: drops.clone(),
        bytes: Cell::new(0),
    });
    let weak = Rc::downgrade(&object);
    let original = Value::map(vec![(
        MapKey::String("outer".into()),
        Value::list(vec![Value::map(vec![(
            MapKey::String("inner".into()),
            Value::NativeObject(object.clone()),
        )])]),
    )]);
    let mut copy = original.clone();
    assert_eq!(weak.strong_count(), 3);
    let (Value::Map(before), Value::Map(after)) = (&original, &mut copy) else {
        unreachable!()
    };
    assert_ne!(before.as_ptr(), after.as_ptr());
    let (MapKey::String(before_key), MapKey::String(after_key)) = (&before[0].0, &after[0].0)
    else {
        unreachable!()
    };
    assert_eq!(before_key.as_ptr(), after_key.as_ptr());
    after[0].0 = MapKey::String("changed".into());
    let Value::List(nested) = &mut after[0].1 else {
        unreachable!()
    };
    let Value::Map(inner) = &mut nested[0] else {
        unreachable!()
    };
    inner[0].0 = MapKey::String("renamed".into());
    assert_eq!(before[0].0, MapKey::String("outer".into()));
    let initial = original.measure(0, None).unwrap();
    object.bytes.set(100);
    assert_eq!(original.measure(0, None).unwrap(), initial + 100);
    assert!(original.measure(0, Some(initial)).is_err());
    assert_eq!(weak.strong_count(), 3);
    drop(copy);
    assert_eq!(weak.strong_count(), 2);
    drop(object);
    assert_eq!(drops.get(), 0);
    drop(original);
    assert_eq!(drops.get(), 1);
    assert!(weak.upgrade().is_none());
}

#[test]
fn literal_keys_keep_validation_spans_effect_order_and_failed_destinations() {
    use crate::{NativeFunction, NativeSignature, NativeType};
    let key = "é".repeat(40);
    let source = format!("$destination=[('{key}')=>observe()];");
    let literal_start = source.find('\'').unwrap();
    let literal_bytes = size_of::<Value>() + key.len();
    let map_bytes = 2 * size_of::<Value>() + MapKey::LOGICAL_BYTES + key.len();
    for (limit, calls_expected, succeeds) in [
        (literal_bytes - 1, 0, false),
        (literal_bytes, 1, false),
        (map_bytes - 1, 1, false),
        (map_bytes, 1, true),
        (map_bytes + 1, 1, true),
    ] {
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        runtime
            .register(
                "observe",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Int),
                    move |_, _| {
                        observed.set(observed.get() + 1);
                        Ok(Value::Int(7))
                    },
                ),
            )
            .unwrap();
        runtime.set_global("destination", Value::Int(42)).unwrap();
        let result = runtime.execute_source(&source, &mut Vec::new());
        assert_eq!(result.is_ok(), succeeds);
        assert_eq!(calls.get(), calls_expected);
        if let Err(error) = result {
            assert_eq!(runtime.get_global("destination"), Some(&Value::Int(42)));
            assert!(error.message.contains("value byte limit"));
            if calls_expected == 0 {
                assert_eq!(error.span.start, literal_start);
                assert_eq!(error.span.end, literal_start + key.len() + 2);
            }
        } else {
            assert_eq!(
                runtime.get_global("destination"),
                Some(&Value::map(vec![(
                    MapKey::String(key.clone().into()),
                    Value::Int(7)
                ),]))
            );
        }
    }
}

#[test]
fn variable_keys_freeze_before_value_effects_without_early_budget_validation() {
    use crate::{NativeFunction, NativeSignature, NativeType};
    let mut runtime = Runtime::new();
    runtime.execute_source(
        "function make(){ $key='first'; $change=function(){ $key='second'; return 1; }; $map=[($key)=>$change(),$key=>2]; return [$map,$key]; } $result=make();",
        &mut Vec::new(),
    ).unwrap();
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::list(vec![
            Value::map(vec![
                (MapKey::String("first".into()), Value::Int(1)),
                (MapKey::String("second".into()), Value::Int(2)),
            ]),
            Value::String("second".into()),
        ]))
    );

    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let mut runtime = Runtime::new();
    let key = "é".repeat(40);
    runtime
        .set_global("key", Value::String(key.clone()))
        .unwrap();
    runtime.set_global("destination", Value::Int(42)).unwrap();
    runtime
        .register(
            "observe",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Int),
                move |_, _| {
                    observed.set(observed.get() + 1);
                    Ok(Value::Int(7))
                },
            ),
        )
        .unwrap();
    // A retained variable can outlive a host option change. Unlike a literal,
    // its ordinary read does not validate before the value expression runs.
    runtime.options.max_value_bytes = Some(size_of::<Value>() + key.len() - 1);
    let source = "$destination=[($key)=>observe()];";
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert_eq!(calls.get(), 1);
    assert!(error.message.contains("value byte limit"));
    assert_eq!(error.span.start, source.find('[').unwrap());
    assert_eq!(runtime.get_global("destination"), Some(&Value::Int(42)));
}

#[test]
fn borrowed_membership_keys_keep_types_errors_and_checkpoints() {
    let mut runtime = Runtime::new();
    runtime.execute_source(
        "$map=[1=>'int','1'=>'text']; $seen=[(1 in $map),('1' in $map),('missing' in $map)]; $bad=0; try { 1.0 in $map; } catch(TypeError $e) { $bad=1; }",
        &mut Vec::new(),
    ).unwrap();
    assert_eq!(
        runtime.get_global("seen"),
        Some(&Value::list(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(false),
        ]))
    );
    assert_eq!(runtime.get_global("bad"), Some(&Value::Int(1)));
    let mut limited = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(1),
        ..Default::default()
    });
    limited
        .set_global(
            "map",
            Value::map(vec![(MapKey::String("first".into()), Value::Int(1))]),
        )
        .unwrap();
    limited.set_global("destination", Value::Int(42)).unwrap();
    let source = "$destination='missing' in $map;";
    let error = limited.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.contains("instruction limit"));
    assert_eq!(
        &source[error.span.start..error.span.end],
        "'missing' in $map"
    );
    assert_eq!(limited.get_global("destination"), Some(&Value::Int(42)));
}
