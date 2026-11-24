use super::*;

fn pair() -> (Runtime, Runtime) {
    let (fast, mut reference) = crate::test_support::runtimes(RuntimeOptions::default());
    reference.disable_field_reads = true;
    (fast, reference)
}

fn compare(fast: &mut Runtime, reference: &mut Runtime, source: &str) {
    crate::test_support::compare_source(fast, reference, source).unwrap();
}

fn wide() -> String {
    format!(
        "class C{{{}}}$o=new C();",
        (0..64)
            .map(|i| format!("$f{i}:int={i};"))
            .collect::<String>()
    )
}

#[test]
fn warmup_counts_actual_reads_and_name_changes_disable_the_hint() {
    let (mut fast, mut reference) = pair();
    compare(&mut fast, &mut reference, &wide());
    for _ in 0..3 {
        compare(&mut fast, &mut reference, "$seen=$o->f63;");
    }
    assert!(
        fast.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .slot()
            .is_none()
    );
    compare(&mut fast, &mut reference, "$seen=$o->f63;");
    assert_eq!(
        fast.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .slot(),
        Some(63)
    );
    compare(&mut fast, &mut reference, "$seen=$o->f63;");
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(63)));
    assert!(
        fast.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .hits
            .get()
            > 0
    );
    assert_eq!(
        reference.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .hits
            .get(),
        0
    );
    for name in ["f62", "f63", "f62", "f63"] {
        compare(&mut fast, &mut reference, &format!("$seen=$o->{name};"));
    }
    assert!(
        !fast.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .enabled()
    );
    compare(&mut fast, &mut reference, "$o->f63=7;$seen=$o->f63;");
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(7)));
}

#[test]
fn early_and_small_reads_stay_untrained_and_failures_keep_values() {
    let (mut fast, mut reference) = pair();
    compare(&mut fast, &mut reference, &wide());
    compare(
        &mut fast,
        &mut reference,
        "for($i=0;$i<10;$i+=1){$seen=$o->f0;}",
    );
    assert!(
        fast.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .slot()
            .is_none()
    );
    compare(
        &mut fast,
        &mut reference,
        "class Small{$a=1;$b=2;}$s=new Small();for($i=0;$i<10;$i+=1){$seen=$s->b;}",
    );
    assert!(
        !fast.environments.scope(0).classes["Small"]
            .method_cache
            .field_reads
            .enabled()
    );
    compare(
        &mut fast,
        &mut reference,
        "for($i=0;$i<6;$i+=1){$seen=$o->f63;}try{$o->f63='bad';}catch(TypeError $e){}$seen=$o->f63;",
    );
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(63)));
    let Some(Value::NativeObject(object)) = fast.get_global("o") else {
        panic!("object");
    };
    let weak = Rc::downgrade(object);
    compare(&mut fast, &mut reference, "$o=null;");
    fast.collect_garbage();
    assert!(weak.upgrade().is_none());
}

#[derive(Debug)]
struct Switching {
    first: Rc<dyn NativeObject>,
    second: Rc<dyn NativeObject>,
    calls: std::cell::Cell<usize>,
}
impl NativeObject for Switching {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        let calls = self.calls.get();
        self.calls.set(calls + 1);
        if calls % 2 == 0 {
            self.first.as_any()
        } else {
            self.second.as_any()
        }
    }
    fn type_name(&self) -> &str {
        "proxy"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        panic!("script downcast expected")
    }
}

#[test]
fn warm_hint_preserves_two_host_downcasts_and_the_second_class() {
    let (mut fast, mut reference) = pair();
    compare(&mut fast, &mut reference, &wide());
    compare(
        &mut fast,
        &mut reference,
        "class Other{$f63=99;}$other=new Other();for($i=0;$i<6;$i+=1){$seen=$o->f63;}",
    );
    for runtime in [&mut fast, &mut reference] {
        let Some(Value::NativeObject(first)) = runtime.get_global("o").cloned() else {
            panic!("first");
        };
        let Some(Value::NativeObject(second)) = runtime.get_global("other").cloned() else {
            panic!("second");
        };
        let concrete = Rc::new(Switching {
            first,
            second,
            calls: std::cell::Cell::new(0),
        });
        let object: Rc<dyn NativeObject> = concrete.clone();
        for count in 1..=3 {
            let value = runtime
                .object_property(&object, "f63", Span::new(1, 2))
                .unwrap_or_else(|_| panic!("read failed"));
            assert_eq!(value, Value::Int(99));
            assert_eq!(concrete.calls.get(), count * 2);
        }
    }
}

#[test]
fn sparse_slots_and_inherited_layouts_keep_the_original_behavior() {
    let (mut fast, mut reference) = pair();
    let source = wide().replace("$f0:int=0;", "$f0:int;");
    compare(&mut fast, &mut reference, &source);
    compare(
        &mut fast,
        &mut reference,
        "for($i=0;$i<6;$i+=1){$seen=$o->f63;}$o->f0=7;$o->f63=42;$seen=$o->f63;",
    );
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(42)));
    assert!(
        fast.environments.scope(0).classes["C"]
            .method_cache
            .field_reads
            .hits
            .get()
            > 0
    );
    let fields = (0..64)
        .map(|i| format!("$f{i}:int={i};"))
        .collect::<String>();
    compare(
        &mut fast,
        &mut reference,
        &format!(
            "class E extends Error{{{fields}}}$e=new E('message');for($i=0;$i<6;$i+=1){{$seen=$e->f63;}}"
        ),
    );
    assert!(
        !fast.environments.scope(0).classes["E"]
            .method_cache
            .field_reads
            .enabled()
    );
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(63)));
}

#[test]
fn warmed_reads_preserve_exact_limits_cancellation_and_recovery() {
    for limit in 0..20 {
        let (mut fast, mut reference) = pair();
        compare(&mut fast, &mut reference, &wide());
        compare(
            &mut fast,
            &mut reference,
            "for($i=0;$i<6;$i+=1){$seen=$o->f63;}$kept=9;",
        );
        fast.options.instruction_limit = Some(limit);
        reference.options.instruction_limit = Some(limit);
        let _ = crate::test_support::compare_source(
            &mut fast,
            &mut reference,
            "$kept=$o->f63;$after=1;",
        );
    }
    let (mut fast, mut reference) = pair();
    compare(&mut fast, &mut reference, &wide());
    compare(
        &mut fast,
        &mut reference,
        "for($i=0;$i<6;$i+=1){$seen=$o->f63;}$kept=9;",
    );
    let flag = Arc::new(AtomicBool::new(true));
    fast.options.interrupt_flag = Some(flag.clone());
    reference.options.interrupt_flag = Some(flag.clone());
    assert!(
        crate::test_support::compare_source(&mut fast, &mut reference, "$kept=$o->f63;").is_err()
    );
    assert_eq!(fast.get_global("kept"), Some(&Value::Int(9)));
    flag.store(false, Ordering::Relaxed);
    compare(&mut fast, &mut reference, "$kept=$o->f63;");
    assert_eq!(fast.get_global("kept"), Some(&Value::Int(63)));
    let exact =
        std::mem::size_of::<Value>() * 65 + (0..64).map(|i| format!("f{i}").len()).sum::<usize>();
    for limit in [exact - 1, exact, exact + 1] {
        let (mut fast, mut reference) = pair();
        fast.options.max_value_bytes = Some(limit);
        reference.options.max_value_bytes = Some(limit);
        let source = wide() + "for($i=0;$i<6;$i+=1){$seen=$o->f63;}";
        let result = crate::test_support::compare_source(&mut fast, &mut reference, &source);
        assert_eq!(result.is_ok(), limit >= exact);
    }
}
