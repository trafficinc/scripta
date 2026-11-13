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
fn discarded_closures_retain_captures_until_runtime_drop() {
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
        // This intentionally documents current arena retention, not GC behavior.
        assert_eq!(drops.get(), 0);
    }
    drop(runtime);
    assert_eq!(drops.get(), counts.created.get());
}

#[test]
fn scope_limit_stops_retention_and_unwinds_failed_calls() {
    let (mut runtime, counts, drops) = tracked_runtime(RuntimeOptions {
        max_scopes: Some(16),
        ..Default::default()
    });
    run(
        &mut runtime,
        "function factory() { $r=resource(); return function () { return $r; }; }",
    )
    .unwrap();
    let error = run(
        &mut runtime,
        "for ($i=0; $i<100; $i+=1) { $f=factory(); $f=null; }",
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
                "for ($i=0; $i<100; $i+=1) { $f=factory(); $f=null; }"
            )
            .unwrap_err()
            .message
            .contains("scope limit")
        );
        assert_eq!(counts.created.get(), retained);
        run(&mut runtime, "$recovered = true;").unwrap();
    }
    drop(runtime);
    assert_eq!(drops.get(), retained);
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
        assert_eq!(drops.get(), 0);
        drop(runtime);
        assert_eq!(drops.get(), 21);
        assert_eq!(Rc::strong_count(&counts), 1);
        assert_eq!(Rc::strong_count(&drops), 1);
    }
}
