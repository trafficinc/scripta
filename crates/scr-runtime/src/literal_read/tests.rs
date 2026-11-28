use super::*;
use crate::{MapKey, RuntimeOptions, test_support};
use std::rc::Rc;

fn pair(options: RuntimeOptions) -> (Runtime, Runtime) {
    let (fast, mut reference) = test_support::runtimes(options);
    reference.disable_literal_reads = true;
    (fast, reference)
}

#[test]
fn literal_reads_preserve_typed_keys_grouping_and_collection_snapshots() {
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    test_support::compare_source(
        &mut fast,
        &mut reference,
        r#"
        $row=["rank"=>7,1=>11,"1"=>22,"世界"=>[3,4]];
        $a=$row["rank"]; $b=($row)[("1")]; $c=$row[1];
        $snapshot=$row["世界"]; $snapshot[0]=99;
        $list=[5,6]; $d=$list[0]; $e=$list[-1];
        $nested=[["rank"=>8]]; $f=$nested[0]["rank"];
        function make($row){return function(){return $row["rank"];};}
        $callback=make($row); $g=$callback();
        "#,
    )
    .unwrap();
    for (name, value) in [
        ("a", 7),
        ("b", 22),
        ("c", 11),
        ("d", 5),
        ("e", 6),
        ("f", 8),
        ("g", 7),
    ] {
        assert_eq!(fast.get_global(name), Some(&Value::Int(value)));
    }
    assert_eq!(fast.literal_reads_executed, 6);
    assert_eq!(reference.literal_reads_executed, 0);
}

#[test]
fn errors_keep_source_spans_stacks_and_failed_destinations() {
    for (value, expression) in [
        (Value::map(vec![]), r#"$row["absent"]"#),
        (Value::map(vec![]), r#"($row)[("absent")]"#),
        (Value::list(vec![Value::Int(1)]), "$row[1]"),
        (Value::list(vec![Value::Int(1)]), r#"$row["rank"]"#),
        (Value::Int(1), r#"$row["rank"]"#),
        (Value::Int(1), r#"$missing["rank"]"#),
        (Value::Int(1), r#"($missing)["rank"]"#),
    ] {
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        for runtime in [&mut fast, &mut reference] {
            runtime.set_global("row", value.clone()).unwrap();
        }
        let source =
            format!("$seen=42;function read($row){{return {expression};}}$seen=read($row);");
        assert!(
            test_support::compare_with_recovery(
                &mut fast,
                &mut reference,
                &source,
                "$recovered=1;",
            )
            .is_err()
        );
        assert_eq!(fast.get_global("seen"), Some(&Value::Int(42)));
        assert!(fast.literal_reads_executed > 0);
    }
}

#[test]
fn unsupported_keys_effects_and_receivers_keep_general_evaluation() {
    for source in [
        r#"$row=["rank"=>7];$key="rank";$seen=$row[$key];"#,
        r#"$row=["rank"=>7];$key="rank";$seen=$row[($key="rank")];"#,
        r#"$row=["rank"=>7];function key(){return "rank";}$seen=$row[key()];"#,
        r#"$row=["rank"=>7];try{$seen=$row[false];}catch(TypeError $e){$seen=9;}"#,
        r#"class C{function read(){return $this["rank"];}}$c=new C();$seen=$c->read();"#,
    ] {
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        let _ =
            test_support::compare_with_recovery(&mut fast, &mut reference, source, "$recovered=1;");
        assert_eq!(fast.literal_reads_executed, 0, "{source}");
    }
}

#[test]
fn large_map_hints_verify_positions_after_writes_and_replacement() {
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    for round in 0..3 {
        let entries: Vec<_> = (0..64)
            .map(|i| {
                (
                    MapKey::String(format!("key{}", (i + round) % 64).into()),
                    Value::Int(i),
                )
            })
            .collect();
        for runtime in [&mut fast, &mut reference] {
            runtime
                .set_global("replacement", Value::map(entries.clone()))
                .unwrap();
        }
        test_support::compare_source(
            &mut fast,
            &mut reference,
            r#"$row=$replacement;$seen=$row["key63"];$row["key63"]=99;$updated=$row["key63"];"#,
        )
        .unwrap();
        assert_eq!(
            fast.get_global("seen"),
            Some(&Value::Int((63 - round + 64) % 64))
        );
        assert_eq!(fast.get_global("updated"), Some(&Value::Int(99)));
    }
}

#[test]
fn quotas_and_interruption_preserve_checkpoints_and_key_validation_order() {
    let source = r#"$seen=42;function read($row){return $row["rank"];}$seen=read($row);"#;
    let row = Value::map(vec![(MapKey::String("rank".into()), Value::Int(7))]);
    let mut options = Vec::new();
    for limit in 0..25 {
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
    let key_bytes = size_of::<Value>() + "rank".len();
    for bytes in [
        0,
        key_bytes - 1,
        key_bytes,
        key_bytes + 1,
        row.measure(0, None).unwrap(),
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
        let quota = option.max_value_bytes.is_some();
        let (mut fast, mut reference) = pair(RuntimeOptions::default());
        for runtime in [&mut fast, &mut reference] {
            runtime.set_global("row", row.clone()).unwrap();
            runtime.options = option.clone();
        }
        let _ = test_support::compare_source(&mut fast, &mut reference, source);
        if quota {
            assert_eq!(fast.literal_reads_executed, 0);
        }
    }
    // Key validation must fail before reporting a missing-map-key error.
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    for runtime in [&mut fast, &mut reference] {
        runtime.set_global("row", Value::map(vec![])).unwrap();
        runtime.options.max_value_bytes = Some(key_bytes - 1);
    }
    assert!(test_support::compare_source(&mut fast, &mut reference, r#"$row["rank"];"#).is_err());
}

#[test]
fn selected_collections_keep_handle_owners_and_release_them_after_removal() {
    let payload: Rc<[u8]> = Rc::from(&b"payload"[..]);
    let selected = Value::map(vec![(MapKey::Int(0), Value::Bytes(payload.clone()))]);
    let row = Value::map(vec![(MapKey::String("rank".into()), selected)]);
    let (mut fast, mut reference) = pair(RuntimeOptions::default());
    for runtime in [&mut fast, &mut reference] {
        runtime.set_global("row", row.clone()).unwrap();
    }
    let before = Rc::strong_count(&payload);
    test_support::compare_source(&mut fast, &mut reference, r#"$seen=$row["rank"];"#).unwrap();
    assert_eq!(Rc::strong_count(&payload), before + 2);
    test_support::compare_source(&mut fast, &mut reference, "$seen=null;$row=null;").unwrap();
    assert_eq!(Rc::strong_count(&payload), 2);
    drop(row);
    assert_eq!(Rc::strong_count(&payload), 1);
}
