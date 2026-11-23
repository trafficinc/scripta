use super::*;
use std::cell::Cell;

fn paired() -> (Runtime, Runtime) {
    let fast = Runtime::new();
    let mut slow = Runtime::new();
    slow.disable_method_cache = true;
    (fast, slow)
}

fn compare(
    fast: &mut Runtime,
    slow: &mut Runtime,
    source: &str,
) -> std::result::Result<u8, RuntimeError> {
    let mut a = Vec::new();
    let mut b = Vec::new();
    let result = fast.execute_source(source, &mut a);
    assert_eq!(result, slow.execute_source(source, &mut b), "{source}");
    assert_eq!(a, b);
    assert_eq!(fast.steps, slow.steps);
    assert_eq!(fast.environments.live, slow.environments.live);
    assert_eq!(fast.environments.current, slow.environments.current);
    assert!(fast.stack.is_empty() && slow.stack.is_empty());
    assert!(fast.environments.temporary_roots.is_empty());
    for name in ["seen", "events", "kept"] {
        assert_eq!(
            fast.get_global(name),
            slow.get_global(name),
            "{source}: {name}"
        );
    }
    result
}

#[test]
fn reused_spans_change_names_fields_classes_and_receivers_without_stale_bindings() {
    let (mut fast, mut slow) = paired();
    compare(&mut fast, &mut slow,
        "class A{$field=function(){return 9;};function first(){return 1;}function other(){return 2;}}class B{function first(){return 3;}}$o=new A();"
    ).unwrap();
    let source = "$seen=0;for($i=0;$i<4;$i+=1){$seen+=$o->first();}";
    for (text, expected) in [
        (source.to_owned(), 4),
        (source.replace("first", "other"), 8),
        (source.replace("first", "field"), 36),
    ] {
        compare(&mut fast, &mut slow, &text).unwrap();
        assert_eq!(fast.get_global("seen"), Some(&Value::Int(expected)));
    }
    assert!(
        fast.environments.scope(0).classes["A"]
            .method_cache
            .hits
            .get()
            > 0
    );
    compare(&mut fast, &mut slow, "$o->field=function(){return 11;};").unwrap();
    compare(&mut fast, &mut slow, &source.replace("first", "field")).unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(44)));
    compare(&mut fast, &mut slow, "$o=new B();").unwrap();
    compare(&mut fast, &mut slow, source).unwrap();
    assert_eq!(fast.get_global("seen"), Some(&Value::Int(12)));
}

#[derive(Debug)]
struct NativeProbe {
    casts: Rc<Cell<usize>>,
    reads: Rc<Cell<usize>>,
}
impl NativeObject for NativeProbe {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.casts.set(self.casts.get() + 1);
        None
    }
    fn type_name(&self) -> &str {
        "Counter"
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        self.reads.set(self.reads.get() + 1);
        Ok(Value::native_function(
            "next",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Int], NativeType::Int),
                |_, args| Ok(args[0].clone()),
            ),
        ))
    }
}

#[test]
fn native_getters_and_foreign_runtime_checks_are_not_bypassed_by_warm_sites() {
    let (mut fast, mut slow) = paired();
    let mut counters = Vec::new();
    for runtime in [&mut fast, &mut slow] {
        let casts = Rc::new(Cell::new(0));
        let reads = Rc::new(Cell::new(0));
        runtime
            .set_global(
                "native",
                Value::NativeObject(Rc::new(NativeProbe {
                    casts: casts.clone(),
                    reads: reads.clone(),
                })),
            )
            .unwrap();
        counters.push((casts, reads));
    }
    compare(&mut fast, &mut slow,
        "class Counter{function next($v){return $v+1;}}function invoke($o){return $o->next(4);}$script=new Counter();$seen=[invoke($script),invoke($script),invoke($native),invoke($script),invoke($native)];"
    ).unwrap();
    assert_eq!(
        fast.get_global("seen"),
        Some(&Value::List([5, 5, 4, 5, 4].map(Value::Int).into()))
    );
    assert_eq!(counters[0].0.get(), counters[1].0.get());
    assert_eq!(counters[0].1.get(), 2);
    assert_eq!(counters[1].1.get(), 2);
    let mut foreign = Runtime::new();
    foreign
        .set_global("object", fast.get_global("script").unwrap().clone())
        .unwrap();
    let error = foreign
        .execute_source(
            "$events=0;$kept=7;$kept=$object->next($events+=1);",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(
        error
            .message
            .contains("object belongs to a different runtime")
    );
    assert_eq!(foreign.get_global("events"), Some(&Value::Int(0)));
    assert_eq!(foreign.get_global("kept"), Some(&Value::Int(7)));
}

#[test]
fn inherited_hits_preserve_sources_gc_roots_and_do_not_retain_metadata() {
    let mut runtime = Runtime::new();
    let previous = runtime.environments.enter(0);
    let definition = "$secret=42;class Base extends Error{function read($v:int):int{if($v===0){return 1/0;}return $secret+$v;}}";
    runtime.execute_source(definition, &mut Vec::new()).unwrap();
    let base = runtime
        .environments
        .scope(runtime.environments.current)
        .classes["Base"]
        .clone();
    let weak_method = Rc::downgrade(&base.declaration.methods[0]);
    runtime.environments.leave(previous);
    runtime
        .environments
        .scope_mut(0)
        .classes
        .insert("Base".into(), base);
    runtime.execute_source("class Child extends Base{}$o=new Child('');function invoke($o,$cb){return $o->read($cb());}$seen=0;", &mut Vec::new()).unwrap();
    let weak_class = Rc::downgrade(&runtime.environments.scope(0).classes["Child"]);
    runtime
        .execute_source(
            "$seen=invoke($o,function(){return 1;});$seen=invoke($o,function(){return 1;});",
            &mut Vec::new(),
        )
        .unwrap();
    assert!(weak_class.upgrade().unwrap().method_cache.hits.get() > 0);
    assert_eq!(weak_method.strong_count(), 1);
    let error = runtime
        .execute_source("$seen=invoke($o,function(){return 0;});", &mut Vec::new())
        .unwrap_err();
    assert_eq!(error.source.as_ref().unwrap().text, definition);
    assert_eq!(&definition[error.span.start..error.span.end], "1/0");
    runtime
        .register(
            "prune",
            NativeFunction::reentrant(
                NativeSignature::new(vec![], NativeType::Int),
                |context, _| {
                    let globals = context.runtime.environments.scope_mut(0);
                    globals.variables.remove("o");
                    globals.classes.clear();
                    context.runtime.collect_garbage();
                    Ok(Value::Int(1))
                },
            ),
        )
        .unwrap();
    runtime
        .execute_source("$seen=invoke($o,prune);", &mut Vec::new())
        .unwrap();
    assert_eq!(runtime.get_global("seen"), Some(&Value::Int(43)));
    runtime.collect_garbage();
    assert!(weak_class.upgrade().is_none());
    assert!(weak_method.upgrade().is_none());
    assert_eq!(runtime.environments.live, 1);
    assert!(runtime.environments.temporary_roots.is_empty());
}

#[test]
fn warmed_method_hits_preserve_exact_limits_errors_and_recovery() {
    let mut options = Vec::new();
    for steps in 0..100 {
        options.push(RuntimeOptions {
            instruction_limit: Some(steps),
            ..Default::default()
        });
    }
    for scopes in 0..16 {
        options.push(RuntimeOptions {
            max_scopes: Some(scopes),
            ..Default::default()
        });
    }
    for slots in 0..6 {
        options.push(RuntimeOptions {
            max_value_bytes: Some(slots * std::mem::size_of::<Value>()),
            ..Default::default()
        });
    }
    for limit in options {
        let (mut fast, mut slow) = paired();
        compare(&mut fast, &mut slow, "class C{function read($n:int):int{if($n===0){return 1;}return $this->read($n-1)+1;}}$o=new C();$seen=$o->read(3);$kept='unchanged';").unwrap();
        fast.options = limit.clone();
        slow.options = limit;
        let _ = compare(&mut fast, &mut slow, "$kept=$o->read(4);");
        fast.options = RuntimeOptions::default();
        slow.options = RuntimeOptions::default();
        compare(&mut fast, &mut slow, "$seen=$o->read(2);").unwrap();
    }
}

#[test]
fn warmed_argument_default_and_body_cancellation_unwinds_and_recovers() {
    use std::sync::atomic::Ordering;
    for (method, call) in [
        ("function read($x){return $x;}", "$o->read(step())"),
        ("function read($x=step()){return $x;}", "$o->read()"),
        ("function read(){step();return 1;}", "$o->read()"),
    ] {
        let flag = Arc::new(AtomicBool::new(false));
        let armed = Rc::new(Cell::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        });
        let trigger = flag.clone();
        let state = armed.clone();
        runtime
            .register(
                "step",
                NativeFunction::new(
                    NativeSignature::new(vec![], NativeType::Int),
                    move |_, _| {
                        if state.get() {
                            trigger.store(true, Ordering::Relaxed);
                        }
                        Ok(Value::Int(1))
                    },
                ),
            )
            .unwrap();
        runtime.execute_source(&format!("class C{{{method}}}$o=new C();function invoke($o){{return {call};}}$seen=invoke($o);$seen=invoke($o);$kept='unchanged';$caught=false;"), &mut Vec::new()).unwrap();
        assert!(
            runtime.environments.scope(0).classes["C"]
                .method_cache
                .hits
                .get()
                > 0
        );
        armed.set(true);
        let error = runtime
            .execute_source(
                "try{$kept=invoke($o);}catch(Error $e){$caught=true;}",
                &mut Vec::new(),
            )
            .unwrap_err();
        assert!(error.message.contains("execution interrupted"));
        assert_eq!(
            runtime.get_global("kept"),
            Some(&Value::String("unchanged".into()))
        );
        assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
        assert!(runtime.stack.is_empty() && runtime.environments.temporary_roots.is_empty());
        flag.store(false, Ordering::Relaxed);
        armed.set(false);
        runtime
            .execute_source("$seen=invoke($o);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(1)));
    }
}
