use scr_ast::Program;
use scr_runtime::{Runtime, RuntimeError, RuntimeOptions, Value};
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn parse(source: &str) -> Program {
    scr_parser::parse(&scr_lexer::lex(&format!("<?scr {source}")).unwrap()).unwrap()
}

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, RuntimeError> {
    runtime.execute(&parse(source), &mut Vec::new())
}

#[test]
fn forward_calls_mutual_recursion_and_source_order() {
    let mut runtime = Runtime::new();
    let mut output = Vec::new();
    runtime
        .execute(
            &parse(
                r#"
        echo "a"; $answer = odd(9); echo "b";
        function odd($n: int): bool { if ($n === 0) return false; return even($n - 1); }
        echo "c";
        function even($n: int): bool { if ($n === 0) return true; return odd($n - 1); }
        function unused() { echo "unreachable"; }
    "#,
            ),
            &mut output,
        )
        .unwrap();
    assert_eq!(output, b"abc");
    assert_eq!(runtime.get_global("answer"), Some(&Value::Bool(true)));
}

#[test]
fn functions_isolate_globals_and_caller_locals() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        "$x = 1; $result = write(9); function write($x) { $local = 4; $x += $local; return $x; }",
    )
    .unwrap();
    assert_eq!(runtime.get_global("x"), Some(&Value::Int(1)));
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(13)));
    assert!(runtime.get_global("local").is_none());
    for source in [
        "$global = 3; function read() { return $global; } read();",
        "function caller() { $secret = 3; return read(); } function read() { return $secret; } caller();",
        "$x = 1; function change() { $x += 1; } change();",
    ] {
        let error = run(&mut Runtime::new(), source).unwrap_err();
        assert!(error.message.contains("undefined variable"), "{error}");
    }
}

#[test]
fn arguments_are_left_to_right_deep_values() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        $i = 0; $args = pair($i += 1, $i += 1);
        function pair($a, $b) { return [$a, $b]; }
        $original = [["a" => [1]]]; $copy = mutate($original);
        function mutate($data: list): list { $data[0]["a"][0] = 99; return $data; }
        $before = $original[0]["a"][0]; $after = $copy[0]["a"][0];
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("args"),
        Some(&Value::List(vec![Value::Int(1), Value::Int(2)]))
    );
    assert_eq!(runtime.get_global("before"), Some(&Value::Int(1)));
    assert_eq!(runtime.get_global("after"), Some(&Value::Int(99)));
}

#[test]
fn nested_loops_early_returns_and_null_returns() {
    let mut runtime = Runtime::new();
    run(&mut runtime, r#"
        $sum = 0;
        for ($i = 0; $i < 3; $i += 1) {
            foreach ([1, 2, 3] as $n) {
                if ($n === 2) continue;
                while (true) { $sum += $n; break; }
                if ($n === 3) break;
            }
        }
        $early = early();
        function early(): int { for (;;) { foreach ([7] as $n) { try { return $n; } catch ($e) { return 0; } } } }
        function empty() {} function bare() { return; }
        $empty = empty(); $bare = bare();
    "#).unwrap();
    assert_eq!(runtime.get_global("sum"), Some(&Value::Int(12)));
    assert_eq!(runtime.get_global("early"), Some(&Value::Int(7)));
    assert_eq!(runtime.get_global("empty"), Some(&Value::Null));
    assert_eq!(runtime.get_global("bare"), Some(&Value::Null));
}

#[test]
fn type_errors_are_clear_catchable_and_preserve_traces() {
    for source in [
        "function f($n: int) {} f(true);",
        "function f($n) {} f();",
        "function f(): int { return false; } f();",
        "function f(): int {} f();",
        "function f(): void { return 1; } f();",
    ] {
        let error = run(&mut Runtime::new(), source).unwrap_err();
        assert!(error.message.starts_with("TypeError:"), "{error}");
        assert_eq!(error.trace.last().unwrap().function, "f");
        let mut runtime = Runtime::new();
        let wrapped = format!(
            "$caught = false; try {{ {source} }} catch (TypeError $e) {{ $caught = true; }}"
        );
        run(&mut runtime, &wrapped).unwrap();
        assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
        assert!(runtime.get_global("e").is_none());
    }
    // A TypeError must not match the string catch simply because its binding
    // exposes a string message. Untyped and Error catches do match it.
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        $caught = false;
        try { try { f("x"); } catch (string $e) { throw "wrong catch"; } }
        catch (Error $e) { $caught = true; }
        function f($x: int) {}
    "#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
}

#[test]
fn throws_unwind_calls_and_rethrows_reach_outer_catches() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        r#"
        $caught = "";
        try { try { outer(); } catch (int $e) {} catch (string $e) { throw $e; } }
        catch (string $e) { $caught = $e; }
        function outer() { return inner(); }
        function inner() { throw "problem"; }
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("caught"),
        Some(&Value::String("problem".into()))
    );
    assert!(runtime.get_global("e").is_none());
    assert!(
        run(&mut runtime, "throw 42;")
            .unwrap_err()
            .message
            .contains("uncaught")
    );
}

#[test]
fn exit_unwinds_nested_calls_loops_and_catches() {
    for code in [0, 7, 255] {
        let mut runtime = Runtime::new();
        let mut output = Vec::new();
        let source = format!(
            r#"
            while (true) {{ try {{ stop(); }} catch ($e) {{ echo "caught"; }} }}
            echo "after";
            function stop() {{ foreach ([1] as $x) {{ exit({code}); }} }}
        "#
        );
        assert_eq!(runtime.execute(&parse(&source), &mut output).unwrap(), code);
        assert!(output.is_empty());
        run(&mut runtime, "$ok = true;").unwrap();
    }
}

#[test]
fn invalid_control_flow_is_rejected_by_parser_and_runtime() {
    for source in [
        "break;",
        "continue;",
        "return 1;",
        "while (true) { function f() { break; } }",
        "function f() { continue; }",
    ] {
        let tokens = scr_lexer::lex(&format!("<?scr {source}")).unwrap();
        assert!(scr_parser::parse(&tokens).is_err(), "{source}");
    }
    // Embedders can supply ASTs directly, bypassing parser validation.
    for kind in [
        scr_ast::StatementKind::Break,
        scr_ast::StatementKind::Continue,
        scr_ast::StatementKind::Return(None),
    ] {
        let program = Program {
            statements: vec![scr_ast::Statement {
                kind,
                span: Default::default(),
            }],
        };
        let error = Runtime::new()
            .execute(&program, &mut Vec::new())
            .unwrap_err();
        assert!(error.message.contains("invalid control flow"));
    }
}

#[test]
fn recursion_limit_is_configurable_exact_and_cleans_up() {
    for limit in [0, 1, 4, 16, 128, usize::MAX] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            recursion_limit: limit,
            ..Default::default()
        });
        let error = run(
            &mut runtime,
            "function recurse() { return recurse(); } recurse();",
        )
        .unwrap_err();
        assert!(error.message.contains("maximum call depth"));
        assert_eq!(
            error.trace.len(),
            limit.min(scr_runtime::MAX_RECURSION_LIMIT) + 1
        );
        run(&mut runtime, "$ok = 1;").unwrap();
    }
    let source = "function count($n) { if ($n === 0) return 0; return count($n - 1); } count(3);";
    assert!(
        run(
            &mut Runtime::with_options(RuntimeOptions {
                recursion_limit: 4,
                ..Default::default()
            }),
            source
        )
        .is_ok()
    );
    assert!(
        run(
            &mut Runtime::with_options(RuntimeOptions {
                recursion_limit: 3,
                ..Default::default()
            }),
            source
        )
        .is_err()
    );
}

#[test]
fn empty_loop_back_edges_obey_execution_budget() {
    for source in [
        "while (true) {}",
        "for (;;) {}",
        "foreach ([1, 2, 3] as $n) {}",
    ] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            instruction_limit: Some(2),
            ..Default::default()
        });
        let error = run(&mut runtime, source).unwrap_err();
        assert!(error.message.contains("instruction limit"));
        // The budget resets at execute, after all scopes have unwound.
        run(&mut runtime, "$ok = 1;").unwrap();
    }
}

#[test]
fn host_can_interrupt_running_loops_without_script_catches_swallowing_it() {
    struct CancellingWriter(Arc<AtomicBool>);
    impl Write for CancellingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.store(true, Ordering::Relaxed);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for body in [
        "while (true) {}",
        "for (;;) {}",
        "foreach ([1, 2] as $v) {}",
    ] {
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        });
        let program = parse(&format!(
            r#"function f() {{ echo "start"; {body} }} try {{ f(); }} catch (Error $e) {{}}"#
        ));
        let error = runtime
            .execute(&program, &mut CancellingWriter(flag.clone()))
            .unwrap_err();
        assert!(error.message.contains("interrupted"));
        assert_eq!(error.trace.last().unwrap().function, "f");
        flag.store(false, Ordering::Relaxed);
        run(&mut runtime, "$ok = true;").unwrap();
    }
}

#[test]
fn conditional_declarations_are_not_hoisted() {
    let mut runtime = Runtime::new();
    run(
        &mut runtime,
        "if (false) { function absent() {} } function present() {}",
    )
    .unwrap();
    assert!(
        run(&mut runtime, "absent();")
            .unwrap_err()
            .message
            .contains("undefined function")
    );
    assert!(run(&mut runtime, "present();").is_ok());
}
