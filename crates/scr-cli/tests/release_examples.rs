use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, SystemTime},
};
fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}
fn run(name: &str, args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(example(name))
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn every_example_parses_and_simple_examples_run_without_external_runtimes() {
    let names = [
        "hello.scr",
        "variables.scr",
        "loops.scr",
        "functions.scr",
        "file-organizer.scr",
        "log-cleaner.scr",
        "directory-backup.scr",
        "server-health.scr",
        "http-client.scr",
        "command-runner.scr",
    ];
    for name in names {
        scr_cli::pipeline::check(&fs::read_to_string(example(name)).unwrap()).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    for (name, expected) in [
        ("hello.scr", "Hello, Scripta!\n"),
        ("loops.scr", "1\n2\n3\n"),
        ("functions.scr", "Hello, Scripta!\n6\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
            .arg(example(name))
            .env("PATH", dir.path())
            .output()
            .unwrap();
        assert_eq!(success(output), expected);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .args(["version", "--verbose"])
        .output()
        .unwrap();
    let text = success(output);
    assert!(
        text.starts_with(&format!("scr {}\n", env!("CARGO_PKG_VERSION")))
            && text.contains("target: ")
            && text.contains("compiler: rustc ")
    );
}
#[test]
fn organizer_and_cleaner_preview_before_changes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("organized");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), "contents").unwrap();
    success(run(
        "file-organizer.scr",
        &[source.as_os_str(), destination.as_os_str()],
    ));
    assert!(source.join("note.txt").exists());
    assert!(!destination.exists());
    success(run(
        "file-organizer.scr",
        &[
            source.as_os_str(),
            destination.as_os_str(),
            "--apply".as_ref(),
        ],
    ));
    assert_eq!(
        fs::read_to_string(destination.join("txt/note.txt")).unwrap(),
        "contents"
    );
    let old = source.join("old.log");
    fs::write(&old, "old").unwrap();
    let file = fs::OpenOptions::new().write(true).open(&old).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    drop(file);
    success(run("log-cleaner.scr", &[source.as_os_str()]));
    assert!(old.exists());
    success(run(
        "log-cleaner.scr",
        &[source.as_os_str(), "--apply".as_ref()],
    ));
    assert!(!old.exists());
}
#[test]
fn backup_copies_nested_binary_data_without_overwriting_or_recursing_into_destination() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("backup");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested/data.bin"), [0, 255, 7]).unwrap();
    success(run(
        "directory-backup.scr",
        &[source.as_os_str(), destination.as_os_str()],
    ));
    assert!(!destination.exists());
    success(run(
        "directory-backup.scr",
        &[
            source.as_os_str(),
            destination.as_os_str(),
            "--apply".as_ref(),
        ],
    ));
    assert_eq!(
        fs::read(destination.join("nested/data.bin")).unwrap(),
        [0, 255, 7]
    );
    assert_eq!(
        run(
            "directory-backup.scr",
            &[
                source.as_os_str(),
                destination.as_os_str(),
                "--apply".as_ref()
            ]
        )
        .status
        .code(),
        Some(1)
    );
    assert_eq!(
        run(
            "directory-backup.scr",
            &[
                source.as_os_str(),
                source.join("inside").as_os_str(),
                "--apply".as_ref()
            ]
        )
        .status
        .code(),
        Some(1)
    );
    assert!(!source.join("inside").exists());
}
#[test]
fn command_runner_preserves_literal_arguments_and_http_example_uses_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("child.scr");
    fs::write(&child, "<?scr echo Cli::arg(0);").unwrap();
    // Build JSON without introducing a JSON dependency just for the fixture path.
    let escaped = child
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let args = format!("[\"{escaped}\",\"literal ; $(never-run)\"]");
    assert_eq!(
        success(run(
            "command-runner.scr",
            &[env!("CARGO_BIN_EXE_scr").as_ref(), args.as_ref()]
        )),
        "literal ; $(never-run)"
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        loop {
            if let Ok((mut stream, _)) = listener.accept() {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let _ = stream.read(&mut [0; 4096]);
                let body = b"{\"status\":\"up\"}";
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let output = success(run("http-client.scr", &[url.as_ref()]));
    assert!(output.contains("\"status\": \"up\""));
    worker.join().unwrap();
}

#[test]
fn strings_playground_normalizes_a_unicode_record_without_external_programs() {
    let directory = tempfile::tempdir().unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/strings.scr");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "info: café_api: READY\nService prefix: café\nContains api: true\nStarts with café: true\nEnds with _api: true\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let result = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(path)
        .current_dir(directory.path())
        .env("PATH", directory.path())
        .output()
        .unwrap();
    assert_eq!(success(result), expected);
}

#[test]
fn collection_playground_and_log_analyzer_run_without_external_programs() {
    let directory = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground");
    for (name, args, expected) in [
        (
            "collections.scr",
            vec![],
            "Active: auth, café-api\nFirst: auth\nWithout first: [\"café-api\"]\nWith extra: [\"auth\",\"café-api\",\"queue\"]\nHas auth: true\nFields: active, total\nValues: [2,4]\nHas total: true\n",
        ),
        (
            "log-analyzer.scr",
            vec![root.join("sample-events.txt")],
            "Services: auth, café-api, worker\ninfo: 3\nwarn: 1\nerror: 1\nMalformed: 1\n",
        ),
    ] {
        let source = fs::read_to_string(root.join(name)).unwrap();
        scr_cli::pipeline::check(&source).unwrap();
        let formatted = scr_cli::formatter::format(&source).unwrap();
        scr_cli::pipeline::check(&formatted).unwrap();
        assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
            .arg(root.join(name))
            .args(args)
            .current_dir(directory.path())
            .env("PATH", directory.path())
            .output()
            .unwrap();
        assert_eq!(success(output), expected, "{name}");
    }
    let input = directory.path().join("events.txt");
    fs::write(&input, "# comment\r\n\r\nINFO | api | ok\r\nDEBUG | api | ignored\nerror||missing service\nwarn|api|\na|b|c|d\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(root.join("log-analyzer.scr"))
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(
        success(output),
        "Services: api\ninfo: 1\nwarn: 0\nerror: 0\nMalformed: 4\n"
    );
    fs::write(&input, "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(root.join("log-analyzer.scr"))
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(
        success(output),
        "Services: \ninfo: 0\nwarn: 0\nerror: 0\nMalformed: 0\n"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(root.join("log-analyzer.scr"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("Usage:")
    );
}

#[test]
fn error_playground_recovers_and_uncaught_errors_exit_one_with_source_details() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/errors.scr");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "Missing key: recovered\nArithmetic: recovered\nPreserved value: 10\nType mismatch: recovered\nStill running\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(success(output), expected);
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(&path)
        .arg("--uncaught")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("error[E0401]: IndexError: list index out of bounds"));
    assert!(error.contains("errors.scr:6:"));
    assert!(error.contains("at <main>"));
}

#[test]
fn functions_playground_runs_with_defaults_variadics_and_captured_callbacks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/functions.scr");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "Hello, Scripta!\nWelcome, Scripta!\nTotal: 16\nCallback: 14\nCounter: 6, 8\nFresh defaults: [[\"new\"],[\"new\"]]\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(success(output), expected);
}

#[test]
fn reporting_playground_runs_before_and_after_formatting() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/reporting.scr");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "Run summary: 4 jobs\nDuration: 8.6–19.5 ms; rounded average: 13 ms\nAverage bounds: 12–13 ms\nAbsolute adjustment: 2 ms\n{complete}: true\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("run")
        .arg(path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(success(output), expected);
}

#[test]
fn streaming_playground_handles_sample_crlf_empty_files_and_usage() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/streaming.scr");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    let dir = tempfile::tempdir().unwrap();
    let sample = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/sample-events.txt"),
    )
    .unwrap();
    let cases = [
        (
            sample.as_str(),
            "Records: 5\ninfo: 3\nwarn: 1\nerror: 1\nMalformed: 1\nStream closed: true\n",
        ),
        (
            "INFO|café|ok\r\n\r\n# comment\r\nwarn|worker|retry\r\nbad",
            "Records: 2\ninfo: 1\nwarn: 1\nerror: 0\nMalformed: 1\nStream closed: true\n",
        ),
        (
            "",
            "Records: 0\ninfo: 0\nwarn: 0\nerror: 0\nMalformed: 0\nStream closed: true\n",
        ),
    ];
    let formatted_path = dir.path().join("formatted.scr");
    fs::write(&formatted_path, formatted).unwrap();
    for (input, expected) in cases {
        let input_path = dir.path().join("events.txt");
        fs::write(&input_path, input).unwrap();
        for script in [&path, &formatted_path] {
            let output = Command::new(env!("CARGO_BIN_EXE_scr"))
                .arg("run")
                .arg(script)
                .arg(&input_path)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert_eq!(success(output), expected);
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("run")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("Usage:")
    );
    for (bytes, error) in [
        (vec![0xff, b'\n'], "EncodingError"),
        (vec![b'x'; 65537], "ResourceLimitError"),
    ] {
        let input_path = dir.path().join("bad.txt");
        fs::write(&input_path, bytes).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
            .arg("run")
            .arg(&path)
            .arg(input_path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8(output.stderr).unwrap().contains(error));
    }
}

#[test]
fn health_example_runs_locally_and_reference_lists_all_exports() {
    let dir = tempfile::tempdir().unwrap();
    let output = success(run("server-health.scr", &[dir.path().as_os_str()]));
    assert!(
        output.contains("\"cpuPercent\"")
            && output.contains("\"systemMemory\"")
            && output.contains("\"disk\"")
    );
    let reference = include_str!("../../../docs/standard-library.md");
    let modules = [
        (
            "File",
            "exists read write append copy move delete size modified extension openLines readLine close",
        ),
        (
            "Dir",
            "exists create createAll files directories remove current change",
        ),
        ("Path", "join filename extension parent normalize absolute"),
        ("Process", "run spawn shell wait poll kill exists currentId"),
        ("Env", "get set has remove all"),
        ("Cli", "args arg has option prompt readLine"),
        (
            "System",
            "platform architecture hostname cpuUsage memoryUsage diskUsage uptime homeDirectory tempDirectory",
        ),
        ("Date", "now format parse sleep"),
        ("Log", "info warn error debug setFile"),
        ("Json", "encode decode pretty"),
        ("Http", "get post request"),
        (
            "String",
            "trim split join contains startsWith endsWith replace substring lower upper format",
        ),
        ("Math", "abs min max floor ceil round"),
        ("List", "remove slice concat contains sort"),
        ("Map", "create has get keys values remove"),
    ];
    let mut count = 7;
    for (namespace, functions) in modules {
        for name in functions.split_whitespace() {
            assert!(
                reference.contains(&format!("{namespace}::{name}")),
                "missing {namespace}::{name}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 105);
}
