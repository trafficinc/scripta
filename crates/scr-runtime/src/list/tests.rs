use super::*;
use crate::{MapKey, NativeObject, RuntimeOptions};

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
fn plain_clones_share_until_host_mutation_and_keep_owned_vector_transfers() {
    let input = vec![Value::String("original".into()), Value::Int(2)];
    let pointer = input.as_ptr();
    let mut original = List::from(input);
    assert_eq!(original.as_ptr(), pointer);
    let mut copied = original.clone();
    assert_eq!(copied.as_ptr(), pointer);
    copied[0] = Value::String("changed".into());
    assert_ne!(copied.as_ptr(), pointer);
    assert_eq!(original[0], Value::String("original".into()));
    original.push(Value::Int(3));
    assert_eq!(copied.len(), 2);
    let next = copied.clone();
    assert_eq!(next.as_ptr(), copied.as_ptr());
    let detached = next.into_vec();
    assert_ne!(detached.as_ptr(), copied.as_ptr());
    let pointer = copied.as_ptr();
    let owned = copied.into_vec();
    assert_eq!(owned.as_ptr(), pointer);
    // Changing the list payload must not enlarge logical slots used by limits.
    assert_eq!(size_of::<Value>(), 4 * size_of::<usize>());
}

#[test]
fn nested_map_and_list_writes_detach_at_each_shared_list_boundary() {
    let original = List::from(vec![Value::map(vec![(
        MapKey::String("items".into()),
        Value::list(vec![Value::String("世界".into())]),
    )])]);
    let mut copied = original.clone();
    let Value::Map(entries) = &mut copied[0] else {
        panic!()
    };
    let Value::List(items) = &mut entries[0].1 else {
        panic!()
    };
    items[0] = Value::String("edited".into());
    assert_ne!(copied, original);
    let Value::Map(entries) = &original[0] else {
        panic!()
    };
    let Value::List(items) = &entries[0].1 else {
        panic!()
    };
    assert_eq!(items[0], Value::String("世界".into()));
}

#[derive(Debug)]
struct Object;
impl NativeObject for Object {
    fn type_name(&self) -> &str {
        "Object"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("no property".into())
    }
}

#[test]
fn inserted_handles_disable_sharing_without_changing_native_or_byte_alias_counts() {
    let object: Rc<dyn NativeObject> = Rc::new(Object);
    let bytes: Rc<[u8]> = Rc::from(&b"bytes"[..]);
    let mut original = List::from(vec![Value::Int(1)]);
    let before = original.clone();
    original.push(Value::map(vec![(
        MapKey::Int(0),
        Value::list(vec![
            Value::NativeObject(object.clone()),
            Value::Bytes(bytes.clone()),
        ]),
    )]));
    let copied = original.clone();
    assert_ne!(original.as_ptr(), copied.as_ptr());
    assert_eq!(before.as_slice(), &[Value::Int(1)]);
    assert_eq!(Rc::strong_count(&object), 3);
    assert_eq!(Rc::strong_count(&bytes), 3);
    drop(copied);
    assert_eq!(Rc::strong_count(&object), 2);
    original.pop();
    let copied = original.clone();
    assert_eq!(original.as_ptr(), copied.as_ptr());
    assert_eq!(Rc::strong_count(&object), 1);
    assert_eq!(Rc::strong_count(&bytes), 1);
}

fn compare(source: &str, options: RuntimeOptions) {
    let (mut fast, mut reference) = crate::test_support::runtimes(options);
    let first = Cell::new(true);
    let _ = crate::test_support::compare_execution(
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
}

#[test]
fn script_snapshots_failures_closures_and_resources_match_eager_clones() {
    for source in [
        "$a=[['rank'=>1,'data'=>['世界',2]]];$b=$a;$b[0]['data'][0]='changed';$seen=$a[0]['data'][0];",
        "function f($x){$x[0][0]=7;return $x;}$a=[[1]];$b=f($a);$seen=$a[0][0];",
        "$a=[[1,2]];$b=$a;try{$b[0][10]=7;}catch(IndexError $e){}$seen=$a===$b;",
        "$a=[[1,2]];$b=$a;try{$b[0][0]='x'+1;}catch(TypeError $e){}$seen=$a===$b;",
        "$a=[1,2,3];$b=[...$a];$b[0]=7;$seen=[$a,$b];",
        "$a=[[1]];$b=[$x for $x in $a];$b[0][0]=9;$seen=$a[0][0];",
        "$a=[[1],[2]];$b=$a;foreach($a as $v){$v[0]=7;}$seen=[$a,$b];",
        "function make(){ $x=7;return [function(){return $x;}]; }$a=make();$b=$a;$a=[];for($i=0;$i<300;$i+=1){make();}$seen=$b[0]();",
        "class C{$v=1;}$c=new C();$a=[[$c]];$b=$a;$b[0][0]->v=7;$seen=$a[0][0]->v;",
        "function f($x){return [$x,$x];}$a=[1,2];$b=f($a);$b[0][]=3;$seen=[$a,$b];",
    ] {
        compare(source, RuntimeOptions::default());
    }
}

#[test]
fn exact_budgets_cancellation_and_failed_destinations_match_eager_clones() {
    let source = "$a=[[1,2]];$b=$a;$seen=42;$b[0][]=3;$seen=[$a,$b];";
    let mut options = Vec::new();
    for steps in 0..45 {
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
    let slot = size_of::<Value>();
    for bytes in [
        0,
        slot - 1,
        slot,
        3 * slot - 1,
        3 * slot,
        3 * slot + 1,
        4 * slot - 1,
        4 * slot,
        4 * slot + 1,
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
        compare(source, option);
    }
}
