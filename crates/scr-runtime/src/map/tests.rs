use super::*;
use crate::{NativeObject, RuntimeOptions};

fn eager<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            EAGER_CLONES.set(self.0);
        }
    }
    let _restore = Restore(EAGER_CLONES.replace(true));
    run()
}

#[test]
fn typed_entries_share_until_key_or_value_mutation_and_keep_owned_transfers() {
    let entries = vec![
        (MapKey::Int(1), Value::String("integer".into())),
        (MapKey::String("1".into()), Value::String("text".into())),
    ];
    let pointer = entries.as_ptr();
    let original = Map::from(entries);
    let mut copied = original.clone();
    assert_eq!(original.as_ptr(), pointer);
    assert_eq!(copied.as_ptr(), pointer);
    copied[0].1 = Value::Int(7);
    copied[1].0 = MapKey::String("renamed".into());
    copied.push((MapKey::Int(2), Value::Null));
    assert_ne!(copied.as_ptr(), pointer);
    assert_eq!(original.len(), 2);
    assert_eq!(original[0].1, Value::String("integer".into()));
    assert_eq!(original[1].0, MapKey::String("1".into()));
    let shared = copied.clone();
    assert_eq!(shared.as_ptr(), copied.as_ptr());
    let detached = shared.into_vec();
    assert_ne!(detached.as_ptr(), copied.as_ptr());
    let pointer = copied.as_ptr();
    let owned = copied.into_vec();
    assert_eq!(owned.as_ptr(), pointer);
    assert_eq!(size_of::<Value>(), 4 * size_of::<usize>());
}

#[test]
fn nested_map_list_mutations_preserve_snapshots_and_language_equality() {
    let original = Value::map(vec![(
        MapKey::String("items".into()),
        Value::list(vec![Value::map(vec![(MapKey::Int(1), Value::Int(2))])]),
    )]);
    let mut copied = original.clone();
    let Value::Map(entries) = &mut copied else {
        panic!()
    };
    let Value::List(items) = &mut entries[0].1 else {
        panic!()
    };
    let Value::Map(nested) = &mut items[0] else {
        panic!()
    };
    nested[0].1 = Value::Int(7);
    assert_ne!(original, copied);
    let a = Value::map(vec![
        (MapKey::Int(1), Value::Int(2)),
        (MapKey::String("1".into()), Value::Int(3)),
    ]);
    let b = Value::map(vec![
        (MapKey::String("1".into()), Value::Int(3)),
        (MapKey::Int(1), Value::Int(2)),
    ]);
    assert_eq!(a, b);
    let Value::Map(entries) = &original else {
        panic!()
    };
    let Value::List(items) = &entries[0].1 else {
        panic!()
    };
    let Value::Map(nested) = &items[0] else {
        panic!()
    };
    assert_eq!(nested[0].1, Value::Int(2));
}

#[derive(Debug)]
struct Object;
impl crate::NativeObject for Object {
    fn type_name(&self) -> &str {
        "Object"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no property".into())
    }
}

#[test]
fn inserted_handles_keep_native_byte_and_callable_ownership_counts() {
    let object: Rc<dyn NativeObject> = Rc::new(Object);
    let bytes: Rc<[u8]> = Rc::from(&b"bytes"[..]);
    let callable = Value::native_function(
        "held",
        crate::NativeFunction::new(
            crate::NativeSignature::new(vec![], crate::NativeType::Null),
            |_, _| Ok(Value::Null),
        ),
    );
    let Value::Callable(function) = &callable else {
        panic!()
    };
    let weak = Rc::downgrade(&function.0);
    let mut original = Map::from(vec![(MapKey::Int(0), Value::Int(1))]);
    let before = original.clone();
    original.push((
        MapKey::Int(1),
        Value::list(vec![
            Value::NativeObject(object.clone()),
            Value::Bytes(bytes.clone()),
            callable,
        ]),
    ));
    let copied = original.clone();
    assert_ne!(copied.as_ptr(), original.as_ptr());
    assert_eq!(before.len(), 1);
    assert_eq!(Rc::strong_count(&object), 3);
    assert_eq!(Rc::strong_count(&bytes), 3);
    assert_eq!(weak.strong_count(), 2);
    drop(copied);
    original.pop();
    let shared = original.clone();
    assert_eq!(shared.as_ptr(), original.as_ptr());
    assert_eq!(Rc::strong_count(&object), 1);
    assert_eq!(Rc::strong_count(&bytes), 1);
    assert!(weak.upgrade().is_none());
}

fn compare(source: &str, options: RuntimeOptions) -> Result<u8, crate::RuntimeError> {
    let (mut fast, mut reference) = crate::test_support::runtimes(options);
    let first = Cell::new(true);
    let result = crate::test_support::compare_execution(
        &mut fast,
        &mut reference,
        source,
        |runtime, output| {
            if first.replace(false) {
                runtime.execute_source(source, output)
            } else {
                eager(|| runtime.execute_source(source, output))
            }
        },
    );
    let _ = crate::test_support::compare_source(&mut fast, &mut reference, "$recovered=7;");
    result
}

#[test]
fn map_writes_keys_duplicates_closures_and_errors_match_eager_map_clones() {
    for source in [
        "$a=[1=>'int','1'=>'text','nested'=>['x'=>[1]]];$b=$a;$b[1]='changed';$b['nested']['x'][0]=7;$b['new']=9;$seen=[$a,$b];",
        "function f($x){$x['rank']+=1;return $x;}$a=['rank'=>1];$b=f($a);$seen=[$a,$b];",
        "$a=['x'=>[1]];$b=$a;try{$b['x'][10]=7;}catch(IndexError $e){}$seen=$a===$b;",
        "$a=['x'=>1];$b=$a;try{$b[false]=7;}catch(TypeError $e){}$seen=$a===$b;",
        "$a=['x'=>1];$b=$a;try{$b['missing']['x']=7;}catch(IndexError $e){}$seen=$a===$b;",
        "$a=['x'=>1,'x'=>2,'y'=>3];$b=$a;$b['x']=7;$seen=[$a,$b];",
        "$a=['x'=>1,'y'=>2];$b=$a;foreach($a as $k=>$v){$a[$k]+=10;}$seen=[$a,$b];",
        "function make(){ $x=7;return ['f'=>function(){return $x;}];}$a=make();$b=$a;$a=[];for($i=0;$i<300;$i+=1){make();}$seen=$b['f']();",
        "class C{$v=1;}$c=new C();$a=['x'=>['c'=>$c]];$b=$a;$b['x']['c']->v=7;$seen=$a['x']['c']->v;",
        "$a=['x'=>1];$a['copy']=$a;$b=$a;$b['copy']['x']=7;$seen=[$a,$b];",
    ] {
        compare(source, RuntimeOptions::default()).unwrap();
    }
    // Warm ordered-map hints, replace the backing map with a same-width value,
    // and perform shared writes using both integer and string keys.
    let fields = (0..80)
        .map(|i| format!("{i}=>{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let reversed = (0..80)
        .rev()
        .map(|i| format!("{i}=>{i}"))
        .collect::<Vec<_>>()
        .join(",");
    compare(&format!("$a=[{fields}];$seen=$a[70];$b=$a;$b[70]=900;$a=[{reversed}];$a['70']=901;$seen=[$a[70],$a['70'],$b[70]];if($seen!==[70,901,900])throw 'wrong hint';"), RuntimeOptions::default()).unwrap();
}

#[test]
fn exact_limits_cancellation_and_failed_destinations_match_eager_map_clones() {
    let source = "$a=['x'=>1];$b=$a;$seen=42;$b['x']=2;$b['y']=[1];$seen=[$a,$b];";
    let mut options = Vec::new();
    for steps in 0..50 {
        options.push(RuntimeOptions {
            instruction_limit: Some(steps),
            ..Default::default()
        });
    }
    for scopes in 0..5 {
        options.push(RuntimeOptions {
            max_scopes: Some(scopes),
            ..Default::default()
        });
    }
    let exact = Value::map(vec![(MapKey::String("x".into()), Value::Int(1))])
        .measure(0, None)
        .unwrap();
    for bytes in [
        0,
        size_of::<Value>() - 1,
        size_of::<Value>(),
        exact - 1,
        exact,
        exact + 1,
        exact + MapKey::LOGICAL_BYTES + 2 * size_of::<Value>(),
        1024,
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
        let _ = compare(source, option);
    }
}

#[test]
fn sharing_does_not_trust_duplicate_keys_invalid_floats_or_excessive_depth() {
    let mut deep = Value::Int(1);
    for _ in 0..129 {
        deep = Value::map(vec![(MapKey::Int(0), deep)]);
    }
    for value in [
        Value::map(vec![
            (MapKey::Int(0), Value::Int(1)),
            (MapKey::Int(0), Value::Int(2)),
        ]),
        Value::map(vec![(MapKey::Int(0), Value::Float(f64::NAN))]),
        deep,
    ] {
        let copied = value.clone();
        assert_eq!(copied.measure(0, None), value.measure(0, None));
        assert!(copied.measure(0, None).is_err());
        for runtime_value in [value, copied] {
            assert!(
                crate::Runtime::new()
                    .set_global("invalid", runtime_value)
                    .is_err()
            );
        }
    }
}
