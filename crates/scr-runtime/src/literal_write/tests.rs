use super::*;
use crate::{MapKey, RuntimeOptions, test_support};

fn pair(options: RuntimeOptions) -> (Runtime, Runtime) {
    let (fast, mut reference) = test_support::runtimes(options);
    reference.disable_literal_writes = true;
    (fast, reference)
}

#[test]
fn typed_keys_grouping_list_boundaries_compounds_and_retained_snapshots_match() {
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    test_support::compare_source(
        &mut fast,
        &mut reference,
        r#"
        $row=['rank'=>7,1=>11,'1'=>22,'世界'=>[3]]; $snapshot=$row;
        $row['rank']=9; $row[('1')]+=2; $row[1]*=2;
        $result=($row['世界']=[4]); $result[0]=5; $row['new']=null;
        $items=[1,2];$copy=$items;$items[0]=3;$items[2]=4;$items[-1]+=2;
        $text=['s'=>'a'];$text['s'].='b';$text['s']=$text['s'].'c';
        $math=[0=>12];$math[0]-=2;$math[0]/=2;
    "#,
    )
    .unwrap();
    assert!(fast.literal_writes_executed >= 10);
    assert_eq!(reference.literal_writes_executed, 0);
    let Value::Map(row) = fast.get_global("row").unwrap() else {
        panic!()
    };
    assert_eq!(row[0].1, Value::Int(9));
    assert_eq!(row[1].1, Value::Int(22));
    assert_eq!(row[2].1, Value::Int(24));
    assert_eq!(row[3].1, Value::list(vec![Value::Int(4)]));
    assert_eq!(
        fast.get_global("items"),
        Some(&Value::list(vec![
            Value::Int(3),
            Value::Int(2),
            Value::Int(6)
        ]))
    );
}

#[test]
fn failure_order_diagnostics_stacks_and_destinations_match() {
    for source in [
        "$side=0;$missing['rank']=($side=1);",
        "$side=0;($missing)['rank']=($side=1);",
        "$side=0;$row=7;$row['rank']=($side=1);",
        "$side=0;$row=[];$row['rank']=($side=1);",
        "$side=0;$row=[];$row[1]=($side=1);",
        "$side=0;$row=[0];$row[2]+=($side=1);",
        "$side=0;$row=['x'=>7];$row['rank']+=($side=1);",
        "$side=0;$row=['rank'=>7];$row['rank']/=($side=0);",
        "$row=['rank'=>7];function fail($row){$row['rank']/=0;}fail($row);",
        "$row=['rank'=>7];$seen=42;$seen=($row['rank']=1/0);",
        "class C{function f(){$this['rank']=1;}}$c=new C();$c->f();",
    ] {
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        assert!(
            test_support::compare_with_recovery(&mut fast, &mut reference, source, "$recovered=1;")
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn rhs_rebinding_binding_promotion_captures_and_old_compound_values_match() {
    for (body, expected) in [
        (
            "$row=['rank'=>7];$change=function(){$row=['rank'=>20];return 2;};$row['rank']+=$change();return $row;",
            Value::map(vec![(MapKey::String("rank".into()), Value::Int(9))]),
        ),
        (
            "$row=['rank'=>7];$change=function(){$row=['other'=>20];return 2;};$row['rank']+=$change();return $row;",
            Value::map(vec![
                (MapKey::String("other".into()), Value::Int(20)),
                (MapKey::String("rank".into()), Value::Int(9)),
            ]),
        ),
        (
            "$row=['rank'=>7];$change=function(){$row=[0,1];return 9;};try{$row['rank']=$change();}catch(TypeError $e){}return $row;",
            Value::list(vec![Value::Int(0), Value::Int(1)]),
        ),
        (
            "$items=[1,2];$change=function(){$items=[7];return 9;};$items[1]=$change();return $items;",
            Value::list(vec![Value::Int(7), Value::Int(9)]),
        ),
    ] {
        let source = format!("function run(){{{body}}}$seen=run();");
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        test_support::compare_source(&mut fast, &mut reference, &source).unwrap();
        assert_eq!(fast.get_global("seen"), Some(&expected));
    }
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    test_support::compare_source(&mut fast,&mut reference,"function make(){ $row=['rank'=>7];return function(){ $row['rank']+=1;return $row;};}$f=make();$a=$f();$b=$f();").unwrap();
    assert_eq!(
        fast.get_global("a"),
        Some(&Value::map(vec![(
            MapKey::String("rank".into()),
            Value::Int(8)
        )]))
    );
}

#[test]
fn large_map_hints_track_replacements_and_new_keys() {
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    for runtime in [&mut fast, &mut reference] {
        let entries = (0..96)
            .map(|i| (MapKey::String(format!("key{i}").into()), Value::Int(i)))
            .collect();
        runtime.set_global("row", Value::map(entries)).unwrap();
        let reordered = (0..96)
            .rev()
            .map(|i| (MapKey::String(format!("key{i}").into()), Value::Int(i * 10)))
            .collect();
        runtime
            .set_global("replacement", Value::map(reordered))
            .unwrap();
    }
    test_support::compare_source(&mut fast,&mut reference,"function run($row,$replacement){$warm=$row['key90'];$change=function(){$row=$replacement;return 1;};$row['key90']+=$change();$row['new']=3;$row['key0']=4;return $row['key90'];}$seen=run($row,$replacement);").unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(91)));
}

#[test]
fn unsupported_targets_and_byte_quotas_keep_general_handling() {
    for source in [
        "$row=['rank'=>7];$key='rank';$row[$key]=9;",
        "$row=[['rank'=>7]];$row[0]['rank']=9;",
        "$row=[1];$row[]=2;",
        "$row=['rank'=>7];$row[false]=9;",
        "class C{$row=['rank'=>7];}$c=new C();$c->row['rank']=9;",
    ] {
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        let result =
            test_support::compare_with_recovery(&mut fast, &mut reference, source, "$recovered=1;");
        assert_eq!(result.is_ok(), !source.contains("[false]"), "{source}");
        assert_eq!(fast.literal_writes_executed, 0, "{source}");
    }
    let exact = 2 * size_of::<Value>() + MapKey::LOGICAL_BYTES + 4;
    for limit in [
        size_of::<Value>() - 1,
        size_of::<Value>(),
        exact - 1,
        exact,
        exact + 1,
        exact + 32,
    ] {
        let (mut fast, mut reference) = pair(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        let _ = test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            "$row=['rank'=>7];$row['rank']=9;$row['new']=1;",
            "$recovered=1;",
        );
        assert_eq!(fast.literal_writes_executed, 0);
    }
}

#[test]
fn exact_instruction_scope_limits_and_depth_failures_match() {
    let source = "$row=['rank'=>7];$f=function(){$row['rank']+=1;return $row;};$a=$f();$seen=($row['rank']=$a['rank']);";
    for limit in 0..35 {
        let (mut fast, mut reference) = pair(RuntimeOptions {
            instruction_limit: Some(limit),
            ..Default::default()
        });
        let _ = test_support::compare_source(&mut fast, &mut reference, source);
    }
    for limit in 1..6 {
        let (mut fast, mut reference) = pair(RuntimeOptions {
            max_scopes: Some(limit),
            ..Default::default()
        });
        let _ = test_support::compare_source(&mut fast, &mut reference, source);
    }
    for depth in [127, 128] {
        let mut value = Value::Int(7);
        for _ in 0..depth {
            value = Value::list(vec![value]);
        }
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        for runtime in [&mut fast, &mut reference] {
            runtime.set_global("deep", value.clone()).unwrap();
            runtime.set_global("row", Value::map(vec![])).unwrap();
        }
        let outcome = test_support::compare_with_recovery(
            &mut fast,
            &mut reference,
            "$row['value']=$deep;",
            "$recovered=1;",
        );
        assert_eq!(outcome.is_ok(), depth == 127);
    }
}

#[test]
fn reentrant_binding_promotion_and_new_byte_limits_do_not_repeat_rhs() {
    use crate::{NativeFunction, NativeSignature, NativeType};
    use std::{cell::Cell, rc::Rc};
    for limit in [None, Some(91), Some(92), Some(93)] {
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        let mut counts = Vec::new();
        for runtime in [&mut fast, &mut reference] {
            let count = Rc::new(Cell::new(0));
            counts.push(count.clone());
            runtime
                .register(
                    "Host::promote",
                    NativeFunction::reentrant(
                        NativeSignature::new(vec![], NativeType::Int),
                        move |context, _| {
                            count.set(count.get() + 1);
                            let scope = context
                                .runtime
                                .environments
                                .resolve_variable("row")
                                .unwrap()
                                .0;
                            for index in 0..12 {
                                context
                                    .runtime
                                    .environments
                                    .scope_mut(scope)
                                    .variables
                                    .insert(format!("extra{index}"), Value::Int(index));
                            }
                            context.runtime.options.max_value_bytes = limit;
                            Ok(Value::Int(9))
                        },
                    ),
                )
                .unwrap();
        }
        let source = "function run(){$row=['rank'=>7];$row['rank']=Host::promote();return $row;}$seen=run();";
        let result =
            test_support::compare_with_recovery(&mut fast, &mut reference, source, "$recovered=1;");
        let exact = 2 * size_of::<Value>() + MapKey::LOGICAL_BYTES + 4;
        assert_eq!(result.is_ok(), limit.is_none_or(|n| n >= exact));
        assert_eq!(counts[0].get(), 1);
        assert_eq!(counts[1].get(), 1);
    }
}

#[test]
fn native_payload_observations_owners_overflow_cancellation_and_cleanup_match() {
    use crate::{NativeFunction, NativeObject, NativeSignature, NativeType};
    use std::{
        cell::{Cell, RefCell},
        rc::{Rc, Weak},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    #[derive(Debug)]
    struct Probe {
        owner: Weak<Probe>,
        events: Rc<RefCell<Vec<usize>>>,
        drops: Rc<Cell<usize>>,
        bytes: Cell<usize>,
        interrupt: Arc<AtomicBool>,
        trip: bool,
    }
    impl NativeObject for Probe {
        fn type_name(&self) -> &str {
            "Probe"
        }
        fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
            Err("no property".into())
        }
        fn payload_bytes(&self) -> usize {
            self.events.borrow_mut().push(self.owner.strong_count());
            if self.trip {
                self.interrupt.store(true, Ordering::Relaxed);
            }
            self.bytes.get()
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    for (source, overflow, trip) in [
        (
            "$row=['keep'=>$handle];$snapshot=$row;$seen=($row['rank']=$handle);",
            false,
            false,
        ),
        ("$row=['rank'=>7];$row['rank']=Host::huge();", true, false),
        (
            "$row=['rank'=>7];$row['rank']=$handle;$after=1;",
            false,
            true,
        ),
    ] {
        let mut observations = Vec::new();
        for general in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let drops = Rc::new(Cell::new(0));
            let flag = Arc::new(AtomicBool::new(false));
            let probe = Rc::new_cyclic(|owner| Probe {
                owner: owner.clone(),
                events: events.clone(),
                drops: drops.clone(),
                bytes: Cell::new(0),
                interrupt: flag.clone(),
                trip: false,
            });
            // The trip object is created separately so host setup cannot cancel.
            let handle = if trip {
                Rc::new_cyclic(|owner| Probe {
                    owner: owner.clone(),
                    events: events.clone(),
                    drops: drops.clone(),
                    bytes: Cell::new(0),
                    interrupt: flag.clone(),
                    trip,
                })
            } else {
                probe.clone()
            };
            let mut runtime = Runtime::with_options(RuntimeOptions {
                interrupt_flag: Some(flag.clone()),
                ..Default::default()
            });
            runtime.disable_literal_writes = general;
            runtime
                .set_global("handle", Value::NativeObject(handle.clone()))
                .unwrap();
            flag.store(false, Ordering::Relaxed);
            let held = handle.clone();
            runtime
                .register(
                    "Host::huge",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Any),
                        move |_, _| {
                            held.bytes.set(usize::MAX);
                            Ok(Value::NativeObject(held.clone()))
                        },
                    ),
                )
                .unwrap();
            events.borrow_mut().clear();
            let mut output = Vec::new();
            let result = runtime.execute_source(source, &mut output);
            assert_eq!(result.is_err(), overflow || trip);
            if overflow {
                assert_eq!(
                    runtime.get_global("row"),
                    Some(&Value::map(vec![(
                        MapKey::String("rank".into()),
                        Value::Int(7)
                    )]))
                );
            }
            if trip {
                let Value::Map(row) = runtime.get_global("row").unwrap() else {
                    panic!()
                };
                assert!(matches!(row[0].1, Value::NativeObject(_)));
                assert!(runtime.get_global("after").is_none());
            }
            observations.push((
                result,
                output,
                events.borrow().clone(),
                handle.owner.strong_count(),
                runtime.steps,
            ));
            drop(runtime);
            drop(handle);
            drop(probe);
            assert_eq!(drops.get(), if trip { 2 } else { 1 });
        }
        assert_eq!(observations[0], observations[1], "{source}");
    }
}
