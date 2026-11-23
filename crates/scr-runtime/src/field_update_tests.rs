use super::*;
use objects::ScriptObject;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

fn pair(options: RuntimeOptions) -> (Runtime, Runtime) {
    let fast = Runtime::with_options(options.clone());
    let mut slow = Runtime::with_options(options);
    slow.force_field_snapshot = true;
    slow.force_hashed_fields = true;
    (fast, slow)
}
fn compare(fast: &mut Runtime, slow: &mut Runtime, source: &str) -> bool {
    let mut a = Vec::new();
    let mut b = Vec::new();
    let result = fast.execute_source(source, &mut a);
    assert_eq!(result, slow.execute_source(source, &mut b), "{source}");
    if let Err(error) = &result {
        eprintln!("{source}: {error:?}");
    }
    assert_eq!(a, b);
    assert_eq!(fast.steps, slow.steps);
    assert_eq!(fast.environments.live, slow.environments.live);
    assert!(fast.stack.is_empty());
    assert!(slow.stack.is_empty());
    for (name, value) in &fast.environments.scope(0).variables {
        match value {
            Value::NativeObject(object) => {
                if let Some(instance) = object
                    .as_any()
                    .and_then(|v| v.downcast_ref::<ScriptObject>())
                {
                    let Some(Value::NativeObject(other)) = slow.get_global(name) else {
                        panic!("{name}")
                    };
                    let other = other
                        .as_any()
                        .unwrap()
                        .downcast_ref::<ScriptObject>()
                        .unwrap();
                    assert_eq!(
                        instance.payload.get(),
                        other.payload.get(),
                        "{source}: {name}"
                    );
                }
            }
            Value::Callable(_) => {}
            _ => assert_eq!(Some(value), slow.get_global(name), "{source}: {name}"),
        }
    }
    result.is_ok()
}

#[test]
fn borrowed_updates_preserve_inheritance_types_nested_values_and_failed_writes() {
    for source in [
        "class C{$x=0;function next(){return $this->x+=1;}}$c=new C();$a=$c->next();$b=$c->next();",
        "class Base extends Error{$a:int=0;}class C extends Base{$b='é🙂';}$c=new C('test');$c->a=3;$c->b='世界';$a=$c->a;$b=$c->b;",
        "class C{$x=[1,2];$map=['é'=>[3,4]];}$c=new C();$copy=$c->x;$c->x[0]=9;$c->map['é'][1]+=2;$a=$c->x;$b=$c->map;",
        "class C{$x=0;$m=[1=>'integer','1'=>'string','dup'=>1,'dup'=>2];}$c=new C();$copy=$c->m;$c->x=1;$c->m['dup']=3;$seen=$copy;",
        "class C{$x:int=1;}$c=new C();try{$c->x='bad';}catch(TypeError $e){$seen=$c->x;}",
        "class C{$x=[1];}$c=new C();try{$c->x[9]=2;}catch(IndexError $e){$seen=$c->x;}",
        "class C{$x=0;$other=[1,2,3];}$c=new C();$c->x=$c->other;$copy=$c->x;$c->other[0]=9;$seen=$c->x;",
        "class E extends Error{$x=0;}$c=new E('test');$c->x=2;try{throw $c;}catch(E $e){$seen=$e->x;}",
    ] {
        let (mut fast, mut slow) = pair(RuntimeOptions::default());
        assert!(compare(&mut fast, &mut slow, source), "{source}");
    }
}

#[test]
fn field_slots_handle_missing_defaults_out_of_order_writes_and_promotion() {
    for source in [
        "class C{$a:int;$b=2;$c:int;$d=4;$e=5;$f=6;$g=7;$h=8;$i=9;$j:int;}$o=new C();$b=$o->b;$o->j=10;$o->a=1;$o->c=3;$seen=[$o->a,$o->b,$o->c,$o->d,$o->j];",
        "class C{$a:int;$b=2;} $o=new C();try{$o->a;}catch(PropertyError $e){$missing=true;}$o->a=1;try{$o->a='bad';}catch(TypeError $e){}$seen=[$o->a,$o->b];",
        "class C{$a=1;$b=2;$c=3;$d=4;$e=5;$f=6;$g=7;$h=8;$i=9;$j=10;$k=11;$l=12;}$o=new C();$o->l+=10;$o->a=$o->l;$seen=[$o->a,$o->h,$o->l];",
        "class Base extends Error{$a:int;$b=2;}class Child extends Base{$c:int;$d=4;}$o=new Child('message');$o->c=3;$o->a=1;$seen=[$o->a,$o->b,$o->c,$o->d,$o->message];",
        "class C{$a:int;$b=[1];}$o=new C();function rhs($o){$o->a=7;$o->b[0]=9;return [3];}$o->b=rhs($o);$seen=[$o->a,$o->b];",
    ] {
        let (mut fast, mut slow) = pair(RuntimeOptions::default());
        assert!(compare(&mut fast, &mut slow, source), "{source}");
        assert_eq!(fast.get_global("seen"), slow.get_global("seen"));
    }
}

#[test]
fn ordered_defaults_keep_sparse_field_values_and_prior_failure_effects() {
    let mut runtime = Runtime::new();
    runtime.execute_source(
        "$count=0;class C{$missing:int;$first=($count+=1);$other:int;$last=($count+=1);}$o=new C();$p=new C();$seen=[$o->first,$o->last,$p->first,$p->last];class Bad{$first=($count+=1);$bad:int='wrong';}$failed='kept';try{$failed=new Bad();}catch(TypeError $e){}",
        &mut Vec::new(),
    ).unwrap();
    assert_eq!(
        runtime.get_global("seen"),
        Some(&Value::List(
            [1, 2, 3, 4].into_iter().map(Value::Int).collect()
        ))
    );
    assert_eq!(runtime.get_global("count"), Some(&Value::Int(5)));
    assert_eq!(
        runtime.get_global("failed"),
        Some(&Value::String("kept".into()))
    );
}

#[test]
fn initialized_wide_fields_keep_declaration_order_and_release_instances() {
    let mut runtime = Runtime::new();
    runtime
        .execute_source(
            "class C{$z=1;$a=2;$m=3;$n=4;$o=5;$p=6;$q=7;$r=8;$s=9;}$o=new C();$o->s=10;",
            &mut Vec::new(),
        )
        .unwrap();
    let Some(Value::NativeObject(object)) = runtime.get_global("o") else {
        panic!("object");
    };
    let instance = object
        .as_any()
        .unwrap()
        .downcast_ref::<ScriptObject>()
        .unwrap();
    let crate::bindings::Bindings::Slots(slots) =
        &runtime.environments.scope(instance.scope).variables
    else {
        panic!("initialized wide objects retain checked slots");
    };
    assert_eq!(
        slots
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["z", "a", "m", "n", "o", "p", "q", "r", "s"]
    );
    assert_eq!(slots[8].1, Value::Int(10));
    let weak = Rc::downgrade(object);
    runtime.execute_source("$o=null;", &mut Vec::new()).unwrap();
    runtime.collect_garbage();
    assert!(weak.upgrade().is_none());
}

#[test]
fn exact_instruction_scope_and_byte_limits_match_snapshot_measurement() {
    let source = "class C{$a=0;$b='é';$c=[1,2];}$c='kept';$c=new C();$c->a=4;$seen=$c->a;";
    for limit in 0..70 {
        let (mut fast, mut slow) = pair(RuntimeOptions {
            instruction_limit: Some(limit),
            ..RuntimeOptions::default()
        });
        compare(&mut fast, &mut slow, source);
    }
    for limit in 1..=6 {
        let (mut fast, mut slow) = pair(RuntimeOptions {
            max_scopes: Some(limit),
            ..RuntimeOptions::default()
        });
        compare(&mut fast, &mut slow, source);
    }
    let slot = std::mem::size_of::<Value>();
    let exact = slot * 5 + 1 + 3 + 4;
    for limit in [exact - 1, exact, exact + 1] {
        let (mut fast, mut slow) = pair(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..RuntimeOptions::default()
        });
        let created = compare(
            &mut fast,
            &mut slow,
            "class C{$x='abc';$list=[1,2];}$c='kept';$c=new C();",
        );
        assert_eq!(created, limit >= exact);
        if created {
            assert_eq!(
                compare(&mut fast, &mut slow, "$c->x='abcd';"),
                limit > exact
            );
            compare(&mut fast, &mut slow, "$seen=$c->x;");
            assert_eq!(
                fast.get_global("seen"),
                Some(&Value::String(
                    if limit == exact { "abc" } else { "abcd" }.into()
                ))
            );
        }
    }
}

#[derive(Debug)]
struct Observed {
    owner: Weak<Observed>,
    events: Rc<RefCell<Vec<usize>>>,
    bytes: Option<Weak<[u8]>>,
    flag: Option<Arc<AtomicBool>>,
    armed: Rc<Cell<bool>>,
}
impl NativeObject for Observed {
    fn type_name(&self) -> &str {
        "Observed"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("no property".into())
    }
    fn payload_bytes(&self) -> usize {
        let count = self
            .bytes
            .as_ref()
            .map_or_else(|| self.owner.strong_count(), Weak::strong_count);
        self.events.borrow_mut().push(count);
        if self.armed.replace(false) {
            self.flag.as_ref().unwrap().store(true, Ordering::Relaxed);
        }
        count
    }
}
fn observed(
    runtime: &mut Runtime,
    events: Rc<RefCell<Vec<usize>>>,
    flag: Option<Arc<AtomicBool>>,
    armed: Rc<Cell<bool>>,
) {
    let value = Rc::new_cyclic(|owner| Observed {
        owner: owner.clone(),
        bytes: None,
        events,
        flag,
        armed,
    });
    runtime
        .set_global("resource", Value::NativeObject(value))
        .unwrap();
}

#[test]
fn native_handle_ownership_callbacks_and_snapshot_transitions_match() {
    for initial in ["$resource", "null"] {
        for nested in [false, true] {
            let (mut fast, mut slow) = pair(RuntimeOptions::default());
            let a = Rc::new(RefCell::new(Vec::new()));
            let b = Rc::new(RefCell::new(Vec::new()));
            observed(&mut fast, a.clone(), None, Rc::new(Cell::new(false)));
            observed(&mut slow, b.clone(), None, Rc::new(Cell::new(false)));
            let source = format!("class C{{$x=0;$resource={initial};}}$c=new C();");
            assert!(compare(&mut fast, &mut slow, &source));
            if initial == "null" {
                let replacement = if nested {
                    "['key'=>[$resource]]"
                } else {
                    "$resource"
                };
                assert!(compare(
                    &mut fast,
                    &mut slow,
                    &format!("$c->resource={replacement};")
                ));
            }
            a.borrow_mut().clear();
            b.borrow_mut().clear();
            assert!(compare(&mut fast, &mut slow, "$c->x=3;"));
            assert_eq!(*a.borrow(), *b.borrow());
            assert_eq!(a.borrow().last(), Some(&3));
            assert!(compare(&mut fast, &mut slow, "$c->resource=null;$c->x=4;"));
            assert_eq!(*a.borrow(), *b.borrow());
        }
    }
}

#[test]
fn newly_stored_native_callbacks_observe_existing_shared_byte_owners() {
    let (mut fast, mut slow) = pair(RuntimeOptions::default());
    let a = Rc::new(RefCell::new(Vec::new()));
    let b = Rc::new(RefCell::new(Vec::new()));
    for (runtime, events) in [(&mut fast, a.clone()), (&mut slow, b.clone())] {
        let bytes: Rc<[u8]> = Rc::from([1_u8, 2, 3]);
        let weak = Rc::downgrade(&bytes);
        runtime.set_global("bytes", Value::Bytes(bytes)).unwrap();
        runtime
            .register(
                "observer",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Any),
                    move |_, _| {
                        let object = Rc::new_cyclic(|owner| Observed {
                            owner: owner.clone(),
                            events: events.clone(),
                            bytes: Some(weak.clone()),
                            flag: None,
                            armed: Rc::new(Cell::new(false)),
                        });
                        Ok(Value::NativeObject(object))
                    },
                ),
            )
            .unwrap();
    }
    assert!(compare(
        &mut fast,
        &mut slow,
        "class C{$bytes=$bytes;$value=0;}$c=new C();"
    ));
    a.borrow_mut().clear();
    b.borrow_mut().clear();
    assert!(compare(&mut fast, &mut slow, "$c->value=observer();"));
    assert_eq!(*a.borrow(), *b.borrow());
    assert_eq!(a.borrow().last(), Some(&3));
}

#[test]
fn interruption_from_last_native_payload_keeps_the_completed_assignment() {
    let mut errors = Vec::new();
    for snapshots in [false, true] {
        let flag = Arc::new(AtomicBool::new(false));
        let armed = Rc::new(Cell::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime.force_field_snapshot = snapshots;
        observed(
            &mut runtime,
            Rc::new(RefCell::new(Vec::new())),
            Some(flag),
            armed.clone(),
        );
        runtime
            .execute_source(
                "class C{$x=0;$resource=$resource;}$c=new C();",
                &mut Vec::new(),
            )
            .unwrap();
        let Some(Value::NativeObject(object)) = runtime.get_global("c").cloned() else {
            panic!()
        };
        armed.set(true);
        errors.push(
            runtime
                .execute_source("$c->x=7;$after=1;", &mut Vec::new())
                .unwrap_err(),
        );
        let instance = object
            .as_any()
            .unwrap()
            .downcast_ref::<ScriptObject>()
            .unwrap();
        assert_eq!(
            runtime
                .environments
                .scope(instance.scope)
                .variables
                .get("x"),
            Some(&Value::Int(7))
        );
        assert!(runtime.get_global("after").is_none());
        assert!(runtime.stack.is_empty());
    }
    assert_eq!(errors[0], errors[1]);
}

#[test]
fn depth_bounds_and_binary_unicode_payloads_preserve_accounting() {
    fn nested(depth: usize) -> Value {
        (0..depth).fold(Value::Int(1), |value, _| Value::List(vec![value]))
    }
    for depth in [127, 128] {
        let (mut fast, mut slow) = pair(RuntimeOptions::default());
        for runtime in [&mut fast, &mut slow] {
            let value = nested(depth);
            runtime
                .register(
                    "data",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Any),
                        move |_, _| Ok(value.clone()),
                    ),
                )
                .unwrap();
        }
        assert!(compare(&mut fast, &mut slow, "class C{$x=0;}$c=new C();"));
        assert_eq!(compare(&mut fast, &mut slow, "$c->x=data();"), depth == 127);
    }
    let (mut fast, mut slow) = pair(RuntimeOptions::default());
    for runtime in [&mut fast, &mut slow] {
        runtime
            .set_global("bytes", Value::Bytes(Rc::from([0_u8, 255, 128])))
            .unwrap();
    }
    assert!(compare(
        &mut fast,
        &mut slow,
        "class C{$b=$bytes;$s='é🙂';$m=['世界'=>[1,2]];}$c=new C();$copy=$c->b;$c->s='ö';$seen=$c->m;"
    ));
}

#[derive(Debug)]
struct Probe(Rc<Cell<usize>>);
impl Drop for Probe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
impl NativeObject for Probe {
    fn type_name(&self) -> &str {
        "Probe"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("no property".into())
    }
}

#[test]
fn receiver_and_callback_captures_survive_collection_without_snapshot_roots() {
    let dropped = Rc::new(Cell::new(0));
    let probe = Rc::new(Probe(dropped.clone()));
    let weak = Rc::downgrade(&probe);
    let mut runtime = Runtime::new();
    runtime
        .set_global("resource", Value::NativeObject(probe))
        .unwrap();
    runtime.execute_source("class C{$x=0;$callback;}function make($p){$c=new C();$c->callback=function(){return $p;};return $c;}$c=make($resource);",&mut Vec::new()).unwrap();
    let Some(Value::NativeObject(object)) = runtime.get_global("c").cloned() else {
        panic!()
    };
    runtime
        .environments
        .scope_mut(0)
        .variables
        .remove("resource");
    runtime.environments.scope_mut(0).variables.remove("c");
    let instance = object
        .as_any()
        .unwrap()
        .downcast_ref::<ScriptObject>()
        .unwrap();
    assert!(!instance.requires_snapshot.get());
    let dummy = Rc::new(Function::Native {
        name: Rc::from("dead"),
        function: NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
            Ok(Value::Null)
        }),
    });
    for _ in 0..2048 {
        runtime.environments.track_function(&dummy);
    }
    drop(dummy);
    assert!(
        runtime
            .set_object_field(&object, "x", Value::Int(9), Span::default())
            .is_ok()
    );
    assert!(weak.upgrade().is_some());
    let callback = runtime
        .object_property(&object, "callback", Span::default())
        .ok()
        .unwrap();
    assert!(matches!(
        runtime
            .invoke(callback.clone(), &[], &mut Vec::new())
            .unwrap(),
        Value::NativeObject(_)
    ));
    drop(callback);
    drop(object);
    runtime.collect_garbage();
    assert!(weak.upgrade().is_none());
    assert_eq!(dropped.get(), 1);
}
