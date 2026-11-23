use scr_runtime::{
    NativeFunction, NativeObject, NativeSignature, NativeType, Runtime, RuntimeOptions, Value,
};
use std::{cell::Cell, rc::Rc};

#[derive(Default)]
struct Counts {
    created: Cell<usize>,
}

#[derive(Debug)]
struct Resource(Rc<Cell<usize>>);

impl Drop for Resource {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

impl NativeObject for Resource {
    fn type_name(&self) -> &str {
        "Resource"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Err("unknown property".into())
    }
}

fn tracked_runtime(options: RuntimeOptions) -> (Runtime, Rc<Counts>, Rc<Cell<usize>>) {
    let mut runtime = Runtime::with_options(options);
    let counts = Rc::new(Counts::default());
    let drops = Rc::new(Cell::new(0));
    let created = counts.clone();
    let dropped = drops.clone();
    runtime
        .register(
            "resource",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Object("Resource")),
                move |_, _| {
                    created.created.set(created.created.get() + 1);
                    Ok(Value::NativeObject(Rc::new(Resource(dropped.clone()))))
                },
            ),
        )
        .unwrap();
    let live_counts = counts.clone();
    let live_drops = drops.clone();
    runtime
        .register(
            "liveResources",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Int),
                move |_, _| {
                    Ok(Value::Int(
                        (live_counts.created.get() - live_drops.get()) as i64,
                    ))
                },
            ),
        )
        .unwrap();
    (runtime, counts, drops)
}

fn run(runtime: &mut Runtime, body: &str) -> Result<(), scr_runtime::RuntimeError> {
    runtime.execute_source(&format!("<?scr {body}"), &mut std::io::sink())?;
    Ok(())
}

#[test]
fn temporary_resources_drop_after_every_batch_in_one_runtime() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        max_scopes: Some(8),
        ..Default::default()
    });
    run(
        &mut runtime,
        r#"
        function work() {
            $r = resource(); $text = "temporary" . " value";
            $list = [$text, $text]; $map = ["items" => $list];
            $copy = $map; $copy["items"][0] = "changed";
        }
    "#,
    )
    .unwrap();
    for batch in 1..=100 {
        run(&mut runtime, "for ($i=0; $i<100; $i+=1) { work(); }").unwrap();
        assert_eq!(counts.created.get(), batch * 100);
        assert_eq!(drops.get(), counts.created.get());
    }
}

#[test]
fn repeated_errors_release_uncaptured_resources_and_allow_recovery() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        max_scopes: Some(12),
        instruction_limit: Some(100),
        ..Default::default()
    });
    run(
        &mut runtime,
        r#"
        function thrown() { $r=resource(); throw "failure"; }
        function arithmetic() { $r=resource(); 1 / 0; }
        function exhausted() { $r=resource(); while (true) {} }
    "#,
    )
    .unwrap();
    for _ in 0..500 {
        run(&mut runtime, "try { thrown(); } catch ($e) {}").unwrap();
        run(
            &mut runtime,
            "try { arithmetic(); } catch (ArithmeticError $e) {}",
        )
        .unwrap();
        let error = run(&mut runtime, "exhausted();").unwrap_err();
        assert!(error.message.contains("instruction limit"));
        assert_eq!(drops.get(), counts.created.get());
        run(&mut runtime, "$recovered = 42;").unwrap();
        assert_eq!(runtime.get_global("recovered"), Some(&Value::Int(42)));
    }
    assert_eq!(counts.created.get(), 1500);
}

#[test]
fn discarded_closures_release_captures_after_every_execution() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(
        &mut runtime,
        "function factory() { $r=resource(); return function () { return $r; }; }",
    )
    .unwrap();
    for batch in 1..=20 {
        run(
            &mut runtime,
            "for ($i=0; $i<50; $i+=1) { $f=factory(); $f=null; }",
        )
        .unwrap();
        assert_eq!(counts.created.get(), batch * 50);
        assert_eq!(drops.get(), counts.created.get());
    }
    drop(runtime);
    assert_eq!(drops.get(), counts.created.get());
}

#[test]
fn reachable_captures_obey_scope_limit_and_failure_unwinds() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        max_scopes: Some(16),
        ..Default::default()
    });
    run(
        &mut runtime,
        "function factory() { $r=resource(); return function () { return $r; }; }",
    )
    .unwrap();
    run(&mut runtime, "$kept=[];").unwrap();
    let error = run(
        &mut runtime,
        "for ($i=0; $i<100; $i+=1) { $kept[]=factory(); }",
    )
    .unwrap_err();
    assert!(error.message.contains("scope limit"));
    let retained = counts.created.get();
    assert!(retained > 0 && retained < 100);
    assert_eq!(drops.get(), 0);
    for _ in 0..100 {
        assert!(
            run(
                &mut runtime,
                "for ($i=0; $i<100; $i+=1) { $kept[]=factory(); }"
            )
            .unwrap_err()
            .message
            .contains("scope limit")
        );
        assert_eq!(counts.created.get(), retained);
        run(&mut runtime, "$recovered = true;").unwrap();
    }
    run(&mut runtime, "$kept=null;").unwrap();
    assert_eq!(drops.get(), retained);
    run(&mut runtime, "$f=factory(); $f=null;").unwrap();
    assert_eq!(drops.get(), counts.created.get());
}

#[test]
fn repeated_runtime_destruction_releases_global_and_captured_resources() {
    for _ in 0..100 {
        let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
        run(
            &mut runtime,
            r#"
            $global = resource();
            function factory() { $r=resource(); return function () { return $r; }; }
            for ($i=0; $i<20; $i+=1) { $f=factory(); $f=null; }
        "#,
        )
        .unwrap();
        assert_eq!(counts.created.get(), 21);
        assert_eq!(drops.get(), 20);
        drop(runtime);
        assert_eq!(drops.get(), 21);
        assert_eq!(Rc::strong_count(&counts), 1);
        assert_eq!(Rc::strong_count(&drops), 1);
    }
}

const FACTORY: &str =
    "function factory() { $r=resource(); $n=42; return function () { $n+=1; return $n; }; }";
const CHURN: &str =
    "function churn() { for ($j=0; $j<2000; $j+=1) { $f=factory(); $f=null; } return 0; }";

#[test]
fn automatic_collection_reclaims_during_a_single_long_execution() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(
        &mut runtime,
        &format!("{FACTORY} {CHURN} churn(); $live=liveResources();"),
    )
    .unwrap();
    let Some(Value::Int(live)) = runtime.get_global("live") else {
        panic!("missing live count")
    };
    assert!(
        *live < 300,
        "unbounded retention within a single execution: {live}"
    );
    assert_eq!(counts.created.get(), 2000);
    assert_eq!(drops.get(), 2000);
}

fn type_module(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/collector")
        .join(name);
    Value::String(path.to_string_lossy().into_owned())
}

#[test]
fn automatic_collection_covers_type_only_shared_includes() {
    for module in [
        "local-class.scripta",
        "local-interface.scripta",
        "local-method.scripta",
    ] {
        let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
        runtime.set_global("module", type_module(module)).unwrap();
        run(
            &mut runtime,
            "function factory($module) { $r=resource(); require $module; }",
        )
        .unwrap();
        run(
            &mut runtime,
            "for ($i=0; $i<2000; $i+=1) { factory($module); } $live=liveResources();",
        )
        .unwrap();
        let Some(Value::Int(live)) = runtime.get_global("live") else {
            panic!("live resources");
        };
        assert!(*live < 300, "unbounded {module} retention: {live}");
        assert_eq!(counts.created.get(), 2000);
        assert_eq!(drops.get(), counts.created.get());
        assert_eq!(runtime.collect_garbage(), 0);
    }
}

#[test]
fn host_held_method_survives_type_churn_and_reused_scope_slots() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    runtime
        .set_global("module", type_module("local-method.scripta"))
        .unwrap();
    run(
        &mut runtime,
        "function keep($module) { $r=resource(); require $module; return new Local(); }
         function discard($module) { $r=resource(); require $module; }
         $object=keep($module); $method=$object->read; $object=null;",
    )
    .unwrap();
    let method = runtime.get_global("method").unwrap().clone();
    runtime.set_global("method", Value::Null).unwrap();
    let resource = match runtime
        .invoke(method.clone(), &[], &mut std::io::sink())
        .unwrap()
    {
        Value::NativeObject(resource) => Rc::downgrade(&resource),
        _ => panic!("captured resource"),
    };
    for _ in 0..3 {
        run(
            &mut runtime,
            "for ($i=0; $i<1000; $i+=1) { discard($module); }",
        )
        .unwrap();
        runtime.collect_garbage();
        let value = runtime
            .invoke(method.clone(), &[], &mut std::io::sink())
            .unwrap();
        assert_eq!(value.type_name(), "Resource");
        let Value::NativeObject(value) = value else {
            unreachable!();
        };
        assert!(Rc::ptr_eq(&value, &resource.upgrade().unwrap()));
        assert_eq!(counts.created.get() - drops.get(), 1);
    }
    drop(method);
    runtime.collect_garbage();
    assert_eq!(drops.get(), counts.created.get());
    assert!(resource.upgrade().is_none());
}

#[test]
fn type_capture_churn_preserves_scope_boundaries_and_interruption_cleanup() {
    for limit in [4, 5] {
        let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
            max_scopes: Some(limit),
            ..Default::default()
        });
        runtime
            .set_global("module", type_module("local-class.scripta"))
            .unwrap();
        run(
            &mut runtime,
            "function factory($module) { $r=resource(); require $module; }",
        )
        .unwrap();
        let result = run(
            &mut runtime,
            "for ($i=0; $i<100; $i+=1) { factory($module); }",
        );
        if limit == 4 {
            assert!(result.unwrap_err().message.contains("scope limit"));
        } else {
            result.unwrap();
            assert_eq!(counts.created.get(), 100);
        }
        assert_eq!(counts.created.get(), drops.get());
        run(&mut runtime, "$recovered=true;").unwrap();
    }
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    runtime
        .set_global("module", type_module("local-interface.scripta"))
        .unwrap();
    let interrupted = flag.clone();
    runtime
        .register(
            "interrupt",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                move |_, _| {
                    interrupted.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    run(
        &mut runtime,
        "function factory($module) { $r=resource(); require $module; }",
    )
    .unwrap();
    let error = run(
        &mut runtime,
        "for ($i=0; $i<2000; $i+=1) { factory($module); if ($i===300) interrupt(); }",
    )
    .unwrap_err();
    assert!(error.message.contains("interrupted"), "{error:?}");
    assert_eq!(counts.created.get(), 301);
    assert_eq!(counts.created.get(), drops.get());
    flag.store(false, std::sync::atomic::Ordering::Relaxed);
    run(&mut runtime, "$recovered=true;").unwrap();
}

#[test]
fn garbage_is_collected_before_a_scope_limit_rejects_new_work() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        max_scopes: Some(12),
        ..Default::default()
    });
    run(&mut runtime, &format!("{FACTORY} {CHURN} churn();")).unwrap();
    assert_eq!(counts.created.get(), 2000);
    assert_eq!(drops.get(), 2000);
}

#[test]
fn host_held_nested_values_preserve_captures_without_explicit_root_registration() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(
        &mut runtime,
        &format!(r#"{FACTORY} {CHURN} $box=["items" => [factory()]];"#),
    )
    .unwrap();
    let held = runtime.get_global("box").unwrap().clone();
    run(&mut runtime, "$box=null; churn();").unwrap();
    runtime.collect_garbage();
    assert_eq!(counts.created.get() - drops.get(), 1);
    runtime.set_global("restored", held.clone()).unwrap();
    run(
        &mut runtime,
        r#"$answer=$restored["items"][0](); $restored=null;"#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(43)));
    drop(held);
    assert!(runtime.collect_garbage() > 0);
    assert_eq!(counts.created.get(), drops.get());
    assert_eq!(runtime.collect_garbage(), 0);
}

#[test]
fn evaluator_temporaries_and_suspended_callers_survive_collection() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(
        &mut runtime,
        &format!(
            r#"
        {FACTORY} {CHURN}
        function apply($f, $ignored) {{ return $f(); }}
        function outer() {{
            $r=resource(); $unused=function () {{}}; $unused=null;
            churn(); return $r;
        }}
        $answer=apply(factory(), churn());
        $resource=outer();
    "#
        ),
    )
    .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(43)));
    assert!(matches!(
        runtime.get_global("resource"),
        Some(Value::NativeObject(_))
    ));
    assert_eq!(counts.created.get() - drops.get(), 1);
    run(&mut runtime, "$resource=null;").unwrap();
    assert_eq!(counts.created.get(), drops.get());
}

#[test]
fn recursive_cycles_are_preserved_when_reachable_and_collected_when_discarded() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(
        &mut runtime,
        &format!(
            r#"
        {FACTORY} {CHURN}
        function recursive() {{
            $r=resource();
            $f=function ($n) {{ if ($n===0) return 0; return 1+$f($n-1); }};
            return $f;
        }}
        $keep=recursive(); churn(); $answer=$keep(5);
    "#
        ),
    )
    .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(5)));
    assert_eq!(counts.created.get() - drops.get(), 1);
    run(&mut runtime, "$keep=null;").unwrap();
    assert_eq!(counts.created.get(), drops.get());
}

#[test]
fn shared_lexical_parents_and_mutual_recursion_survive_collection() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(
        &mut runtime,
        &format!(
            r#"
        {FACTORY} {CHURN}
        function pair() {{
            $r=resource(); $n=0;
            function even($n) {{ if ($n===0) return true; return odd($n-1); }}
            function odd($n) {{ if ($n===0) return false; return even($n-1); }}
            return [function () {{ $n+=1; return $n; }},
                    function () {{ return $n; }}, even];
        }}
        $pair=pair(); churn(); $first=$pair[0](); $second=$pair[1](); $even=$pair[2](8);
    "#
        ),
    )
    .unwrap();
    assert_eq!(runtime.get_global("first"), Some(&Value::Int(1)));
    assert_eq!(runtime.get_global("second"), Some(&Value::Int(1)));
    assert_eq!(runtime.get_global("even"), Some(&Value::Bool(true)));
    assert_eq!(counts.created.get() - drops.get(), 1);
    run(&mut runtime, "$pair=null;").unwrap();
    assert_eq!(counts.created.get(), drops.get());
}

#[test]
fn failed_executions_collect_unreachable_captured_resources() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        instruction_limit: Some(100),
        ..Default::default()
    });
    run(
        &mut runtime,
        r#"
        function thrown() { $r=resource(); $f=function () {}; throw "failure"; }
        function exhausted() { $r=resource(); $f=function () {}; while (true) {} }
    "#,
    )
    .unwrap();
    for _ in 0..100 {
        assert!(run(&mut runtime, "thrown();").is_err());
        assert!(
            run(&mut runtime, "exhausted();")
                .unwrap_err()
                .message
                .contains("instruction limit")
        );
        assert_eq!(counts.created.get(), drops.get());
    }
}

#[test]
fn foreign_runtime_callables_are_external_roots_but_never_trace_foreign_scope_ids() {
    let (mut first, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(&mut first, &format!("{FACTORY} $f=factory();")).unwrap();
    let mut second = Runtime::new();
    second
        .set_global("foreign", first.get_global("f").unwrap().clone())
        .unwrap();
    run(&mut first, "$f=null;").unwrap();
    second.collect_garbage();
    first.collect_garbage();
    assert_eq!(drops.get(), 0);
    assert!(
        run(&mut second, "$foreign();")
            .unwrap_err()
            .message
            .contains("different runtime")
    );
    drop(second);
    first.collect_garbage();
    assert_eq!(counts.created.get(), drops.get());
}

#[test]
fn callbacks_hidden_inside_native_functions_are_conservative_roots() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(&mut runtime, &format!("{FACTORY} $f=factory();")).unwrap();
    let held = runtime.get_global("f").unwrap().clone();
    let hidden = Value::native_function(
        "hidden",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Any),
            move |_, _| Ok(held.clone()),
        ),
    );
    runtime.set_global("hidden", hidden).unwrap();
    run(&mut runtime, "$f=null;").unwrap();
    assert_eq!(drops.get(), 0);
    run(&mut runtime, "$answer=$hidden()(); $hidden=null;").unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(43)));
    assert_eq!(counts.created.get(), drops.get());
}

#[test]
fn opaque_native_objects_preserve_callbacks_without_running_property_code_during_collection() {
    #[derive(Debug)]
    struct Holder {
        callback: Value,
        reads: Rc<Cell<usize>>,
    }
    impl NativeObject for Holder {
        fn type_name(&self) -> &str {
            "Holder"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            self.reads.set(self.reads.get() + 1);
            Ok(self.callback.clone())
        }
    }
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions::default());
    run(&mut runtime, &format!("{FACTORY} $f=factory();")).unwrap();
    let reads = Rc::new(Cell::new(0));
    runtime
        .set_global(
            "holder",
            Value::NativeObject(Rc::new(Holder {
                callback: runtime.get_global("f").unwrap().clone(),
                reads: reads.clone(),
            })),
        )
        .unwrap();
    run(&mut runtime, "$f=null;").unwrap();
    runtime.collect_garbage();
    assert_eq!(reads.get(), 0);
    assert_eq!(drops.get(), 0);
    run(
        &mut runtime,
        "$saved=$holder->callback; $answer=$saved(); $saved=null; $holder=null;",
    )
    .unwrap();
    assert_eq!(runtime.get_global("answer"), Some(&Value::Int(43)));
    assert_eq!(reads.get(), 1);
    assert_eq!(counts.created.get(), drops.get());
}
