use super::*;
use crate::{RuntimeOptions, copy_validation, test_support};
use std::cell::{Cell, RefCell};

fn entries(count: usize) -> Vec<(MapKey, Value)> {
    (0..count)
        .map(|i| {
            let key = if i % 2 == 0 {
                MapKey::Int((i / 2) as i64)
            } else {
                MapKey::String((i / 2).to_string())
            };
            (key, Value::Int(i as i64))
        })
        .collect()
}

fn compare(value: &Value, depth: usize, limit: Option<usize>) -> Result<usize, String> {
    let expected = with_hashed_map_validation(|| value.measure(depth, limit));
    let actual = value.measure(depth, limit);
    assert_eq!(actual, expected);
    actual
}

#[test]
fn typed_keys_empty_maps_and_threshold_boundaries_keep_logical_sizes() {
    for count in [
        0,
        1,
        2,
        SMALL_MAP_KEYS - 1,
        SMALL_MAP_KEYS,
        SMALL_MAP_KEYS + 1,
        64,
    ] {
        let entries = entries(count);
        let expected = size_of::<Value>()
            + entries.len() * (size_of::<MapKey>() + size_of::<Value>())
            + entries
                .iter()
                .map(|(key, _)| match key {
                    MapKey::String(text) => text.len(),
                    _ => 0,
                })
                .sum::<usize>();
        let value = Value::map(entries);
        assert_eq!(compare(&value, 0, None), Ok(expected));
        for limit in [0, expected - 1, expected, expected + 1] {
            let result = compare(&value, 0, Some(limit));
            assert_eq!(result.is_ok(), limit >= expected);
        }
    }
}

#[test]
fn duplicate_keys_precede_invalid_children_and_payload_observation() {
    for count in [
        2,
        SMALL_MAP_KEYS - 1,
        SMALL_MAP_KEYS,
        SMALL_MAP_KEYS + 1,
        64,
    ] {
        let mut pairs = entries(count);
        pairs[0].1 = Value::Float(f64::NAN);
        pairs[count - 1].0 = pairs[0].0.clone();
        let value = Value::map(pairs);
        assert_eq!(
            compare(&value, 0, None),
            Err("duplicate host map key".into())
        );
        // A root map checks each key before testing the complete node's budget.
        assert_eq!(
            compare(&value, 0, Some(0)),
            Err("duplicate host map key".into())
        );
        let mut copied = vec![value];
        copied.extend(vec![Value::Null; 32]);
        let plain = Value::list(copied.clone());
        for limit in [None, Some(0), Some(1024), Some(100_000)] {
            let expected = with_hashed_map_validation(|| copy_validation::validate(&copied, limit));
            assert_eq!(copy_validation::validate(&copied, limit), expected);
            assert_eq!(
                copy_validation::validate(&copied, limit),
                Some(plain.measure(0, limit).map(|_| ()))
            );
        }
    }
}

#[test]
fn unicode_long_keys_nested_maps_bytes_and_depth_limits_match_hashing() {
    for count in [SMALL_MAP_KEYS, SMALL_MAP_KEYS + 1] {
        let pairs = (0..count)
            .map(|i| {
                (
                    MapKey::String(format!("{}世界{i}", "prefix".repeat(64))),
                    Value::list(vec![
                        Value::Bytes(Rc::from([0_u8, 255])),
                        Value::String("é東京".into()),
                    ]),
                )
            })
            .collect();
        let value = Value::map(pairs);
        let exact = compare(&value, 0, None).unwrap();
        for limit in [exact - 1, exact, exact + 1] {
            let _ = compare(&value, 0, Some(limit));
        }
        let mut values = vec![value];
        values.extend(vec![Value::Int(1); 32]);
        let root = Value::list(values.clone());
        let exact = root.measure(0, None).unwrap();
        for limit in [None, Some(exact - 1), Some(exact), Some(exact + 1)] {
            let expected = with_hashed_map_validation(|| copy_validation::validate(&values, limit));
            assert_eq!(copy_validation::validate(&values, limit), expected);
            assert_eq!(expected, Some(root.measure(0, limit).map(|_| ())));
        }
    }
    let mut value = Value::Null;
    for _ in 0..crate::MAX_VALUE_DEPTH {
        value = Value::map(vec![(MapKey::Int(0), value)]);
    }
    assert!(compare(&value, 0, None).is_ok());
    assert_eq!(
        compare(&value, 1, None),
        Err("value nesting exceeds 128 levels".into())
    );
    let too_deep = Value::map(vec![(MapKey::Int(0), value)]);
    assert_eq!(
        compare(&too_deep, 0, None),
        Err("value nesting exceeds 128 levels".into())
    );
}

#[derive(Debug)]
struct Observed {
    id: usize,
    bytes: Cell<usize>,
    owner: std::rc::Weak<Observed>,
    events: Rc<RefCell<Vec<(usize, usize)>>>,
}
impl NativeObject for Observed {
    fn type_name(&self) -> &str {
        "Observed"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no property".into())
    }
    fn payload_bytes(&self) -> usize {
        self.events
            .borrow_mut()
            .push((self.id, self.owner.strong_count()));
        let inner = Value::map(vec![(MapKey::String("inner".into()), Value::Int(1))]);
        assert_eq!(
            inner.measure(0, None).unwrap(),
            2 * size_of::<Value>() + size_of::<MapKey>() + 5
        );
        self.bytes.get()
    }
}

#[test]
fn dynamic_payload_order_reentrancy_ownership_and_overflow_are_unchanged() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = |id| {
        Rc::new_cyclic(|owner| Observed {
            id,
            bytes: Cell::new(id),
            owner: owner.clone(),
            events: events.clone(),
        })
    };
    let a = observed(1);
    let b = observed(2);
    let value = Value::map(vec![
        (MapKey::Int(1), Value::NativeObject(a.clone())),
        (
            MapKey::String("1".into()),
            Value::list(vec![Value::NativeObject(b.clone())]),
        ),
    ]);
    for overflow in [false, true] {
        b.bytes.set(if overflow { usize::MAX } else { 2 });
        for limit in [None, Some(0), Some(128), Some(100_000)] {
            events.borrow_mut().clear();
            let expected = with_hashed_map_validation(|| value.measure(0, limit));
            let observations = std::mem::take(&mut *events.borrow_mut());
            assert_eq!(value.measure(0, limit), expected);
            assert_eq!(*events.borrow(), observations);
            if !overflow && limit.is_none() {
                assert_eq!(observations, vec![(2, 2), (1, 2)]);
            }
            if overflow && limit.is_none() {
                assert_eq!(expected, Err("value size overflow".into()));
                assert_eq!(observations, vec![(2, 2)]);
            }
        }
    }
    assert_eq!(Rc::strong_count(&a), 2);
    assert_eq!(Rc::strong_count(&b), 2);
    let duplicate = Value::map(vec![
        (MapKey::Int(0), Value::NativeObject(a.clone())),
        (MapKey::Int(0), Value::NativeObject(b.clone())),
    ]);
    events.borrow_mut().clear();
    assert!(compare(&duplicate, 0, None).is_err());
    assert!(events.borrow().is_empty());
    // Repeated measurement must observe a changed host payload, without caching it.
    b.bytes.set(2);
    let exact = value.measure(0, None).unwrap();
    b.bytes.set(3);
    assert_eq!(value.measure(0, None), Ok(exact + 1));
    drop(duplicate);
    drop(value);
    assert_eq!(Rc::strong_count(&a), 1);
    assert_eq!(Rc::strong_count(&b), 1);
}

#[test]
fn native_failures_preserve_diagnostics_and_destinations() {
    let (mut fast, mut reference) = test_support::runtimes(RuntimeOptions::default());
    for runtime in [&mut fast, &mut reference] {
        runtime
            .register(
                "Host::bad",
                crate::NativeFunction::new(
                    crate::NativeSignature::new(vec![], crate::NativeType::Map),
                    |_, _| {
                        Ok(Value::map(vec![
                            (MapKey::Int(0), Value::Float(f64::NAN)),
                            (MapKey::Int(0), Value::Null),
                        ]))
                    },
                ),
            )
            .unwrap();
    }
    let first = Cell::new(true);
    let result = test_support::compare_execution(
        &mut fast,
        &mut reference,
        "invalid native map",
        |runtime, output| {
            let mut run = || runtime.execute_source("$seen=42;$seen=Host::bad();", output);
            if first.replace(false) {
                run()
            } else {
                with_hashed_map_validation(run)
            }
        },
    );
    assert!(
        result
            .unwrap_err()
            .message
            .contains("duplicate host map key")
    );
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(42)));
    for runtime in [&mut fast, &mut reference] {
        let bad = Value::map(vec![
            (MapKey::Int(0), Value::Null),
            (MapKey::Int(0), Value::Null),
        ]);
        assert!(runtime.set_global("seen", bad).is_err());
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(42)));
    }
}

#[test]
fn generated_maps_match_hashed_validation_in_both_walkers() {
    let mut random = 137_u64;
    for case in 0..160 {
        let count = case % 40;
        let pairs = (0..count)
            .map(|_| {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let number = (random >> 32) % 32;
                let key = if random & 1 == 0 {
                    MapKey::Int(number as i64)
                } else {
                    MapKey::String(format!("世界{number}"))
                };
                let value = if random % 31 == 0 {
                    Value::Float(f64::INFINITY)
                } else {
                    Value::Int(number as i64)
                };
                (key, value)
            })
            .collect();
        let value = Value::map(pairs);
        for limit in [None, Some(0), Some(32), Some(512), Some(100_000)] {
            let _ = compare(&value, 0, limit);
            let mut values = vec![value.clone()];
            values.extend(vec![Value::Bool(true); 32]);
            let expected = with_hashed_map_validation(|| copy_validation::validate(&values, limit));
            assert_eq!(copy_validation::validate(&values, limit), expected);
        }
    }
}
