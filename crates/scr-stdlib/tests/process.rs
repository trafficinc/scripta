use scr_runtime::{NativeObject, Runtime, RuntimeError, RuntimeOptions, Value};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn run(runtime: &mut Runtime, source: &str) -> Result<u8, RuntimeError> {
    let source = format!("<?scr {source}");
    runtime.execute(
        &scr_parser::parse(&scr_lexer::lex(&source).unwrap()).unwrap(),
        &mut Vec::new(),
    )
}
fn runtime() -> Runtime {
    configured_runtime(Default::default(), Default::default())
}
fn configured_runtime(
    config: scr_stdlib::process::ProcessConfig,
    options: RuntimeOptions,
) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register_with_options(
        &mut runtime,
        scr_stdlib::StdlibOptions {
            process: config,
            ..Default::default()
        },
    )
    .unwrap();
    runtime
        .set_global(
            "program",
            Value::String(std::env::current_exe().unwrap().to_str().unwrap().into()),
        )
        .unwrap();
    runtime
        .set_global(
            "arguments",
            Value::list(
                ["--exact", "child_fixture", "--nocapture"]
                    .map(|value| Value::String(value.into()))
                    .to_vec(),
            ),
        )
        .unwrap();
    runtime
}
fn property(runtime: &Runtime, object: &str, name: &str) -> Value {
    let Some(Value::NativeObject(object)) = runtime.get_global(object) else {
        panic!("expected native object");
    };
    object.get_property(name).unwrap()
}
fn error(runtime: &mut Runtime, source: &str, expected: &str) {
    let failure = run(runtime, source).unwrap_err();
    assert!(
        failure.message.starts_with(&format!("{expected}:")),
        "{source}: {failure}"
    );
}
fn gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut system = sysinfo::System::new();
        let pid = sysinfo::Pid::from_u32(pid);
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        if system.process(pid).is_none() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn pid(runtime: &Runtime, object: &str) -> u32 {
    let Value::Int(pid) = property(runtime, object, "pid") else {
        panic!();
    };
    pid as u32
}

// The test executable is a portable child fixture. Invoke only this test, with
// fixture mode in its private child environment. No shell or OS utility needed.
#[test]
fn child_fixture() {
    let Ok(mode) = std::env::var("SCR_PROCESS_FIXTURE") else {
        return;
    };
    match mode.as_str() {
        "interactive-response" => {
            std::io::stderr().write_all(b"ready").unwrap();
            let mut input = [0; 4];
            std::io::stdin().read_exact(&mut input).unwrap();
            std::io::stderr().write_all(&input).unwrap();
            let mut tail = Vec::new();
            std::io::stdin().read_to_end(&mut tail).unwrap();
            std::io::stdout().write_all(b"done").unwrap();
            std::process::exit(0);
        }
        "echo" => {
            print!("child stdout 世界");
            eprint!("child stderr");
        }
        "fail" => {
            eprint!("intentional failure");
            std::process::exit(17);
        }
        "environment" => {
            eprint!(
                "{}|{}|{}",
                std::env::var("SCR_CHILD_VALUE").unwrap_or_default(),
                std::env::var("SCR_CHILD_REMOVED").unwrap_or_else(|_| "absent".into()),
                // Windows can report an 8.3 spelling even when the requested
                // directory used a canonical extended-length path.
                fs::canonicalize(std::env::current_dir().unwrap())
                    .unwrap()
                    .display()
            );
        }
        "literal" => {
            let args: Vec<_> = std::env::args().collect();
            let index = args.iter().position(|arg| arg == "--skip").unwrap();
            eprint!("{}", args[index + 1]);
        }
        "duplex" => {
            for _ in 0..128 {
                std::io::stdout().write_all(&[b'o'; 4096]).unwrap();
                std::io::stderr().write_all(&[b'e'; 4096]).unwrap();
            }
        }
        "flood" => loop {
            if std::io::stdout().write_all(&[b'x'; 65536]).is_err() {
                break;
            }
        },
        "invalid" => {
            std::io::stdout().write_all(&[0xff]).unwrap();
        }
        "stdin-echo" => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            std::io::stderr().write_all(&input).unwrap();
            std::process::exit(0);
        }
        "write-before-input" => {
            // A parent that writes a stdin pipe before draining stdout/stderr
            // would deadlock here once all three pipe buffers fill.
            std::io::stdout()
                .write_all(&vec![b'o'; 512 * 1024])
                .unwrap();
            std::io::stderr()
                .write_all(&vec![b'e'; 512 * 1024])
                .unwrap();
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            std::io::stderr().write_all(&input).unwrap();
            std::process::exit(0);
        }
        "early-input-exit" => {
            let mut first = [0];
            std::io::stdin().read_exact(&mut first).unwrap();
            std::io::stderr().write_all(&first).unwrap();
            std::process::exit(17);
        }
        "skip-input" => std::process::exit(0),
        "exact-output" => {
            let count: usize = std::env::var("SCR_PROCESS_BYTES").unwrap().parse().unwrap();
            std::io::stderr().write_all(&vec![b'x'; count]).unwrap();
            std::process::exit(0);
        }
        "sleep" => {
            if let Ok(path) = std::env::var("SCR_CHILD_PID_FILE") {
                fs::write(path, std::process::id().to_string()).unwrap();
            }
            std::thread::sleep(Duration::from_secs(10));
        }
        "descendant" => {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "child_fixture", "--nocapture"])
                .env("SCR_PROCESS_FIXTURE", "late-marker")
                .spawn()
                .unwrap();
            if let Ok(path) = std::env::var("SCR_CHILD_PID_FILE") {
                fs::write(path, child.id().to_string()).unwrap();
            }
            // Parent exits before the descendant and before inherited streams close.
            let _ = child.try_wait();
        }
        "late-marker" => {
            std::thread::sleep(Duration::from_millis(750));
            if let Ok(path) = std::env::var("SCR_LATE_MARKER") {
                fs::write(path, "escaped").unwrap();
            }
        }
        _ => panic!("unknown fixture mode"),
    }
}

#[test]
fn run_returns_complete_readonly_result_and_exit_codes() {
    let mut runtime = runtime();
    run(
        &mut runtime,
        r#"Env::set("SCR_PROCESS_FIXTURE", "echo"); $result = Process::run($program, $arguments);"#,
    )
    .unwrap();
    assert_eq!(property(&runtime, "result", "exitCode"), Value::Int(0));
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
    assert!(
        matches!(property(&runtime, "result", "stdout"), Value::String(value) if value.contains("child stdout 世界"))
    );
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String("child stderr".into())
    );
    assert!(matches!(property(&runtime, "result", "durationMs"), Value::Int(value) if value >= 0));
    error(&mut runtime, "$result->success = false;", "PropertyError");
    run(
        &mut runtime,
        r#"Env::set("SCR_PROCESS_FIXTURE", "fail"); $result = Process::run($program, $arguments);"#,
    )
    .unwrap();
    assert_eq!(property(&runtime, "result", "exitCode"), Value::Int(17));
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(false));
}

#[test]
fn literal_arguments_cannot_become_shell_syntax() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = temp.path().join("injected");
    let literal = format!("space ; & | $(touch {}) \" ' > *", sentinel.display());
    let mut runtime = runtime();
    runtime
        .set_global("literal", Value::String(literal.clone()))
        .unwrap();
    run(&mut runtime, r#"Env::set("SCR_PROCESS_FIXTURE", "literal"); $result = Process::run($program, ["--exact", "child_fixture", "--nocapture", "--skip", $literal]);"#).unwrap();
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String(literal)
    );
    assert!(!sentinel.exists());
}

#[test]
fn working_directory_and_environment_overrides_are_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    runtime
        .set_global("cwd", Value::String(temp.path().to_str().unwrap().into()))
        .unwrap();
    run(&mut runtime, r#"Env::set("SCR_CHILD_VALUE", "runtime"); Env::set("SCR_CHILD_REMOVED", "remove me"); Env::set("SCR_PROCESS_FIXTURE", "environment"); $result = Process::run($program, $arguments, ["cwd" => $cwd, "env" => ["SCR_CHILD_VALUE" => "override", "SCR_CHILD_REMOVED" => null]]); $local = Env::get("SCR_CHILD_VALUE");"#).unwrap();
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String(format!(
            "override|absent|{}",
            fs::canonicalize(temp.path()).unwrap().display()
        ))
    );
    assert_eq!(
        runtime.get_global("local"),
        Some(&Value::String("runtime".into()))
    );
    error(
        &mut runtime,
        r#"Process::run($program, [], ["cwd" => "surely-missing-cwd"]);"#,
        "ProcessError",
    );
}

#[test]
fn simultaneous_stream_capture_cannot_deadlock_and_is_bounded() {
    let mut runtime = runtime();
    run(&mut runtime, r#"Env::set("SCR_PROCESS_FIXTURE", "duplex"); $result = Process::run($program, $arguments);"#).unwrap();
    assert!(
        matches!(property(&runtime, "result", "stdout"), Value::String(value) if value.len() >= 512 * 1024)
    );
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String("e".repeat(512 * 1024))
    );
    run(&mut runtime, r#"Env::set("SCR_PROCESS_FIXTURE", "flood");"#).unwrap();
    error(
        &mut runtime,
        r#"Process::run($program, $arguments, ["maxOutputBytes" => 1024]);"#,
        "OutputLimitError",
    );
    run(
        &mut runtime,
        r#"Env::set("SCR_PROCESS_FIXTURE", "invalid");"#,
    )
    .unwrap();
    error(
        &mut runtime,
        "Process::run($program, $arguments);",
        "EncodingError",
    );
}

#[test]
fn missing_commands_and_invalid_options_raise_typed_errors_before_launch() {
    let mut runtime = runtime();
    error(
        &mut runtime,
        r#"Process::run("scripta-command-that-does-not-exist-9e835a");"#,
        "CommandNotFoundError",
    );
    for source in [
        r#"Process::run("");"#,
        "Process::run();",
        "Process::run(1);",
        r#"Process::run($program, [1]);"#,
        r#"Process::run($program, "oops");"#,
        r#"Process::run($program, [], ["timeoutMs" => -1]);"#,
        r#"Process::run($program, [], ["maxOutputBytes" => 999999999]);"#,
        r#"Process::run($program, [], ["env" => ["BAD=NAME" => "x"]]);"#,
        r#"Process::run($program, [], ["env" => ["OK" => 1]]);"#,
        r#"Process::run($program, [], ["stdout" => "bogus"]);"#,
        r#"Process::run($program, [], ["typo" => true]);"#,
    ] {
        error(&mut runtime, source, "TypeError");
    }
    runtime
        .set_global("nul", Value::String("bad\0arg".into()))
        .unwrap();
    error(&mut runtime, "Process::run($program, [$nul]);", "TypeError");
    run(&mut runtime, r#"$caught = false; try { Process::run("scripta-command-that-does-not-exist-9e835a"); } catch (CommandNotFoundError $e) { $caught = true; }"#).unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
}

#[test]
fn spawned_handles_wait_poll_kill_and_drop_clean_up() {
    let mut runtime = runtime();
    run(&mut runtime, r#"Env::set("SCR_PROCESS_FIXTURE", "echo"); $child = Process::spawn($program, $arguments); $result = Process::wait($child); $polled = Process::poll($child); $running = $child->running;"#).unwrap();
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
    assert_eq!(property(&runtime, "polled", "exitCode"), Value::Int(0));
    assert_eq!(runtime.get_global("running"), Some(&Value::Bool(false)));
    run(&mut runtime, r#"Env::set("SCR_PROCESS_FIXTURE", "sleep"); $child = Process::spawn($program, $arguments); $poll = Process::poll($child);"#).unwrap();
    let child_pid = pid(&runtime, "child");
    assert_eq!(runtime.get_global("poll"), Some(&Value::Null));
    run(&mut runtime, "$result = Process::kill($child);").unwrap();
    assert!(gone(child_pid));
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(false));
    run(
        &mut runtime,
        "$child = Process::spawn($program, $arguments);",
    )
    .unwrap();
    let child_pid = pid(&runtime, "child");
    runtime.set_global("child", Value::Null).unwrap();
    assert!(gone(child_pid));
}

#[test]
fn timeouts_apply_to_runs_and_unattended_spawned_children() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pid");
    let mut runtime = runtime();
    runtime
        .set_global("pidFile", Value::String(pid_file.to_str().unwrap().into()))
        .unwrap();
    run(
        &mut runtime,
        r#"Env::set("SCR_PROCESS_FIXTURE", "sleep"); Env::set("SCR_CHILD_PID_FILE", $pidFile);"#,
    )
    .unwrap();
    let start = Instant::now();
    error(
        &mut runtime,
        r#"Process::run($program, $arguments, ["timeoutMs" => 300]);"#,
        "TimeoutError",
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    if let Ok(pid) = fs::read_to_string(&pid_file) {
        assert!(gone(pid.parse().unwrap()));
    }
    run(
        &mut runtime,
        r#"$child = Process::spawn($program, $arguments, ["timeoutMs" => 30]); sleep(100);"#,
    )
    .unwrap();
    error(&mut runtime, "Process::wait($child);", "TimeoutError");
    assert!(gone(pid(&runtime, "child")));
}

#[test]
fn cancellation_kills_and_reaps_running_child_before_returning() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pid");
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
        .set_global(
            "program",
            Value::String(std::env::current_exe().unwrap().to_str().unwrap().into()),
        )
        .unwrap();
    runtime
        .set_global("pidFile", Value::String(pid_file.to_str().unwrap().into()))
        .unwrap();
    let observer = pid_file.clone();
    let cancel = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !observer.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        flag.store(true, Ordering::Relaxed);
    });
    let failure = run(&mut runtime, r#"Env::set("SCR_PROCESS_FIXTURE", "sleep"); Env::set("SCR_CHILD_PID_FILE", $pidFile); try { Process::run($program, ["--exact", "child_fixture", "--nocapture"]); } catch (Error $e) {}"#).unwrap_err();
    cancel.join().unwrap();
    assert!(failure.message.contains("interrupted"));
    assert!(gone(fs::read_to_string(pid_file).unwrap().parse().unwrap()));
}

#[test]
fn descendants_do_not_keep_capture_open_after_leader_exits() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = runtime();
    let marker = temp.path().join("late");
    runtime
        .set_global("marker", Value::String(marker.to_str().unwrap().into()))
        .unwrap();
    run(
        &mut runtime,
        r#"Env::set("SCR_PROCESS_FIXTURE", "descendant"); Env::set("SCR_LATE_MARKER", $marker);"#,
    )
    .unwrap();
    for options in [
        "['timeoutMs'=>2000]",
        "['timeoutMs'=>2000,'input'=>Bytes::encodeUtf8('unread descendant input'),'outputType'=>'bytes']",
    ] {
        run(
            &mut runtime,
            &format!("$result=Process::run($program,$arguments,{options});"),
        )
        .unwrap();
        assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
        std::thread::sleep(Duration::from_millis(900));
        assert!(!marker.exists());
    }
}

#[test]
fn shell_is_explicit_and_separately_permission_gated() {
    let mut denied = runtime();
    error(
        &mut denied,
        r#"Process::shell("echo shell");"#,
        "PermissionError",
    );
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .insert("process.shell".into());
    let mut allowed = Runtime::with_options(options);
    scr_stdlib::register(&mut allowed).unwrap();
    run(
        &mut allowed,
        r#"$result = Process::shell("echo explicit-shell");"#,
    )
    .unwrap();
    assert!(
        matches!(property(&allowed, "result", "stdout"), Value::String(value) if value.contains("explicit-shell"))
    );
}

#[test]
fn process_inspection_permissions_and_forged_handles() {
    let mut runtime = runtime();
    run(
        &mut runtime,
        "$pid = Process::currentId(); $exists = Process::exists($pid);",
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("pid"),
        Some(&Value::Int(i64::from(std::process::id())))
    );
    assert_eq!(runtime.get_global("exists"), Some(&Value::Bool(true)));
    error(&mut runtime, "Process::exists(-1);", "TypeError");
    #[derive(Debug)]
    struct Fake;
    impl NativeObject for Fake {
        fn type_name(&self) -> &str {
            "ProcessHandle"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Ok(Value::Int(i64::from(std::process::id())))
        }
    }
    runtime
        .set_global("fake", Value::NativeObject(std::rc::Rc::new(Fake)))
        .unwrap();
    error(&mut runtime, "Process::kill($fake);", "TypeError");
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .remove("process.execute");
    options
        .context
        .permissions
        .allowed
        .remove("process.inspect");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    error(
        &mut denied,
        r#"Process::run("anything");"#,
        "PermissionError",
    );
    error(&mut denied, "Process::exists(1);", "PermissionError");
}

#[test]
fn relative_executable_paths_resolve_against_requested_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let filename = exe.file_name().unwrap();
    fs::copy(&exe, temp.path().join(filename)).unwrap();
    let mut runtime = runtime();
    runtime
        .set_global("cwd", Value::String(temp.path().to_str().unwrap().into()))
        .unwrap();
    runtime
        .set_global(
            "relative",
            Value::String(Path::new(".").join(filename).to_str().unwrap().into()),
        )
        .unwrap();
    run(
        &mut runtime,
        r#"$result = Process::run($relative, $arguments, ["cwd" => $cwd]);"#,
    )
    .unwrap();
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
}

#[test]
fn null_streams_and_host_output_ceiling_are_configurable() {
    let mut rt = runtime();
    run(&mut rt, r#"Env::set("SCR_PROCESS_FIXTURE", "echo"); $result = Process::run($program, $arguments, ["stdout" => "null", "stderr" => "null", "maxOutputBytes" => 0]);"#).unwrap();
    assert_eq!(property(&rt, "result", "success"), Value::Bool(true));
    assert_eq!(
        property(&rt, "result", "stdout"),
        Value::String(String::new())
    );
    assert_eq!(
        property(&rt, "result", "stderr"),
        Value::String(String::new())
    );
    let mut options = scr_stdlib::StdlibOptions::default();
    options.process.max_output_bytes = 128;
    let mut limited = Runtime::new();
    scr_stdlib::register_with_options(&mut limited, options).unwrap();
    error(
        &mut limited,
        r#"Process::run("ignored", [], ["maxOutputBytes" => 129]);"#,
        "TypeError",
    );
}

#[test]
fn string_and_binary_stdin_preserve_nul_utf8_empty_and_eof() {
    let mut runtime = runtime();
    run(
        &mut runtime,
        "Env::set('SCR_PROCESS_FIXTURE','stdin-echo');",
    )
    .unwrap();
    for data in [Vec::new(), b"\0\xffabc\r\n".to_vec(), (0..=255).collect()] {
        runtime
            .set_global("input", Value::Bytes(data.clone().into()))
            .unwrap();
        run(&mut runtime, "$result=Process::run($program,$arguments,['input'=>$input,'stdout'=>'null','outputType'=>'bytes']);").unwrap();
        assert_eq!(
            property(&runtime, "result", "stderr"),
            Value::Bytes(data.into())
        );
        assert_eq!(
            property(&runtime, "result", "stdout"),
            Value::Bytes(Vec::<u8>::new().into())
        );
        assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
    }
    let text = "é\0東京\r\n";
    runtime
        .set_global("input", Value::String(text.into()))
        .unwrap();
    run(
        &mut runtime,
        "$result=Process::run($program,$arguments,['input'=>$input,'stdout'=>'null']);",
    )
    .unwrap();
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String(text.into())
    );
    for options in [
        "['stdout'=>'null','outputType'=>'bytes']",
        "['stdin'=>'null','stdout'=>'null','outputType'=>'bytes']",
    ] {
        run(
            &mut runtime,
            &format!("$result=Process::run($program,$arguments,{options});"),
        )
        .unwrap();
        assert_eq!(
            property(&runtime, "result", "stderr"),
            Value::Bytes(Vec::<u8>::new().into())
        );
    }
    runtime
        .set_global("input", Value::Bytes(vec![0xff].into()))
        .unwrap();
    run(&mut runtime, "$result='unchanged';").unwrap();
    error(
        &mut runtime,
        "$result=Process::run($program,$arguments,['input'=>$input,'stdout'=>'null']);",
        "EncodingError",
    );
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
}

#[test]
fn large_bidirectional_transfers_run_and_spawn_without_pipe_deadlocks() {
    let input: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
    let mut runtime = runtime();
    runtime
        .set_global("input", Value::Bytes(input.as_slice().into()))
        .unwrap();
    run(
        &mut runtime,
        "Env::set('SCR_PROCESS_FIXTURE','write-before-input');",
    )
    .unwrap();
    for method in ["run", "spawn"] {
        let call = format!(
            "Process::{method}($program,$arguments,['input'=>$input,'outputType'=>'bytes','timeoutMs'=>5000])"
        );
        if method == "spawn" {
            run(
                &mut runtime,
                &format!(
                    "$child={call}; $result=Process::wait($child); $again=Process::poll($child);"
                ),
            )
            .unwrap();
            assert_eq!(
                property(&runtime, "again", "stderr"),
                property(&runtime, "result", "stderr")
            );
        } else {
            run(&mut runtime, &format!("$result={call};")).unwrap();
        }
        let Value::Bytes(stdout) = property(&runtime, "result", "stdout") else {
            panic!()
        };
        assert!(stdout.ends_with(&vec![b'o'; 512 * 1024]));
        let Value::Bytes(stderr) = property(&runtime, "result", "stderr") else {
            panic!()
        };
        assert_eq!(&stderr[..512 * 1024], vec![b'e'; 512 * 1024]);
        assert_eq!(&stderr[512 * 1024..], input);
        assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
    }
}

#[test]
fn early_exit_discards_unread_stdin_without_a_broken_pipe_error() {
    let mut runtime = runtime();
    runtime
        .set_global("input", Value::Bytes(vec![0xff; 2 * 1024 * 1024].into()))
        .unwrap();
    for (mode, code, expected) in [
        ("skip-input", 0, Vec::new()),
        ("early-input-exit", 17, vec![0xff]),
    ] {
        runtime
            .set_global("mode", Value::String(mode.into()))
            .unwrap();
        run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE',$mode); $result=Process::run($program,$arguments,['input'=>$input,'stdout'=>'null','outputType'=>'bytes','timeoutMs'=>2000]);").unwrap();
        assert_eq!(property(&runtime, "result", "exitCode"), Value::Int(code));
        assert_eq!(
            property(&runtime, "result", "stderr"),
            Value::Bytes(expected.into())
        );
    }
}

#[test]
fn input_ceilings_conflicts_and_output_type_are_checked_before_launch() {
    let config = scr_stdlib::process::ProcessConfig {
        max_input_bytes: 2,
        ..Default::default()
    };
    let mut runtime = configured_runtime(config, Default::default());
    run(
        &mut runtime,
        "Env::set('SCR_PROCESS_FIXTURE','stdin-echo'); $result='unchanged';",
    )
    .unwrap();
    for options in [
        "['input'=>null]",
        "['input'=>1]",
        "['input'=>'x','stdin'=>'inherit']",
        "['stdin'=>'null','input'=>'']",
        "['maxInputBytes'=>3]",
        "['maxInputBytes'=>-1]",
        "['outputType'=>'binary']",
        "['outputType'=>true]",
    ] {
        error(
            &mut runtime,
            &format!("$result=Process::run('never-launched',[],{options});"),
            "TypeError",
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
    for input in [
        Value::String("abc".into()),
        Value::Bytes(vec![1, 2, 3].into()),
    ] {
        runtime.set_global("input", input).unwrap();
        error(
            &mut runtime,
            "$result=Process::run('never-launched',[],['input'=>$input]);",
            "InputLimitError",
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
    run(&mut runtime,"$result=Process::run($program,$arguments,['input'=>'é','stdout'=>'null','maxInputBytes'=>2]);").unwrap();
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String("é".into())
    );
    error(
        &mut runtime,
        "Process::run('never-launched',[],['input'=>'é','maxInputBytes'=>1]);",
        "InputLimitError",
    );
    run(&mut runtime,"$result=Process::run($program,$arguments,['input'=>'','maxInputBytes'=>0,'stdout'=>'null']);").unwrap();
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::String(String::new())
    );
}

#[test]
fn exact_output_and_logical_value_budgets_charge_results_and_completed_handles() {
    let mut runtime = runtime();
    run(
        &mut runtime,
        "Env::set('SCR_PROCESS_FIXTURE','exact-output'); Env::set('SCR_PROCESS_BYTES','4096');",
    )
    .unwrap();
    for output_type in ["text", "bytes"] {
        run(&mut runtime,&format!("$result=Process::run($program,$arguments,['stdout'=>'null','outputType'=>'{output_type}','maxOutputBytes'=>4096]);")).unwrap();
        let Some(Value::NativeObject(result)) = runtime.get_global("result") else {
            panic!()
        };
        assert_eq!(result.payload_bytes(), 4096);
        run(&mut runtime, "$result='unchanged';").unwrap();
        error(
            &mut runtime,
            &format!(
                "$result=Process::run($program,$arguments,['stdout'=>'null','outputType'=>'{output_type}','maxOutputBytes'=>4095]);"
            ),
            "OutputLimitError",
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
    for payload_budget in [4095, 4096] {
        let mut rt = configured_runtime(
            Default::default(),
            RuntimeOptions {
                max_value_bytes: Some(std::mem::size_of::<Value>() + payload_budget),
                ..Default::default()
            },
        );
        run(&mut rt,"Env::set('SCR_PROCESS_FIXTURE','exact-output'); Env::set('SCR_PROCESS_BYTES','4096'); $result='unchanged';").unwrap();
        if payload_budget == 4095 {
            error(
                &mut rt,
                "$result=Process::run($program,$arguments,['stdout'=>'null','outputType'=>'bytes']);",
                "ResourceLimitError",
            );
            assert_eq!(
                rt.get_global("result"),
                Some(&Value::String("unchanged".into()))
            );
        } else {
            run(&mut rt,"$child=Process::spawn($program,$arguments,['stdout'=>'null','outputType'=>'bytes']); $result=Process::wait($child);").unwrap();
            for name in ["child", "result"] {
                let value = rt.get_global(name).unwrap().clone();
                let Value::NativeObject(object) = &value else {
                    panic!()
                };
                assert_eq!(object.payload_bytes(), 4096);
                assert!(rt.set_global("nested", Value::list(vec![value])).is_err());
            }
        }
    }
}

#[test]
fn pending_input_timeouts_cancellation_and_drop_reap_the_owned_child() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let flag = Arc::new(AtomicBool::new(false));
    let mut rt = configured_runtime(
        Default::default(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
    );
    rt.set_global("input", Value::Bytes(vec![0xff; 2 * 1024 * 1024].into()))
        .unwrap();
    rt.set_global("pidFile", Value::String(pid_file.to_str().unwrap().into()))
        .unwrap();
    run(&mut rt,"Env::set('SCR_PROCESS_FIXTURE','sleep'); Env::set('SCR_CHILD_PID_FILE',$pidFile); $result='unchanged';").unwrap();
    error(
        &mut rt,
        "$result=Process::run($program,$arguments,['input'=>$input,'timeoutMs'=>300]);",
        "TimeoutError",
    );
    assert_eq!(
        rt.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
    assert!(gone(
        fs::read_to_string(&pid_file).unwrap().parse().unwrap()
    ));
    let cancellation_pid = dir.path().join("cancellation-pid");
    rt.set_global(
        "pidFile",
        Value::String(cancellation_pid.to_str().unwrap().into()),
    )
    .unwrap();
    run(&mut rt, "Env::set('SCR_CHILD_PID_FILE',$pidFile);").unwrap();
    let observer = cancellation_pid.clone();
    let cancel = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !observer.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        flag.store(true, Ordering::Relaxed);
    });
    let error = run(
        &mut rt,
        "$result=Process::run($program,$arguments,['input'=>$input,'timeoutMs'=>5000]);",
    )
    .unwrap_err();
    cancel.join().unwrap();
    assert!(error.message.contains("interrupted"));
    assert_eq!(
        rt.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
    assert!(gone(
        fs::read_to_string(cancellation_pid)
            .unwrap()
            .parse()
            .unwrap()
    ));
    // Dropping an unattended spawned handle also releases the input file/child.
    let mut rt = runtime();
    rt.set_global("input", Value::Bytes(vec![0xff; 2 * 1024 * 1024].into()))
        .unwrap();
    run(&mut rt,"Env::set('SCR_PROCESS_FIXTURE','sleep'); $child=Process::spawn($program,$arguments,['input'=>$input]);").unwrap();
    let child_pid = pid(&rt, "child");
    rt.set_global("child", Value::Null).unwrap();
    assert!(gone(child_pid));
}

#[cfg(windows)]
#[test]
fn windows_explicit_shell_preserves_its_command_language_quoting() {
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .insert("process.shell".into());
    let mut rt = Runtime::with_options(options);
    scr_stdlib::register(&mut rt).unwrap();
    rt.set_global(
        "code",
        Value::String("echo \"quoted words\" & echo second-line".into()),
    )
    .unwrap();
    run(&mut rt, "$result = Process::shell($code);").unwrap();
    assert!(
        matches!(property(&rt, "result", "stdout"), Value::String(value) if value.contains("quoted words") && value.contains("second-line") && !value.contains("\\\""))
    );
}

fn interactive_read(runtime: &mut Runtime, stream: &str, timeout: u32) -> Option<Vec<u8>> {
    run(
        runtime,
        &format!("$chunk = Process::read{stream}($child, 65536, {timeout});"),
    )
    .unwrap();
    match runtime.get_global("chunk").unwrap() {
        Value::Null => None,
        Value::Bytes(data) => Some(data.to_vec()),
        other => panic!("unexpected output {other:?}"),
    }
}
fn interactive_drain(runtime: &mut Runtime) -> (Vec<u8>, Vec<u8>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        assert!(Instant::now() < deadline, "interactive drain timed out");
        let out = interactive_read(runtime, "Stdout", 5);
        let err = interactive_read(runtime, "Stderr", 5);
        if out.is_none() && err.is_none() {
            break;
        }
        stdout.extend(out.unwrap_or_default());
        stderr.extend(err.unwrap_or_default());
    }
    (stdout, stderr)
}

#[test]
fn interactive_responses_arrive_before_stdin_eof_and_remain_binary() {
    let mut runtime = runtime();
    run(&mut runtime, "Env::set('SCR_PROCESS_FIXTURE','interactive-response'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>5000]);").unwrap();
    assert_eq!(
        interactive_read(&mut runtime, "Stderr", 1000).unwrap(),
        b"ready"
    );
    runtime
        .set_global("input", Value::Bytes(vec![0xff, 0, 0xc3, 0xa9].into()))
        .unwrap();
    run(
        &mut runtime,
        "$accepted=Process::writeStdin($child,$input);",
    )
    .unwrap();
    assert_eq!(runtime.get_global("accepted"), Some(&Value::Int(4)));
    assert_eq!(
        interactive_read(&mut runtime, "Stderr", 1000).unwrap(),
        [0xff, 0, 0xc3, 0xa9]
    );
    run(&mut runtime, "$pending=Process::poll($child);").unwrap();
    assert_eq!(runtime.get_global("pending"), Some(&Value::Null));
    run(&mut runtime, "$result=42;").unwrap();
    error(
        &mut runtime,
        "$result=Process::wait($child);",
        "ProcessError",
    );
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    run(
        &mut runtime,
        "Process::closeStdin($child); Process::closeStdin($child); $result=Process::wait($child);",
    )
    .unwrap();
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
    assert_eq!(
        property(&runtime, "result", "stdout"),
        Value::Bytes(Vec::<u8>::new().into())
    );
    assert_eq!(
        property(&runtime, "result", "stderr"),
        Value::Bytes(Vec::<u8>::new().into())
    );
    let (stdout, stderr) = interactive_drain(&mut runtime);
    assert!(stdout.ends_with(b"done"));
    assert!(stderr.is_empty());
    error(
        &mut runtime,
        "Process::writeStdin($child,'x');",
        "ProcessError",
    );
}

#[test]
fn interactive_large_bidirectional_transfer_cannot_deadlock_on_output() {
    let mut runtime = runtime();
    let data: Vec<_> = (0..1024 * 1024).map(|n| (n % 256) as u8).collect();
    runtime
        .set_global("input", Value::Bytes(data.clone().into()))
        .unwrap();
    run(&mut runtime, "Env::set('SCR_PROCESS_FIXTURE','write-before-input'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>10000]); $offset=0;").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    while runtime.get_global("offset") != Some(&Value::Int(data.len() as i64)) {
        assert!(Instant::now() < deadline);
        run(&mut runtime, "$size=Math::min(16384,len($input)-$offset); $offset=$offset+Process::writeStdin($child,Bytes::slice($input,$offset,$size));").unwrap();
        stdout.extend(interactive_read(&mut runtime, "Stdout", 0).unwrap_or_default());
        stderr.extend(interactive_read(&mut runtime, "Stderr", 0).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(1));
    }
    run(
        &mut runtime,
        "Process::closeStdin($child); $result=Process::wait($child);",
    )
    .unwrap();
    let (out, err) = interactive_drain(&mut runtime);
    stdout.extend(out);
    stderr.extend(err);
    assert!(stdout.ends_with(&vec![b'o'; 512 * 1024]));
    let mut expected = vec![b'e'; 512 * 1024];
    expected.extend(data);
    assert_eq!(stderr, expected);
    assert_eq!(property(&runtime, "result", "success"), Value::Bool(true));
}

#[test]
fn interactive_input_limits_ranges_invalid_modes_and_failed_destinations_are_strict() {
    let mut runtime = runtime();
    run(&mut runtime, "$result=42;").unwrap();
    for options in [
        "['interactive'=>1]",
        "['interactive'=>true,'stdin'=>'null']",
        "['interactive'=>true,'stdout'=>'capture']",
        "['interactive'=>true,'stderr'=>'inherit']",
        "['interactive'=>true,'input'=>'']",
        "['interactive'=>true,'outputType'=>'bytes']",
        "['interactive'=>true,'timeoutMs'=>null]",
        "['interactive'=>true,'timeoutMs'=>0]",
    ] {
        error(
            &mut runtime,
            &format!("$result=Process::spawn('never-launched',[],{options});"),
            "TypeError",
        );
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    }
    error(
        &mut runtime,
        "Process::run('never-launched',[],['interactive'=>true]);",
        "TypeError",
    );
    run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE','stdin-echo'); $child=Process::spawn($program,$arguments,['interactive'=>true,'maxInputBytes'=>2,'timeoutMs'=>3000]);").unwrap();
    let source = "function invalidRead($h) { return Process::readStdout($h, 0); } $result=invalidRead($child);";
    let failure = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(failure.message.starts_with("RangeError:"));
    assert_eq!(
        failure.span.start,
        source.find("Process::readStdout").unwrap()
    );
    assert!(
        failure
            .trace
            .iter()
            .any(|frame| frame.function == "invalidRead")
    );
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    for expression in [
        "Process::readStdout($child,0)",
        "Process::readStderr($child,65537)",
        "Process::readStdout($child,1,-1)",
        "Process::readStderr($child,1,30001)",
    ] {
        error(
            &mut runtime,
            &format!("$result={expression};"),
            "RangeError",
        );
        assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    }
    for expression in [
        "Process::writeStdin($child,1)",
        "Process::readStdout($child,1.0)",
        "Process::readStderr($child,1,null)",
        "Process::closeStdin($child,1)",
    ] {
        error(&mut runtime, &format!("$result={expression};"), "TypeError");
    }
    error(
        &mut runtime,
        "$result=Process::writeStdin($child,'abc');",
        "InputLimitError",
    );
    run(&mut runtime, "$accepted=Process::writeStdin($child,'é');").unwrap();
    assert_eq!(runtime.get_global("accepted"), Some(&Value::Int(2)));
    error(
        &mut runtime,
        "$result=Process::writeStdin($child,'x');",
        "InputLimitError",
    );
    run(
        &mut runtime,
        "Process::closeStdin($child); Process::wait($child);",
    )
    .unwrap();
    let (_, stderr) = interactive_drain(&mut runtime);
    assert_eq!(stderr, "é".as_bytes());
    run(
        &mut runtime,
        "$plain=Process::spawn($program,$arguments,['input'=>'']);",
    )
    .unwrap();
    error(
        &mut runtime,
        "Process::readStdout($plain,1);",
        "ProcessError",
    );
    run(&mut runtime, "Process::wait($plain);").unwrap();
}

#[test]
fn interactive_broken_pipe_preserves_early_exit_status() {
    let mut runtime = runtime();
    run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE','early-input-exit'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>3000]);").unwrap();
    runtime
        .set_global("payload", Value::Bytes(vec![0xff; 65536].into()))
        .unwrap();
    run(&mut runtime, "Process::writeStdin($child,$payload);").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        run(&mut runtime, "$result=Process::poll($child);").unwrap();
        if runtime.get_global("result") != Some(&Value::Null) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(property(&runtime, "result", "exitCode"), Value::Int(17));
    error(
        &mut runtime,
        "Process::writeStdin($child,'unread');",
        "BrokenPipeError",
    );
    run(&mut runtime, "$again=Process::wait($child);").unwrap();
    assert_eq!(property(&runtime, "again", "success"), Value::Bool(false));
}

#[test]
fn interactive_foreign_and_forged_handles_and_permissions_are_rejected() {
    let mut owner = runtime();
    run(&mut owner,"Env::set('SCR_PROCESS_FIXTURE','sleep'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>3000]);").unwrap();
    let mut foreign = runtime();
    foreign
        .set_global("child", owner.get_global("child").unwrap().clone())
        .unwrap();
    for expression in [
        "Process::readStdout($child,1)",
        "Process::readStderr($child,1)",
        "Process::writeStdin($child,'x')",
        "Process::closeStdin($child)",
        "Process::wait($child)",
        "Process::poll($child)",
        "Process::kill($child)",
    ] {
        error(&mut foreign, &format!("{expression};"), "TypeError");
    }
    #[derive(Debug)]
    struct Fake;
    impl NativeObject for Fake {
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("forged".into())
        }
        fn type_name(&self) -> &str {
            "ProcessHandle"
        }
    }
    foreign
        .set_global("child", Value::NativeObject(std::rc::Rc::new(Fake)))
        .unwrap();
    error(&mut foreign, "Process::readStdout($child,1);", "TypeError");
    let mut denied_options = RuntimeOptions::default();
    denied_options
        .context
        .permissions
        .allowed
        .remove("process.execute");
    let mut denied = configured_runtime(Default::default(), denied_options);
    denied
        .set_global("child", owner.get_global("child").unwrap().clone())
        .unwrap();
    error(
        &mut denied,
        "Process::writeStdin($child,'x');",
        "PermissionError",
    );
    run(&mut owner, "Process::kill($child);").unwrap();
}

#[test]
fn interactive_timeouts_output_limits_and_cancellation_reap_owned_children() {
    let mut runtime = runtime();
    run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE','sleep'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>60]);").unwrap();
    let child_pid = pid(&runtime, "child");
    error(
        &mut runtime,
        "Process::readStderr($child,1,1000);",
        "TimeoutError",
    );
    assert!(gone(child_pid));
    run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE','flood'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>3000,'maxOutputBytes'=>100]); Process::closeStdin($child);").unwrap();
    let child_pid = pid(&runtime, "child");
    error(
        &mut runtime,
        "Process::readStderr($child,1,1000);",
        "OutputLimitError",
    );
    error(&mut runtime, "Process::wait($child);", "OutputLimitError");
    assert!(gone(child_pid));
    let flag = Arc::new(AtomicBool::new(false));
    let mut cancelled = configured_runtime(
        Default::default(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
    );
    run(&mut cancelled,"Env::set('SCR_PROCESS_FIXTURE','sleep'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>3000]);").unwrap();
    cancelled
        .set_global("payload", Value::Bytes(vec![b'x'; 65536].into()))
        .unwrap();
    run(&mut cancelled, "Process::writeStdin($child,$payload);").unwrap();
    let child_pid = pid(&cancelled, "child");
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        flag.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    assert!(
        run(
            &mut cancelled,
            "try { Process::readStderr($child,1,30000); } catch (Error $error) {}"
        )
        .is_err()
    );
    worker.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(gone(child_pid));
}

#[test]
fn interactive_last_alias_drop_and_kill_cleanup_child_trees() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("late");
    let mut runtime = runtime();
    runtime
        .set_global("marker", Value::String(marker.to_str().unwrap().into()))
        .unwrap();
    run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE','sleep'); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>3000]); $alias=$child;").unwrap();
    runtime
        .set_global("payload", Value::Bytes(vec![b'x'; 65536].into()))
        .unwrap();
    run(&mut runtime, "Process::writeStdin($child,$payload);").unwrap();
    let child_pid = pid(&runtime, "child");
    run(&mut runtime, "$child=null;").unwrap();
    assert!(!gone_immediately(child_pid));
    run(&mut runtime, "$alias=null;").unwrap();
    assert!(gone(child_pid));
    run(&mut runtime,"Env::set('SCR_PROCESS_FIXTURE','descendant'); Env::set('SCR_LATE_MARKER',$marker); $child=Process::spawn($program,$arguments,['interactive'=>true,'timeoutMs'=>3000]); Process::closeStdin($child); Process::wait($child);").unwrap();
    std::thread::sleep(Duration::from_millis(900));
    assert!(!marker.exists());
}
fn gone_immediately(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).is_none()
}
