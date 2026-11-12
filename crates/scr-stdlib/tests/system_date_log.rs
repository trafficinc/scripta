use scr_runtime::{Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::{FilesystemPolicy, StdlibOptions, log::LogConfig};
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
struct Capture(Rc<RefCell<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn runtime() -> Runtime {
    let mut runtime = Runtime::new();
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), RuntimeError> {
    let source = format!("<?scr {source}");
    runtime
        .execute(
            &scr_parser::parse(&scr_lexer::lex(&source).unwrap()).unwrap(),
            &mut Vec::new(),
        )
        .map(|_| ())
}
fn error(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = run(runtime, source).unwrap_err();
    assert!(
        error.message.starts_with(&format!("{kind}:")),
        "{source}: {error}"
    );
}
fn field(runtime: &Runtime, name: &str, property: &str) -> Value {
    let Some(Value::NativeObject(object)) = runtime.get_global(name) else {
        panic!("missing {name}")
    };
    object.get_property(property).unwrap()
}
fn int(value: Value) -> i64 {
    let Value::Int(value) = value else { panic!() };
    value
}

#[test]
fn dates_use_unix_milliseconds_utc_and_explicit_offsets() {
    let mut runtime = runtime();
    run(
        &mut runtime,
        r#"
        $epoch = Date::parse("1970-01-01T00:00:00Z");
        $offset = Date::parse("1970-01-01T01:00:00+01:00");
        $before = Date::parse("1969-12-31T23:59:59.999Z");
        $leap = Date::parse("2024-02-29T12:34:56.123456Z");
        $formatted = Date::format($leap, "%Y-%m-%dT%H:%M:%S%.3f%:z");
        $now = Date::now(); Date::sleep(0);
    "#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("epoch"), Some(&Value::Int(0)));
    assert_eq!(runtime.get_global("offset"), Some(&Value::Int(0)));
    assert_eq!(runtime.get_global("before"), Some(&Value::Int(-1)));
    assert_eq!(
        runtime.get_global("formatted"),
        Some(&Value::String("2024-02-29T12:34:56.123+00:00".into()))
    );
    assert!(matches!(runtime.get_global("now"), Some(Value::Int(n)) if *n > 0));
    for source in [
        r#"Date::parse("2023-02-29T00:00:00Z");"#,
        r#"Date::parse("2024-01-01");"#,
        r#"Date::parse("2016-12-31T23:59:60Z");"#,
        r#"Date::format(0, "%Q");"#,
        r#"Date::format(0, "%#z");"#,
    ] {
        error(&mut runtime, source, "DateError");
    }
    error(
        &mut runtime,
        r#"Date::format(9223372036854775807, "%Y");"#,
        "RangeError",
    );
    for source in [
        "Date::now(1);",
        "Date::sleep(-1);",
        "Date::sleep(1.0);",
        "Date::parse(0);",
    ] {
        error(&mut runtime, source, "TypeError");
    }
}

#[test]
fn sleep_is_cancellable_and_permission_checked() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.remove("sleep");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    error(&mut denied, "Date::sleep(0);", "PermissionError");
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let start = Instant::now();
    assert!(run(&mut runtime, "Date::sleep(5000);").is_err());
    worker.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn host_metrics_have_portable_types_and_real_sampling_interval() {
    let mut runtime = runtime();
    runtime
        .set_global(
            "path",
            Value::String(std::env::temp_dir().to_str().unwrap().into()),
        )
        .unwrap();
    run(&mut runtime, "$platform = System::platform(); $arch = System::architecture(); $host = System::hostname(); $home = System::homeDirectory(); $temp = System::tempDirectory(); $up = System::uptime(); $memory = System::memoryUsage(); $disk = System::diskUsage($path);").unwrap();
    assert_eq!(
        runtime.get_global("platform"),
        Some(&Value::String(std::env::consts::OS.into()))
    );
    assert_eq!(
        runtime.get_global("arch"),
        Some(&Value::String(std::env::consts::ARCH.into()))
    );
    for key in ["host", "home", "temp"] {
        assert!(matches!(runtime.get_global(key), Some(Value::String(s)) if !s.is_empty()));
    }
    assert!(matches!(runtime.get_global("up"), Some(Value::Int(n)) if *n > 0));
    for name in ["disk", "memory"] {
        let total = int(field(&runtime, name, "totalBytes"));
        let used = int(field(&runtime, name, "usedBytes"));
        let available = int(field(&runtime, name, "availableBytes"));
        assert!(total > 0 && used >= 0 && available >= 0 && used <= total && available <= total);
        assert!(
            matches!(field(&runtime, name, "percentUsed"), Value::Float(n) if (0.0..=100.0).contains(&n))
        );
    }
    let start = Instant::now();
    run(&mut runtime, "$cpu = System::cpuUsage();").unwrap();
    assert!(start.elapsed() >= Duration::from_millis(200));
    assert!(
        matches!(runtime.get_global("cpu"), Some(Value::Float(n)) if (0.0..=100.0).contains(n))
    );
    error(&mut runtime, "System::cpuUsage(0);", "RangeError");
    error(&mut runtime, "System::cpuUsage(60001);", "RangeError");
    error(&mut runtime, "System::cpuUsage(-1);", "TypeError");
    error(&mut runtime, "System::diskUsage(1);", "TypeError");
    error(&mut runtime, r#"System::diskUsage("");"#, "PathError");
    runtime
        .set_global(
            "path",
            Value::String(
                std::env::temp_dir()
                    .join("scr-nonexistent-phase10-volume-path")
                    .to_str()
                    .unwrap()
                    .into(),
            ),
        )
        .unwrap();
    error(&mut runtime, "System::diskUsage($path);", "IoError");
}

#[test]
fn logs_are_json_lines_on_stderr_with_uniform_levels_and_redaction() {
    let capture = Capture::default();
    let mut runtime = runtime();
    runtime.set_error_stream(capture.clone());
    run(&mut runtime, r#"Env::set("SERVICE_SECRET", "hidden-value"); Log::info("hello\n世界"); Log::warn("warning"); Log::error("failure"); Log::debug("details"); Log::info("password=secret"); Log::info("value: hidden-value");"#).unwrap();
    let output = String::from_utf8(capture.0.borrow().clone()).unwrap();
    let records: Vec<serde_json::Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 6);
    for (record, level) in records
        .iter()
        .zip(["info", "warn", "error", "debug", "info", "info"])
    {
        assert_eq!(record["level"], level);
        chrono::DateTime::parse_from_rfc3339(record["timestamp"].as_str().unwrap()).unwrap();
    }
    assert_eq!(records[0]["message"], "hello\n世界");
    assert!(!output.contains("secret"));
    assert!(!output.contains("hidden-value"));
    error(&mut runtime, "Log::info(1);", "TypeError");
}

#[test]
fn log_files_append_and_reset_to_stderr_with_host_redactions() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("events.jsonl");
    std::fs::write(&file, "existing\n").unwrap();
    let mut runtime = Runtime::new();
    scr_stdlib::register_with_options(
        &mut runtime,
        StdlibOptions {
            log: LogConfig {
                file: Some(file.clone()),
                redacted_values: vec!["hide-me".into()],
            },
            ..Default::default()
        },
    )
    .unwrap();
    let capture = Capture::default();
    runtime.set_error_stream(capture.clone());
    run(
        &mut runtime,
        r#"Log::info("hide-me"); Log::setFile(null); Log::warn("stderr");"#,
    )
    .unwrap();
    let data = std::fs::read_to_string(&file).unwrap();
    assert!(data.starts_with("existing\n"));
    assert!(!data.contains("hide-me"));
    assert!(data.contains("[REDACTED]"));
    assert!(
        String::from_utf8(capture.0.borrow().clone())
            .unwrap()
            .contains("stderr")
    );
    runtime
        .set_global("file", Value::String(file.to_str().unwrap().into()))
        .unwrap();
    run(&mut runtime, r#"Log::setFile($file); Log::debug("again");"#).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("again"));
}

#[test]
fn inspection_and_file_logging_respect_capabilities_and_root_policy() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.remove("system.inspect");
    options.context.permissions.allowed.remove("log.write");
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    for source in [
        "System::hostname();",
        "System::memoryUsage();",
        "System::cpuUsage();",
        r#"System::diskUsage(".");"#,
        "System::uptime();",
        "System::homeDirectory();",
        "System::tempDirectory();",
        r#"Log::info("x");"#,
        "Log::setFile(null);",
    ] {
        error(&mut denied, source, "PermissionError");
    }
    let mut restricted = Runtime::new();
    scr_stdlib::register_with_options(
        &mut restricted,
        StdlibOptions {
            filesystem: FilesystemPolicy {
                roots: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    error(
        &mut restricted,
        r#"System::diskUsage(".");"#,
        "PermissionError",
    );
    error(
        &mut restricted,
        r#"Log::setFile("denied.jsonl");"#,
        "PermissionError",
    );
    struct Fail;
    impl Write for Fail {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut runtime = runtime();
    runtime.set_error_stream(Fail);
    error(&mut runtime, r#"Log::error("x");"#, "IoError");
}
