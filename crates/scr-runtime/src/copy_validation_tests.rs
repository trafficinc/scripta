use super::*;
use std::cell::{Cell, RefCell};

fn pair(options: RuntimeOptions) -> (Runtime, Runtime) {
    let (fast, mut slow) = crate::test_support::runtimes(options);
    slow.disable_copy_validation = true;
    (fast, slow)
}
fn compare(fast: &mut Runtime, slow: &mut Runtime, source: &str) {
    let _ = crate::test_support::compare_source(fast, slow, source);
}
#[test]
fn only_discarded_plain_variable_copies_use_specialized_validation() {
    let (mut fast, mut slow) = pair(RuntimeOptions::default());
    let original = Value::list(vec![Value::Int(i64::MIN); 64]);
    for runtime in [&mut fast, &mut slow] {
        runtime.set_global("source", original.clone()).unwrap();
    }
    compare(
        &mut fast,
        &mut slow,
        r#"
        $copy=$source; $copy[0]=99; $snapshot=$copy;
        $retained=($copy=$source); $grouped=($source);
        $small=[1,2]; $smallcopy=$small;
        class Holder { $items=[]; } $object=new Holder();
        $object->items=$source; $field=$object->items;
        $outer=[[]]; $outer[0]=$source; $indexed=$outer[0];
    "#,
    );
    assert_eq!(fast.copy_validations_executed, 2);
    assert_eq!(slow.copy_validations_executed, 0);
    for name in ["source", "copy", "retained", "grouped", "field", "indexed"] {
        assert_eq!(fast.get_global(name), Some(&original));
        assert_eq!(fast.get_global(name), slow.get_global(name));
    }
    let Value::List(snapshot) = fast.get_global("snapshot").unwrap() else {
        panic!()
    };
    assert_eq!(snapshot[0], Value::Int(99));
}

#[test]
fn exact_copy_limits_preserve_errors_spans_and_failed_destinations() {
    let exact = 65 * size_of::<Value>();
    for limit in [
        0,
        size_of::<Value>() - 1,
        size_of::<Value>(),
        exact - 1,
        exact,
    ] {
        let (mut fast, mut slow) = pair(RuntimeOptions::default());
        for runtime in [&mut fast, &mut slow] {
            runtime
                .set_global("source", Value::list(vec![Value::Int(i64::MAX); 64]))
                .unwrap();
            runtime
                .set_global("destination", Value::String("saved".into()))
                .unwrap();
            runtime.options.max_value_bytes = Some(limit);
        }
        compare(&mut fast, &mut slow, "$destination = $source;");
        assert_eq!(fast.copy_validations_executed, 1);
        assert_eq!(
            fast.get_global("destination"),
            slow.get_global("destination")
        );
        if limit < exact {
            assert_eq!(
                fast.get_global("destination"),
                Some(&Value::String("saved".into()))
            );
        }
    }
}

#[test]
fn copied_lists_invalidate_previous_map_hints_and_keep_name_slot_fallback() {
    let (mut fast, mut slow) = pair(RuntimeOptions::default());
    for runtime in [&mut fast, &mut slow] {
        runtime
            .set_global("source", Value::list(vec![Value::Int(7); 64]))
            .unwrap();
        runtime
            .set_global(
                "destination",
                Value::Map(
                    (0..120)
                        .map(|i| (MapKey::String(format!("key{i}")), Value::Int(i)))
                        .collect(),
                ),
            )
            .unwrap();
    }
    compare(
        &mut fast,
        &mut slow,
        r#"
        $before=$destination["key119"]; $destination=$source;
        $listvalue=$destination[63]; $newname=$source;
        $destination=["key119"=>42]; $after=$destination["key119"];
    "#,
    );
    assert_eq!(fast.copy_validations_executed, 2);
    for (name, expected) in [("before", 119), ("listvalue", 7), ("after", 42)] {
        assert_eq!(fast.get_global(name), Some(&Value::Int(expected)));
    }
    assert_eq!(fast.get_global("newname"), slow.get_global("newname"));
}

#[derive(Debug)]
struct Observed {
    id: usize,
    owner: std::rc::Weak<Observed>,
    events: Rc<RefCell<Vec<(usize, usize)>>>,
    flag: Option<Arc<AtomicBool>>,
    armed: Cell<bool>,
}
impl NativeObject for Observed {
    fn type_name(&self) -> &str {
        "Observed"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("no property".into())
    }
    fn payload_bytes(&self) -> usize {
        self.events
            .borrow_mut()
            .push((self.id, self.owner.strong_count()));
        if self.armed.replace(false) {
            self.flag.as_ref().unwrap().store(true, Ordering::Relaxed);
        }
        // Reentrant sizing must not share borrowed scratch or add handle owners.
        assert_eq!(
            Value::list(vec![Value::Null; 64]).measure(0, None).unwrap(),
            65 * size_of::<Value>()
        );
        self.id
    }
}
fn observed(
    id: usize,
    events: Rc<RefCell<Vec<(usize, usize)>>>,
    flag: Option<Arc<AtomicBool>>,
) -> Rc<Observed> {
    Rc::new_cyclic(|owner| Observed {
        id,
        owner: owner.clone(),
        events,
        flag,
        armed: Cell::new(false),
    })
}
fn check_measure(prefix: Value, events: &Rc<RefCell<Vec<(usize, usize)>>>) {
    let mut items = vec![prefix];
    items.extend(vec![Value::Bool(true); 48]);
    let value = Value::list(items);
    let Value::List(items) = &value else {
        unreachable!()
    };
    for limit in [
        None,
        Some(0),
        Some(32),
        Some(48 * size_of::<Value>()),
        Some(1_000_000),
    ] {
        events.borrow_mut().clear();
        let expected = value.measure(0, limit).map(|_| ());
        let observations = std::mem::take(&mut *events.borrow_mut());
        assert_eq!(copy_validation::validate(items, limit), Some(expected));
        assert_eq!(*events.borrow(), observations);
    }
}

#[test]
fn specialized_prefix_matches_dynamic_payload_order_depth_and_error_precedence() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let a = observed(1, events.clone(), None);
    let b = observed(2, events.clone(), None);
    check_measure(
        Value::Map(vec![
            (MapKey::Int(1), Value::NativeObject(a.clone())),
            (
                MapKey::String("1".into()),
                Value::list(vec![
                    Value::NativeObject(b.clone()),
                    Value::String("é東京".into()),
                ]),
            ),
        ]),
        &events,
    );
    check_measure(
        Value::Map(vec![
            (MapKey::Int(1), Value::Float(f64::NAN)),
            (MapKey::Int(1), Value::Null),
        ]),
        &events,
    );
    check_measure(Value::Float(f64::INFINITY), &events);
    check_measure(Value::Bytes(Rc::from([0_u8, 255])), &events);
    let huge = observed(usize::MAX - size_of::<Value>(), events.clone(), None);
    check_measure(Value::NativeObject(huge), &events);
    let mut nested = Value::Null;
    for _ in 0..127 {
        nested = Value::list(vec![nested]);
    }
    check_measure(nested.clone(), &events);
    check_measure(Value::list(vec![nested]), &events);
    assert_eq!(Rc::strong_count(&a), 1);
    assert_eq!(Rc::strong_count(&b), 1);
    assert!(copy_validation::validate(&vec![Value::Float(1.0); 64], None).is_none());
    assert!(copy_validation::validate(&vec![Value::Int(1); 31], None).is_none());
}

#[test]
fn payload_cancellation_and_cleanup_match_the_ordinary_copy_path() {
    let mut states = Vec::new();
    for _ in 0..2 {
        let flag = Arc::new(AtomicBool::new(false));
        let events = Rc::new(RefCell::new(Vec::new()));
        let object = observed(1, events.clone(), Some(flag.clone()));
        let weak = Rc::downgrade(&object);
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag),
            ..Default::default()
        });
        let mut items = vec![Value::NativeObject(object.clone())];
        items.extend(vec![Value::Int(1); 63]);
        runtime.set_global("source", Value::list(items)).unwrap();
        events.borrow_mut().clear();
        object.armed.set(true);
        states.push((runtime, events, object, weak));
    }
    let (mut fast, a, object_a, weak_a) = states.remove(0);
    let (mut slow, b, object_b, weak_b) = states.remove(0);
    slow.disable_copy_validation = true;
    compare(
        &mut fast,
        &mut slow,
        "$destination=$source; echo \"unreachable\";",
    );
    assert_eq!(fast.copy_validations_executed, 1);
    assert_eq!(*a.borrow(), *b.borrow());
    assert!(fast.get_global("destination").is_some());
    assert_eq!(Rc::strong_count(&object_a), 3);
    drop(fast);
    drop(slow);
    drop(object_a);
    drop(object_b);
    assert!(weak_a.upgrade().is_none());
    assert!(weak_b.upgrade().is_none());
}
