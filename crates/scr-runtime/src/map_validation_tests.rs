use super::*;
use crate::{RuntimeOptions, copy_validation, test_support};
use std::cell::{Cell, RefCell};

fn entries(count: usize) -> Vec<(MapKey, Value)> {
    (0..count)
        .map(|i| {
            let key = if i % 2 == 0 {
                MapKey::Int((i / 2) as i64)
            } else {
                MapKey::String((i / 2).to_string().into())
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
            + entries.len() * (MapKey::LOGICAL_BYTES + size_of::<Value>())
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
                    MapKey::String(format!("{}世界{i}", "prefix".repeat(64)).into()),
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
            2 * size_of::<Value>() + MapKey::LOGICAL_BYTES + 5
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
                    MapKey::String(format!("世界{number}").into())
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

#[test]
fn cached_large_keys_reject_duplicates_after_every_mutable_access_and_preserve_aliases() {
    let edits: [fn(&mut crate::Map); 5] = [
        |m| m[1].0 = m[0].0.clone(),
        |m| {
            let entry = m[0].clone();
            m.push(entry);
        },
        |m| m.extend([(MapKey::Int(0), Value::Null)]),
        |m| {
            let first = m[0].0.clone();
            for (index, entry) in m.iter_mut().enumerate() {
                if index == 1 {
                    entry.0 = first.clone();
                }
            }
        },
        |m| {
            let raw: &mut Vec<(MapKey, Value)> = m;
            raw[1].0 = raw[0].0.clone();
        },
    ];
    for edit in edits {
        let original = Value::map(entries(64));
        let expected = original.measure(0, None).unwrap();
        let Value::Map(base) = &original else {
            unreachable!()
        };
        assert!(base.keys_validated());
        let mut copied = original.clone();
        let Value::Map(changed) = &mut copied else {
            unreachable!()
        };
        edit(changed);
        assert!(!changed.keys_validated());
        assert!(base.keys_validated());
        assert_eq!(
            compare(&copied, 0, Some(0)),
            Err("duplicate host map key".into())
        );
        assert_eq!(
            compare(&copied, 0, None),
            Err("duplicate host map key".into())
        );
        assert_eq!(original.measure(0, None), Ok(expected));
        let Value::Map(changed) = &mut copied else {
            unreachable!()
        };
        changed.clear();
        changed.extend(entries(64));
        assert_eq!(compare(&copied, 0, None), Ok(expected));
    }
    let original = Value::map(entries(64));
    original.measure(0, None).unwrap();
    let Value::Map(map) = original else {
        unreachable!()
    };
    let mut raw = map.into_vec();
    raw.push(raw[0].clone());
    assert_eq!(
        Value::map(raw).measure(0, None),
        Err("duplicate host map key".into())
    );
}

#[test]
fn cached_keys_do_not_cache_values_depth_budgets_or_dynamic_payload_observations() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = Rc::new_cyclic(|owner| Observed {
        id: 1,
        bytes: Cell::new(3),
        owner: owner.clone(),
        events: events.clone(),
    });
    let mut pairs = entries(64);
    pairs[0].1 = Value::NativeObject(observed.clone());
    pairs[63].1 = Value::Bytes(Rc::from([0_u8, 255]));
    let value = Value::map(pairs);
    let exact = value.measure(0, None).unwrap();
    let Value::Map(map) = &value else {
        unreachable!()
    };
    assert!(map.keys_validated());
    for bytes in [3, 4, usize::MAX] {
        observed.bytes.set(bytes);
        for limit in [None, Some(0), Some(exact - 1), Some(exact), Some(exact + 1)] {
            events.borrow_mut().clear();
            let expected = with_hashed_map_validation(|| value.measure(0, limit));
            let previous = std::mem::take(&mut *events.borrow_mut());
            assert_eq!(value.measure(0, limit), expected);
            assert_eq!(*events.borrow(), previous);
        }
    }
    observed.bytes.set(3);
    let copied = value.clone();
    let Value::Map(map) = &copied else {
        unreachable!()
    };
    assert!(map.keys_validated());
    assert_eq!(Rc::strong_count(&observed), 3);
    let _ = compare(&copied, 0, None).unwrap();
    assert_eq!(
        value.measure(crate::MAX_VALUE_DEPTH, None),
        Err("value nesting exceeds 128 levels".into())
    );
    drop(copied);
    assert_eq!(Rc::strong_count(&observed), 2);
    let mut nested = Value::map(entries(64));
    nested.measure(0, None).unwrap();
    let Value::Map(map) = &mut nested else {
        unreachable!()
    };
    map[0].1 = Value::Float(f64::NAN);
    assert_eq!(
        compare(&nested, 0, None),
        Err("floating-point value must be finite".into())
    );
    assert_eq!(
        compare(&nested, 0, Some(0)),
        Err("execution value byte limit exceeded".into())
    );
    let mut list = vec![nested];
    list.extend(vec![Value::Null; 32]);
    for limit in [None, Some(0), Some(4096), Some(100_000)] {
        let expected = with_hashed_map_validation(|| copy_validation::validate(&list, limit));
        assert_eq!(copy_validation::validate(&list, limit), expected);
    }
}

#[test]
fn native_duplicate_mutations_of_previously_validated_maps_keep_failed_destinations() {
    let (mut fast, mut reference) = test_support::runtimes(RuntimeOptions::default());
    for runtime in [&mut fast, &mut reference] {
        let value = Value::map(entries(64));
        value.measure(0, None).unwrap();
        runtime
            .register(
                "Host::bad_cached",
                crate::NativeFunction::new(
                    crate::NativeSignature::new(vec![], crate::NativeType::Map),
                    move |_, _| {
                        let mut result = value.clone();
                        let Value::Map(map) = &mut result else {
                            unreachable!()
                        };
                        map[1].0 = map[0].0.clone();
                        Ok(result)
                    },
                ),
            )
            .unwrap();
    }
    let first = Cell::new(true);
    let result = test_support::compare_execution(
        &mut fast,
        &mut reference,
        "mutated validated native map",
        |runtime, output| {
            let mut run = || runtime.execute_source("$seen=42;$seen=Host::bad_cached();", output);
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
}

#[test]
fn ordered_subset_proofs_are_checked_and_never_certify_output_values() {
    let source = Value::map(entries(64));
    source.measure(0, None).unwrap();
    let Value::Map(map) = &source else {
        unreachable!()
    };
    let selected: Vec<_> = map
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 3 != 0)
        .map(|(_, v)| v.clone())
        .collect();
    let result = crate::Map::from_ordered_subset(map, selected.clone());
    assert!(result.keys_validated());
    let value = Value::Map(result);
    let exact = compare(&value, 0, None).unwrap();
    for limit in [exact - 1, exact, exact + 1] {
        let _ = compare(&value, 0, Some(limit));
    }
    let mut invalid_values = selected.clone();
    invalid_values[0].1 = Value::Float(f64::NAN);
    let result = crate::Map::from_ordered_subset(map, invalid_values);
    assert!(result.keys_validated());
    assert_eq!(
        Value::Map(result).measure(0, None),
        Err("floating-point value must be finite".into())
    );
    let mut duplicate = selected.clone();
    duplicate[1].0 = duplicate[0].0.clone();
    let result = crate::Map::from_ordered_subset(map, duplicate);
    assert!(!result.keys_validated());
    assert_eq!(
        Value::Map(result).measure(0, None),
        Err("duplicate host map key".into())
    );
    let mut reordered = selected.clone();
    reordered.reverse();
    let result = crate::Map::from_ordered_subset(map, reordered);
    assert!(!result.keys_validated());
    assert!(Value::Map(result).measure(0, None).is_ok());
    let mut foreign = selected.clone();
    foreign[0].0 = MapKey::Int(999);
    let result = crate::Map::from_ordered_subset(map, foreign);
    assert!(!result.keys_validated());
    assert!(Value::Map(result).measure(0, None).is_ok());
    let unvalidated = crate::Map::from(entries(64));
    let result = crate::Map::from_ordered_subset(&unvalidated, selected);
    assert!(!result.keys_validated());
    assert!(Value::Map(result).measure(0, None).is_ok());
}
