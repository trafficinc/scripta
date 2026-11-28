use super::*;
use crate::{MapKey, RuntimeOptions, test_support};
use std::rc::Rc;

fn pair() -> (Runtime, Runtime) {
    let (fast, mut reference) = test_support::runtimes(RuntimeOptions::default());
    reference.disable_nested_literal_reads = true;
    (fast, reference)
}

#[test]
fn mixed_collections_typed_keys_grouping_and_snapshots_match() {
    let (mut fast, mut reference) = pair();
    test_support::compare_source(
        &mut fast,
        &mut reference,
        r#"
        $row=["meta"=>["rank"=>7,1=>11,"1"=>22,"世界"=>[3,4]]];
        $a=$row["meta"]["rank"]; $b=(($row)[("meta")])[("1")];
        $c=$row["meta"][1]; $snapshot=$row["meta"]["世界"]; $snapshot[0]=99;
        $list=[[5,6],[7]];$d=$list[0][1];
        $map=[1=>["x"=>9]];$e=$map[1]["x"];
        function make($row){return function(){return $row["meta"]["rank"];};}
        $callback=make($row);$f=$callback();
        $row["meta"]["rank"]=8;$g=$row["meta"]["rank"];
        "#,
    )
    .unwrap();
    assert_eq!(fast.nested_literal_reads_executed, 8);
    assert_eq!(reference.nested_literal_reads_executed, 0);
    for (name, value) in [
        ("a", 7),
        ("b", 22),
        ("c", 11),
        ("d", 6),
        ("e", 9),
        ("f", 7),
        ("g", 8),
    ] {
        assert_eq!(fast.get_global(name), Some(&Value::Int(value)));
    }
    test_support::compare_source(&mut fast, &mut reference, "$original=$row['meta']['世界'];")
        .unwrap();
    assert_eq!(
        fast.get_global("original"),
        Some(&Value::list(vec![Value::Int(3), Value::Int(4)]))
    );
}

#[test]
fn prefix_and_final_errors_keep_distinct_spans_stacks_and_destinations() {
    for expression in [
        "$missing['meta']['rank']",
        "(($missing))['meta']['rank']",
        "$row['absent']['rank']",
        "($row['absent'])['rank']",
        "$row['scalar']['rank']",
        "$row['meta']['absent']",
        "$row['list'][9]",
        "$row['list']['rank']",
        "$row['text'][0]",
        "$scalar[0][0]",
        "$list['wrong'][0]",
        "$list[9][0]",
    ] {
        let (mut fast, mut reference) = pair();
        let source = format!(
            "$seen=42;$row=['meta'=>['rank'=>7],'scalar'=>1,'list'=>[8],'text'=>'é'];$scalar=3;$list=[[4]];function read($row,$scalar,$list){{return {expression};}}$seen=read($row,$scalar,$list);"
        );
        assert!(
            test_support::compare_with_recovery(
                &mut fast,
                &mut reference,
                &source,
                "$recovered=1;"
            )
            .is_err(),
            "{expression}"
        );
        assert_eq!(fast.get_global("seen"), Some(&Value::Int(42)));
        assert_eq!(fast.nested_literal_reads_executed, 1);
    }
}

#[test]
fn unsupported_dynamic_deep_receiver_and_negative_forms_keep_general_execution() {
    for source in [
        "$row=['meta'=>['rank'=>7]];$key='meta';$seen=$row[$key]['rank'];",
        "$row=['meta'=>['rank'=>7]];$key='rank';$seen=$row['meta'][$key];",
        "$row=['meta'=>['rank'=>7]];function key(){return 'rank';}$seen=$row['meta'][key()];",
        "$row=['meta'=>['rank'=>7]];$seen=$row['meta'][($row=['other'=>1])];",
        "$row=['meta'=>['rank'=>[7]]];$seen=$row['meta']['rank'][0];",
        "$row=[[7]];$seen=$row[-1][0];",
        "$row=[[7]];$seen=$row[0][-1];",
        "function row(){return ['meta'=>['rank'=>7]];}$seen=row()['meta']['rank'];",
        "class C{$row=['meta'=>['rank'=>7]];function read(){return $this->row['meta']['rank'];}}$c=new C();$seen=$c->read();",
        "class C{function read(){return $this['meta']['rank'];}}$c=new C();$seen=$c->read();",
        "$row=['meta'=>['rank'=>7]];$seen=$row[false]['rank'];",
    ] {
        let (mut fast, mut reference) = pair();
        let _ =
            test_support::compare_with_recovery(&mut fast, &mut reference, source, "$recovered=1;");
        assert_eq!(fast.nested_literal_reads_executed, 0, "{source}");
    }
}

#[test]
fn first_level_map_hints_survive_replacement_reordering_and_mutation() {
    let (mut fast, mut reference) = pair();
    for round in 0..4 {
        let entries = (0..64)
            .map(|i| {
                (
                    MapKey::String(format!("key{}", (i + round) % 64).into()),
                    Value::map(vec![(MapKey::String("rank".into()), Value::Int(i))]),
                )
            })
            .collect();
        let replacement = Value::map(entries);
        for runtime in [&mut fast, &mut reference] {
            runtime
                .set_global("replacement", replacement.clone())
                .unwrap();
        }
        test_support::compare_source(&mut fast,&mut reference,"$row=$replacement;$seen=$row['key63']['rank'];$row['key63']['rank']=99;$updated=$row['key63']['rank'];$snapshot=$row;$row['key63']=['rank'=>123];$after=$row['key63']['rank'];$old=$snapshot['key63']['rank'];").unwrap();
        assert_eq!(
            fast.get_global("seen"),
            Some(&Value::Int((63 - round + 64) % 64))
        );
        assert_eq!(fast.get_global("updated"), Some(&Value::Int(99)));
        assert_eq!(fast.get_global("old"), Some(&Value::Int(99)));
        assert_eq!(fast.get_global("after"), Some(&Value::Int(123)));
    }
    assert!(fast.nested_literal_reads_executed > 0);
}

#[test]
fn exact_instruction_scope_recursion_byte_and_cancellation_boundaries_match() {
    let row = Value::map(vec![(
        MapKey::String("meta".into()),
        Value::map(vec![(MapKey::String("rank".into()), Value::Int(7))]),
    )]);
    let mut options = Vec::new();
    for limit in 0..30 {
        options.push(RuntimeOptions {
            instruction_limit: Some(limit),
            ..Default::default()
        });
    }
    for limit in 0..6 {
        options.push(RuntimeOptions {
            max_scopes: Some(limit),
            ..Default::default()
        });
    }
    for limit in 0..4 {
        options.push(RuntimeOptions {
            recursion_limit: limit,
            ..Default::default()
        });
    }
    let key_bytes = size_of::<Value>() + 4;
    let row_bytes = row.measure(0, None).unwrap();
    for bytes in [
        0,
        key_bytes - 1,
        key_bytes,
        key_bytes + 1,
        row_bytes - 1,
        row_bytes,
        row_bytes + 1,
    ] {
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
        for expression in [
            "$row['meta']['rank']",
            "$row['absent']['rank']",
            "$row['meta']['absent']",
        ] {
            let (mut fast, mut reference) = pair();
            for runtime in [&mut fast, &mut reference] {
                runtime.set_global("row", row.clone()).unwrap();
                runtime.options = option.clone();
            }
            let source =
                format!("$seen=42;function read($row){{return {expression};}}$seen=read($row);");
            let _ = test_support::compare_source(&mut fast, &mut reference, &source);
            if option.max_value_bytes.is_some() {
                assert_eq!(fast.nested_literal_reads_executed, 0);
            }
        }
    }
}

#[test]
fn final_bytes_collections_and_callable_results_preserve_owners_and_capture_lifetimes() {
    let payload: Rc<[u8]> = Rc::from(&b"payload"[..]);
    let row = Value::map(vec![(
        MapKey::String("meta".into()),
        Value::map(vec![
            (
                MapKey::String("bytes".into()),
                Value::Bytes(payload.clone()),
            ),
            (
                MapKey::String("list".into()),
                Value::list(vec![Value::Bytes(payload.clone())]),
            ),
        ]),
    )]);
    let (mut fast, mut reference) = pair();
    for runtime in [&mut fast, &mut reference] {
        runtime.set_global("row", row.clone()).unwrap();
    }
    let before = Rc::strong_count(&payload);
    test_support::compare_source(&mut fast,&mut reference,"$bytes=$row['meta']['bytes'];$list=$row['meta']['list'];function make(){$x=8;return ['meta'=>['f'=>function(){return $x;}]];}$closures=make();$f=$closures['meta']['f'];$closures=null;$seen=$f();").unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(8)));
    assert_eq!(Rc::strong_count(&payload), before + 4);
    test_support::compare_source(
        &mut fast,
        &mut reference,
        "$bytes=null;$list=null;$row=null;$f=null;",
    )
    .unwrap();
    fast.collect_garbage();
    reference.collect_garbage();
    assert_eq!(Rc::strong_count(&payload), 3);
    assert_eq!(fast.environments.live, 1);
    assert_eq!(reference.environments.live, 1);
    drop(row);
    assert_eq!(Rc::strong_count(&payload), 1);
}

#[test]
fn native_results_preserve_payload_observations_owner_counts_and_cleanup() {
    use crate::NativeObject;
    use std::cell::{Cell, RefCell};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    #[derive(Debug)]
    struct Probe {
        owner: std::rc::Weak<Probe>,
        events: Rc<RefCell<Vec<usize>>>,
        drops: Rc<Cell<usize>>,
        bytes: Cell<usize>,
        interrupt: Arc<AtomicBool>,
        trip: Cell<bool>,
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
            if self.trip.get() {
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
    for (collection, overflow, trip) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let mut observations = Vec::new();
        for general in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let drops = Rc::new(Cell::new(0));
            let interrupt = Arc::new(AtomicBool::new(false));
            let handle = Rc::new_cyclic(|owner| Probe {
                owner: owner.clone(),
                events: events.clone(),
                drops: drops.clone(),
                bytes: Cell::new(0),
                interrupt: interrupt.clone(),
                trip: Cell::new(false),
            });
            let mut runtime = Runtime::with_options(RuntimeOptions {
                interrupt_flag: Some(interrupt),
                ..Default::default()
            });
            runtime.disable_nested_literal_reads = general;
            runtime
                .set_global(
                    "row",
                    Value::map(vec![(
                        MapKey::String("meta".into()),
                        Value::map(vec![(
                            MapKey::String("handle".into()),
                            if collection {
                                Value::list(vec![Value::NativeObject(handle.clone())])
                            } else {
                                Value::NativeObject(handle.clone())
                            },
                        )]),
                    )]),
                )
                .unwrap();
            handle.bytes.set(if overflow { usize::MAX } else { 0 });
            handle.trip.set(trip);
            events.borrow_mut().clear();
            let mut output = Vec::new();
            let result = runtime.execute_source(
                "$seen=42;$seen=$row['meta']['handle'];$after=1;",
                &mut output,
            );
            assert_eq!(result.is_err(), overflow || trip);
            if overflow {
                assert_eq!(runtime.get_global("seen"), Some(&Value::Int(42)));
            }
            observations.push((
                result,
                output,
                runtime.steps,
                events.borrow().clone(),
                Rc::strong_count(&handle),
            ));
            assert_eq!(runtime.nested_literal_reads_executed, usize::from(!general));
            drop(runtime);
            assert_eq!(Rc::strong_count(&handle), 1);
            drop(handle);
            assert_eq!(drops.get(), 1);
        }
        assert_eq!(observations[0], observations[1]);
    }
}

#[test]
fn quota_fallback_preserves_literal_validation_between_prefix_lookups() {
    for source in [
        "$missing['x']['a key exceeding the byte quota'];",
        "$row['absent']['a key exceeding the byte quota'];",
        "$row['x']['a key exceeding the byte quota'];",
        "$row['a key exceeding the byte quota']['x'];",
    ] {
        let (mut fast, mut reference) = pair();
        for runtime in [&mut fast, &mut reference] {
            runtime
                .set_global(
                    "row",
                    Value::map(vec![(MapKey::String("x".into()), Value::map(vec![]))]),
                )
                .unwrap();
            runtime.options.max_value_bytes = Some(size_of::<Value>() + 6);
        }
        assert!(test_support::compare_source(&mut fast, &mut reference, source).is_err());
        assert_eq!(fast.nested_literal_reads_executed, 0);
    }
}
