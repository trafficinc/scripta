use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use std::{fs, path::Path};

fn runtime(dir: &Path) -> Runtime {
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            script_path: Some(dir.join("main.scr")),
            working_directory: dir.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register(&mut rt).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) -> Result<String, RuntimeError> {
    let mut out = Vec::new();
    rt.execute_source(&format!("<?scr {source}"), &mut out)?;
    Ok(String::from_utf8(out).unwrap())
}
fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), format!("<?scr {body}")).unwrap();
}

#[test]
fn aliases_isolate_functions_variables_and_caller_names_but_keep_native_functions() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.scr",
        "$name='a'; function normalize() { return String::upper($name); } function greet() { return normalize(); }",
    );
    write(
        dir.path(),
        "b.scr",
        "$name='b'; function normalize() { return String::upper($name); } function greet() { return normalize(); }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(run(&mut rt, "$name='caller'; function normalize() { return 'caller'; } require_once 'a.scr' as A; require_once 'b.scr' as B; echo A::greet() . B::greet() . normalize() . $name;").unwrap(), "ABcallercaller");
    assert!(
        run(&mut rt, "greet();")
            .unwrap_err()
            .message
            .contains("undefined function greet")
    );
    assert!(
        run(&mut rt, "A::missing();")
            .unwrap_err()
            .message
            .contains("module alias A has no function missing")
    );
    write(dir.path(), "read-caller.scr", "echo $name;");
    assert!(
        run(&mut rt, "require 'read-caller.scr' as Read;")
            .unwrap_err()
            .message
            .contains("undefined variable $name")
    );
    write(dir.path(), "call-caller.scr", "normalize();");
    assert!(
        run(&mut rt, "require 'call-caller.scr' as Call;")
            .unwrap_err()
            .message
            .contains("undefined function normalize")
    );
    assert_eq!(run(&mut rt, "echo A::greet();").unwrap(), "A");
}

#[test]
fn once_aliases_share_state_and_require_reexecutes_in_a_fresh_instance() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "counter.scr",
        "echo 'init;'; $n=0; function next() { $n+=1; return str($n); }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(run(&mut rt, "require_once 'counter.scr' as Counter; echo Counter::next(); require_once './counter.scr' as Other; echo Other::next(); require_once 'counter.scr' as Counter; echo Counter::next();").unwrap(), "init;123");
    assert_eq!(run(&mut rt, "require 'counter.scr' as Counter; echo Counter::next() . Other::next(); require_once 'counter.scr' as Latest; echo Latest::next();").unwrap(), "init;142");
    rt.collect_garbage();
    assert_eq!(
        run(&mut rt, "echo Counter::next() . Other::next();").unwrap(),
        "35"
    );
    // Legacy loading remains independent and shares the caller's scope.
    assert_eq!(
        run(
            &mut rt,
            "require_once 'counter.scr'; require_once 'counter.scr'; echo str($n);"
        )
        .unwrap(),
        "init;0"
    );
    assert!(
        run(&mut rt, "require 'counter.scr';")
            .unwrap_err()
            .message
            .contains("already declared")
    );
}

#[test]
fn alias_conflicts_are_catchable_and_do_not_execute_or_replace_the_module() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.scr", "function name() { return 'a'; }");
    write(
        dir.path(),
        "b.scr",
        "echo 'should not run'; function name() { return 'b'; }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(run(&mut rt, "require_once 'a.scr' as Lib; try { require 'b.scr' as Lib; } catch (ModuleAliasError $e) { echo 'conflict;'; } echo Lib::name();").unwrap(), "conflict;a");
    assert_eq!(run(&mut rt, "try { require 'b.scr' as String; } catch (ModuleAliasError $e) { echo String::upper('reserved'); }").unwrap(), "RESERVED");
    let error = rt
        .register(
            "Lib::new",
            scr_runtime::NativeFunction::new(
                scr_runtime::NativeSignature::new(vec![], scr_runtime::NativeType::Null),
                |_, _| Ok(Value::Null),
            ),
        )
        .unwrap_err();
    assert_eq!(error.kind, "RegistryError");
    assert_eq!(run(&mut rt, "echo Lib::name();").unwrap(), "a");
}

#[test]
fn local_includes_once_tracking_and_aliases_follow_the_enclosing_module() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "helper.scr",
        "function helper() { return 'helper'; }",
    );
    write(
        dir.path(),
        "inner.scr",
        "function value() { return 'inner'; }",
    );
    write(
        dir.path(),
        "outer.scr",
        "require_once 'helper.scr'; require_once './helper.scr'; require_once 'inner.scr' as Inner; function value() { return helper() . Inner::value(); }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(run(&mut rt, "require 'outer.scr' as First; require 'outer.scr' as Second; require_once 'helper.scr'; echo First::value() . Second::value() . helper();").unwrap(), "helperinnerhelperinnerhelper");
    assert!(
        run(&mut rt, "Inner::value();")
            .unwrap_err()
            .message
            .contains("undefined function Inner::value")
    );
    assert_eq!(run(&mut rt, "function factory() { require_once 'outer.scr' as Local; return function () { return Local::value(); }; } $f=factory();").unwrap(), "");
    rt.collect_garbage();
    assert_eq!(run(&mut rt, "echo $f();").unwrap(), "helperinner");
}

#[test]
fn module_cache_and_old_alias_edges_survive_collection_even_without_functions() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "empty.scr", "echo 'loaded';");
    write(
        dir.path(),
        "lib.scr",
        "$n=0; function tick() { $n+=1; return str($n); } function callback() { return function () { return tick(); }; }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(run(&mut rt, "{ require_once 'empty.scr' as Empty; } require_once 'lib.scr' as Old; $f=Old::callback(); require 'lib.scr' as New; echo Old::tick();").unwrap(), "loaded1");
    for _ in 0..3 {
        rt.collect_garbage();
    }
    assert_eq!(
        run(
            &mut rt,
            "require_once 'empty.scr' as Again; echo Old::tick() . New::tick() . $f();"
        )
        .unwrap(),
        "213"
    );
    // Exercise replaced cache roots: Old is reachable only through the alias graph.
    for _ in 0..3 {
        rt.collect_garbage();
    }
    assert_eq!(
        run(&mut rt, "echo Old::tick() . New::tick();").unwrap(),
        "42"
    );
}

#[test]
fn failed_initialization_restores_scope_preserves_existing_alias_and_allows_retry() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "lib.scr", "function name() { return 'old'; }");
    let mut rt = runtime(dir.path());
    run(&mut rt, "require_once 'lib.scr' as Lib;").unwrap();
    write(
        dir.path(),
        "lib.scr",
        "$caller='changed'; function name() { return 'partial'; } throw 'failed';",
    );
    assert_eq!(run(&mut rt, "$caller='safe'; try { require 'lib.scr' as Lib; } catch ($e) { echo 'caught;'; } echo Lib::name() . $caller;").unwrap(), "caught;oldsafe");
    write(dir.path(), "lib.scr", "function name() { return 'new'; }");
    assert_eq!(
        run(&mut rt, "require 'lib.scr' as Lib; echo Lib::name();").unwrap(),
        "new"
    );
    write(
        dir.path(),
        "retry.scr",
        "function value() { return 'partial'; } throw 'failed';",
    );
    run(
        &mut rt,
        "try { require_once 'retry.scr' as Retry; } catch ($e) {}",
    )
    .unwrap();
    write(
        dir.path(),
        "retry.scr",
        "function value() { return 'retry'; }",
    );
    assert_eq!(
        run(
            &mut rt,
            "require_once 'retry.scr' as Retry; echo Retry::value();"
        )
        .unwrap(),
        "retry"
    );
    assert_eq!(run(&mut rt, "echo Lib::name();").unwrap(), "new");
}

#[test]
fn cycles_source_errors_and_call_traces_retain_files_and_do_not_poison_runtime() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "cycle.scr",
        "require_once 'cycle.scr' as Cycle;",
    );
    write(dir.path(), "syntax.scr", "function broken(;");
    write(
        dir.path(),
        "trace.scr",
        "function fail() { return $missing; }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(run(&mut rt, "try { require_once 'cycle.scr' as Cycle; } catch (CircularIncludeError $e) { echo 'cycle'; }").unwrap(), "cycle");
    let error = run(&mut rt, "require_once 'syntax.scr' as Syntax;").unwrap_err();
    assert_eq!(error.code, "E0201");
    assert!(error.source.unwrap().path.ends_with("syntax.scr"));
    run(&mut rt, "require_once 'trace.scr' as Trace;").unwrap();
    let error = run(&mut rt, "Trace::fail();").unwrap_err();
    assert!(error.source.as_ref().unwrap().path.ends_with("trace.scr"));
    assert!(error.trace.iter().any(|frame| frame.function == "fail"
        && frame.source.as_ref().unwrap().path.ends_with("main.scr")));
    assert_eq!(run(&mut rt, "echo 'recovered';").unwrap(), "recovered");
}

#[test]
fn cached_aliases_still_require_permissions_and_host_policy_and_scopes_are_budgeted() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "lib.scr", "function value() { return 'ok'; }");
    let mut rt = runtime(dir.path());
    run(&mut rt, "require_once 'lib.scr' as Lib;").unwrap();
    rt.set_module_policy(|_| {
        Err(scr_runtime::NativeError::new(
            "PermissionError",
            "denied by host",
        ))
    });
    assert!(
        run(&mut rt, "require_once 'lib.scr' as Other;")
            .unwrap_err()
            .message
            .contains("denied by host")
    );
    assert_eq!(run(&mut rt, "echo Lib::value();").unwrap(), "ok");
    let mut options = RuntimeOptions::default();
    options.context.script_path = Some(dir.path().join("main.scr"));
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.read");
    let mut denied = Runtime::with_options(options.clone());
    assert!(
        run(&mut denied, "require_once 'lib.scr' as Lib;")
            .unwrap_err()
            .message
            .contains("permission denied")
    );
    options
        .context
        .permissions
        .allowed
        .insert("filesystem.read".into());
    options.max_scopes = Some(1);
    let mut limited = Runtime::with_options(options);
    assert!(
        run(&mut limited, "require_once 'lib.scr' as Lib;")
            .unwrap_err()
            .message
            .contains("scope limit")
    );
    assert_eq!(run(&mut limited, "echo 'recovered';").unwrap(), "recovered");
}

#[test]
fn alias_formatting_preserves_comments_and_execution_and_check_does_not_load_files() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "lib.scr",
        "function greet($name='Ada') { return $name; }",
    );
    let source = "<?scr require_once /* path */ ('lib.scr') as /* alias */ Helpers;echo Helpers :: greet ( );";
    scr_cli::pipeline::check(source).unwrap();
    scr_cli::pipeline::check("<?scr require_once 'nonexistent' as Missing; Missing::unknown();")
        .unwrap();
    let formatted = scr_cli::formatter::format(source).unwrap();
    assert!(formatted.contains("as /* alias */ Helpers"), "{formatted}");
    assert!(formatted.contains("Helpers::greet()"));
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    for source in [source, &formatted] {
        let mut rt = runtime(dir.path());
        let mut output = Vec::new();
        rt.execute_source(source, &mut output).unwrap();
        assert_eq!(output, b"Ada");
    }
}

#[test]
fn repeated_require_reclaims_old_instances_and_fatal_failures_restore_scope() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "lib.scr", "function value() { return 'ok'; }");
    let mut rt = Runtime::with_options(RuntimeOptions {
        max_scopes: Some(8),
        context: ContextOptions {
            script_path: Some(dir.path().join("main.scr")),
            ..Default::default()
        },
        ..Default::default()
    });
    // Each fresh module contains a function/environment cycle. Scope pressure
    // must reclaim replaced instances while retaining the current binding/cache.
    assert_eq!(run(&mut rt, "for ($i=0; $i<100; $i+=1) { require 'lib.scr' as Lib; } require_once 'lib.scr' as Current; echo Current::value();").unwrap(), "ok");
    write(dir.path(), "steps.scr", "while (true) {}");
    let mut limited = Runtime::with_options(RuntimeOptions {
        instruction_limit: Some(20),
        context: ContextOptions {
            script_path: Some(dir.path().join("main.scr")),
            ..Default::default()
        },
        ..Default::default()
    });
    assert!(
        run(&mut limited, "require_once 'steps.scr' as Steps;")
            .unwrap_err()
            .message
            .contains("instruction limit")
    );
    write(
        dir.path(),
        "steps.scr",
        "function value() { return 'retry'; }",
    );
    assert_eq!(
        run(
            &mut limited,
            "require_once 'steps.scr' as Steps; echo Steps::value();"
        )
        .unwrap(),
        "retry"
    );
    write(
        dir.path(),
        "exit.scr",
        "function value() { return 'partial'; } exit 7;",
    );
    assert_eq!(
        rt.execute_source("<?scr require_once 'exit.scr' as Exiting;", &mut Vec::new())
            .unwrap(),
        7
    );
    assert!(
        run(&mut rt, "Exiting::value();")
            .unwrap_err()
            .message
            .contains("undefined function")
    );
    write(
        dir.path(),
        "exit.scr",
        "function value() { return 'complete'; }",
    );
    assert_eq!(
        run(
            &mut rt,
            "require_once 'exit.scr' as Exiting; echo Exiting::value();"
        )
        .unwrap(),
        "complete"
    );
}

#[test]
fn module_entry_once_identity_is_retained_for_later_calls_and_canonical_aliases() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "self.scr",
        "echo 'init'; function check() { require_once 'self.scr'; return 'ok'; }",
    );
    let mut rt = runtime(dir.path());
    assert_eq!(
        run(
            &mut rt,
            "require_once 'self.scr' as Lib; echo Lib::check();"
        )
        .unwrap(),
        "initok"
    );
    rt.collect_garbage();
    assert_eq!(run(&mut rt, "echo Lib::check();").unwrap(), "ok");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("self.scr"), dir.path().join("link.scr"))
            .unwrap();
        assert_eq!(
            run(
                &mut rt,
                "require_once 'link.scr' as Lib; echo Lib::check();"
            )
            .unwrap(),
            "ok"
        );
    }
}
