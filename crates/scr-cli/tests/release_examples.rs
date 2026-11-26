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
    Command::new(env!("CARGO_BIN_EXE_scripta"))
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
fn hashing_example_streams_empty_and_binary_files_with_known_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("payload.bin");
    // Expected digests/tags independently generated with Python hashlib/hmac.
    for (data, expected) in [
        (
            Vec::new(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nkjWYym1krypdunnc0CGooP5cX1V1Ga2q8K1TLUUG3TA=\n",
        ),
        (
            (0..50000).map(|i| (i % 256) as u8).collect(),
            "9d3550b2e0ae28ea766fd775454403cd4888c27509cb10d1be190f89b3f1decd\nBSBKVqXVAZCPnyN9tilKbYpyKH/vQBiDHFfDL694Seo=\n",
        ),
    ] {
        fs::write(&input, data).unwrap();
        assert_eq!(
            success(run("hashing.scripta", &[input.as_os_str()])),
            expected
        );
    }
    let missing = run("hashing.scripta", &[]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        String::from_utf8(missing.stdout)
            .unwrap()
            .contains("Usage:")
    );
}
#[test]
fn every_example_parses_and_simple_examples_run_without_external_runtimes() {
    let names = [
        "hello.scripta",
        "mysql.scripta",
        "postgres.scripta",
        "html-escaping.scripta",
        "password-hashing.scripta",
        "variables.scripta",
        "string-interpolation.scripta",
        "loops.scripta",
        "functions.scripta",
        "interfaces.scripta",
        "exceptions.scripta",
        "file-organizer.scripta",
        "log-cleaner.scripta",
        "directory-backup.scripta",
        "server-health.scripta",
        "http-client.scripta",
        "http-timing.scripta",
        "timezones.scripta",
        "web-app.scripta",
        "web-sessions.scripta",
        "web-uploads.scripta",
        "web-streaming.scripta",
        "binary-streams.scripta",
        "http-stream-upload.scripta",
        "socket-echo.scripta",
        "web-transfers.scripta",
        "web-proxy.scripta",
        "command-runner.scripta",
        "atomic-report.scripta",
        "data-handling.scripta",
        "url-encoding.scripta",
        "binary-encoding.scripta",
        "hashing.scripta",
        "secure-random.scripta",
        "process-data.scripta",
        "regex-captures.scripta",
        "csv-reader.scripta",
        "download.scripta",
    ];
    for name in names {
        scr_cli::pipeline::check(&fs::read_to_string(example(name)).unwrap()).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    for (name, expected) in [
        ("hello.scripta", "Hello, Scripta!\n"),
        ("loops.scripta", "1\n2\n3\n"),
        (
            "regex-captures.scripta",
            "ada: 2..16 id=42\nbob: 17..25 id=none\nEmpty capture: 0..0\nNo match: true\n",
        ),
        (
            "secure-random.scripta",
            "Key bytes: 32\nToken characters: 32\nTag verified: true\nChanged bytes equal: false\nEmpty bytes: 0\n",
        ),
        (
            "binary-encoding.scripta",
            "Y2Fmw6k=\ncafé\n+/8A/w==\n-_8A_w\nfbff00ff\nMalformed input rejected; destination unchanged\n",
        ),
        (
            "string-interpolation.scripta",
            "Hello, Ada!\nFirst item: apple\nTotal: 100; available: true\nLiteral: {$user->name}\n{$user->name}\n",
        ),
        ("functions.scripta", "Hello, Scripta!\n6\n"),
        (
            "exceptions.scripta",
            "MissingEmail: Email is required\nCannot create account\nCause: MissingEmail: Email is required\nValidation complete\n",
        ),
        (
            "interfaces.scripta",
            "Email: Welcome, Ada\nSMS: Welcome, Grace\n",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg(example(name))
            .env("PATH", dir.path())
            .output()
            .unwrap();
        assert_eq!(success(output), expected);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .args(["version", "--verbose"])
        .output()
        .unwrap();
    let text = success(output);
    assert!(
        text.starts_with(&format!("scripta {}\n", env!("CARGO_PKG_VERSION")))
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
        "file-organizer.scripta",
        &[source.as_os_str(), destination.as_os_str()],
    ));
    assert!(source.join("note.txt").exists());
    assert!(!destination.exists());
    success(run(
        "file-organizer.scripta",
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
    success(run("log-cleaner.scripta", &[source.as_os_str()]));
    assert!(old.exists());
    success(run(
        "log-cleaner.scripta",
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
        "directory-backup.scripta",
        &[source.as_os_str(), destination.as_os_str()],
    ));
    assert!(!destination.exists());
    success(run(
        "directory-backup.scripta",
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
            "directory-backup.scripta",
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
            "directory-backup.scripta",
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
fn process_data_example_supplies_stdin_and_encodes_binary_captures() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/process-stdin.scripta");
    let escaped = fixture
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let arguments = format!("[\"{escaped}\"]");
    for (input, stdout_hex) in [
        ("68656c6c6f2070726f636573730a", "68656c6c6f2070726f63657373"),
        ("c3a90a", "c3a9"),
        ("", ""),
    ] {
        let report = success(run(
            "process-data.scripta",
            &[
                env!("CARGO_BIN_EXE_scripta").as_ref(),
                arguments.as_ref(),
                input.as_ref(),
            ],
        ));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&report).unwrap(),
            serde_json::json!({"exitCode":0,"stdoutHex":stdout_hex,"stderrHex":""})
        );
    }
    let missing = run("process-data.scripta", &[]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        String::from_utf8(missing.stdout)
            .unwrap()
            .contains("Usage:")
    );
}
#[test]
fn command_runner_preserves_literal_arguments_and_http_example_uses_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let child = dir.path().join("child.scripta");
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
            "command-runner.scripta",
            &[env!("CARGO_BIN_EXE_scripta").as_ref(), args.as_ref()]
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
    let output = success(run("http-client.scripta", &[url.as_ref()]));
    assert!(output.contains("\"status\": \"up\""));
    worker.join().unwrap();
}

#[test]
fn strings_playground_normalizes_a_unicode_record_without_external_programs() {
    let directory = tempfile::tempdir().unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/strings.scripta");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "info: café_api: READY\nService prefix: café\nContains api: true\nStarts with café: true\nEnds with _api: true\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let result = Command::new(env!("CARGO_BIN_EXE_scripta"))
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
            "collections.scripta",
            vec![],
            r#"Active: auth, café-api
First: auth
Without first: ["café-api"]
With extra: ["auth","café-api","queue"]
Has auth: true
Fields: active, total
Values: [2,4]
Has total: true
Next: frontend
Last: queue
Edited: ["frontend","replacement","café-api","queue"]
Removed: ["auth"]
Reversed: ["queue","café-api","auth","frontend"]
Chunks: [["frontend","auth"],["café-api","queue"]]
Filled: ["pending","pending","pending"]
Unique: [1,1.0,null]
Auth index: 1
Sum: 9
Product: 24
Added: ["frontend","queue"]
Shared: ["auth","café-api"]
Merged: {"region":"local","retries":3,"timeout":20}
Replaced: {"retries":3,"timeout":20}
Combined: {"a":3,"b":2}
Flipped keys: [1,"1"]
Flipped values: ["last","second"]
Timeout key: timeout
Reverse keys: ["region","timeout","retries"]
Different values: {"region":"local","timeout":20}
Shared values: {"retries":3}
New keys: {"region":"local"}
Existing keys: {"retries":3,"timeout":20}
Durations: [12,8,15]
By job: {"api":15,"worker":8}
"#,
        ),
        (
            "log-analyzer.scripta",
            vec![root.join("sample-events.txt")],
            "Services: auth, café-api, worker\ninfo: 3\nwarn: 1\nerror: 1\nMalformed: 1\n",
        ),
    ] {
        let source = fs::read_to_string(root.join(name)).unwrap();
        scr_cli::pipeline::check(&source).unwrap();
        let formatted = scr_cli::formatter::format(&source).unwrap();
        scr_cli::pipeline::check(&formatted).unwrap();
        assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(root.join("log-analyzer.scripta"))
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(
        success(output),
        "Services: api\ninfo: 1\nwarn: 0\nerror: 0\nMalformed: 4\n"
    );
    fs::write(&input, "").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(root.join("log-analyzer.scripta"))
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(
        success(output),
        "Services: \ninfo: 0\nwarn: 0\nerror: 0\nMalformed: 0\n"
    );
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(root.join("log-analyzer.scripta"))
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/errors.scripta");
    let source = fs::read_to_string(&path).unwrap();
    let errors = scr_cli::pipeline::check(&source).unwrap_err();
    assert!(errors.iter().any(|error| error.code == "E0310"
        && &source[error.span.start..error.span.end] == "$number + \"1\""));
    let formatted = scr_cli::formatter::format(&source).unwrap();
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "Missing key: recovered\nArithmetic: recovered\nPreserved value: 10\nType mismatch: recovered\nStill running\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(success(output), expected);
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(&path)
        .arg("--uncaught")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("error[E0401]: IndexError: list index out of bounds"));
    assert!(error.contains("errors.scripta:4:"));
    assert!(error.contains("at <main>"));
}

#[test]
fn functions_playground_runs_with_defaults_variadics_and_captured_callbacks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/functions.scripta");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "Hello, Scripta!\nWelcome, Scripta!\nTotal: 16\nCallback: 14\nCounter: 6, 8\nFresh defaults: [[\"new\"],[\"new\"]]\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(success(output), expected);
}

#[test]
fn reporting_playground_runs_before_and_after_formatting() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/reporting.scripta");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&formatted).unwrap(), formatted);
    let mut output = Vec::new();
    scr_cli::pipeline::run(&formatted, &mut output).unwrap();
    let expected = "Run summary: 4 jobs\nDuration: 8.6–19.5 ms; rounded average: 13 ms\nAverage bounds: 12–13 ms\nAbsolute adjustment: 2 ms\n{complete}: true\n";
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("run")
        .arg(path)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(success(output), expected);
}

#[test]
fn streaming_playground_handles_sample_crlf_empty_files_and_usage() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../playground/streaming.scripta");
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
    let formatted_path = dir.path().join("formatted.scripta");
    fs::write(&formatted_path, formatted).unwrap();
    for (input, expected) in cases {
        let input_path = dir.path().join("events.txt");
        fs::write(&input_path, input).unwrap();
        for script in [&path, &formatted_path] {
            let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
                .arg("run")
                .arg(script)
                .arg(&input_path)
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert_eq!(success(output), expected);
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
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
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
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
    let output = success(run("server-health.scripta", &[dir.path().as_os_str()]));
    assert!(
        output.contains("\"cpuPercent\"")
            && output.contains("\"systemMemory\"")
            && output.contains("\"disk\"")
    );
    let reference = include_str!("../../../docs/standard-library.md");
    let modules = [
        (
            "File",
            "exists read readBytes writeBytes appendBytes write writeAtomic append copy move delete size modified extension openLines readLine close lock unlock",
        ),
        (
            "Dir",
            "exists create createAll files directories entries walk glob remove current change",
        ),
        (
            "Path",
            "join filename extension parent normalize absolute canonical matchesGlob",
        ),
        ("Temp", "file directory close"),
        ("Process", "run spawn shell wait poll kill exists currentId"),
        ("Env", "get set has remove all"),
        ("Cli", "args arg has option isTerminal prompt readLine"),
        (
            "System",
            "platform architecture hostname cpuUsage memoryUsage diskUsage uptime homeDirectory tempDirectory",
        ),
        ("Date", "now format parse sleep"),
        ("Clock", "monotonicMs"),
        ("Log", "info warn error debug setFile"),
        ("Json", "encode decode pretty"),
        ("Html", "escape"),
        ("Password", "hash verify needsRehash"),
        ("Http", "get post request download"),
        (
            "Socket",
            "connect listen accept bindDatagram read write receive sendTo shutdown close setBlocking setTimeout poll localAddress peerAddress setOption getOption",
        ),
        (
            "Stream",
            "open request read write flush close seek tell eof copy send receive",
        ),
        (
            "Url",
            "encodeComponent decodeComponent buildQuery parseQuery parse serialize resolve isValid",
        ),
        ("Form", "encode decode"),
        ("Sqlite", "open execute query close"),
        ("Bytes", "encodeUtf8 decodeUtf8 slice timingSafeEqual"),
        ("Random", "bytes"),
        ("Base64", "encode decode encodeUrl decodeUrl"),
        ("Hex", "encode decode"),
        (
            "Hash",
            "sha256 hmacSha256 sha256Init hmacSha256Init update finalize close",
        ),
        ("Csv", "parse encode open readRow close"),
        ("Regex", "isMatch findAll replace captures capturesAll"),
        (
            "String",
            "trim split join contains startsWith endsWith replace substring lower upper format graphemeLength graphemeSubstring",
        ),
        ("Math", "abs min max floor ceil round"),
        (
            "List",
            "remove slice concat contains sort push unshift pop shift splice reverse chunk fill search unique sum product diff intersect column",
        ),
        (
            "Map",
            "create has get keys values remove merge replace combine flip search reverse diff intersect diffKeys intersectKeys column",
        ),
    ];
    let output_functions = "flush";
    for name in output_functions.split_whitespace() {
        assert!(reference.contains(&format!("`{name}(")), "missing {name}");
    }
    let mut count = 7 + output_functions.split_whitespace().count();
    for (namespace, functions) in modules {
        for name in functions.split_whitespace() {
            assert!(
                reference.contains(&format!("{namespace}::{name}")),
                "missing {namespace}::{name}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 226);
    for name in [
        "start",
        "id",
        "get",
        "has",
        "set",
        "remove",
        "all",
        "regenerate",
        "destroy",
        "close",
    ] {
        assert!(
            reference.contains(&format!("Session::{name}")),
            "missing Session::{name}"
        );
    }
    for name in [
        "request",
        "status",
        "header",
        "addHeader",
        "redirect",
        "write",
        "download",
    ] {
        assert!(
            reference.contains(&format!("Web::{name}")),
            "missing Web::{name}"
        );
    }
}

#[test]
fn atomic_report_previews_and_publishes_complete_json_before_and_after_formatting() {
    let source = fs::read_to_string(example("atomic-report.scripta")).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(source, formatted);
    for source in [source, formatted] {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("report.scripta");
        let destination = dir.path().join("report.json");
        fs::write(&script, source).unwrap();
        let invoke = |apply: bool| {
            let mut command = Command::new(env!("CARGO_BIN_EXE_scripta"));
            command
                .current_dir(dir.path())
                .arg(&script)
                .arg(&destination)
                .env("PATH", dir.path());
            if apply {
                command.arg("--apply");
            }
            success(command.output().unwrap())
        };
        assert!(invoke(false).starts_with("Would atomically write:"));
        assert!(!destination.exists());
        assert!(invoke(true).starts_with("Updated report:"));
        fs::write(&destination, "obsolete contents").unwrap();
        invoke(true);
        let mut rt = scr_runtime::Runtime::new();
        scr_stdlib::register(&mut rt).unwrap();
        rt.set_global(
            "json",
            scr_runtime::Value::String(fs::read_to_string(&destination).unwrap()),
        )
        .unwrap();
        let mut output = Vec::new();
        rt.execute_source("<?scr $report=Json::decode($json); echo $report['status']; echo type($report['generatedAt']);", &mut output).unwrap();
        assert_eq!(output, b"okint");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}

#[test]
fn data_and_csv_examples_run_without_external_programs() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(example("data-handling.scripta"))
        .current_dir(directory.path())
        .env("PATH", directory.path())
        .output()
        .unwrap();
    let text = success(output);
    assert!(text.starts_with("UTF-8 bytes: 14\nFirst grapheme: 👍🏽\n👍🏽 café\n"));
    assert!(text.ends_with("[\"job-12\",\"job-34\"]\nwarning: retry\n"));
    let file = directory.path().join("records.csv");
    fs::write(&file, "name,note\r\nAda,\"first\nsecond\"\r\n").unwrap();
    assert_eq!(
        success(run("csv-reader.scripta", &[file.as_os_str()])),
        "[\"name\",\"note\"]\n[\"Ada\",\"first\\nsecond\"]\n"
    );
    for name in ["csv-reader.scripta", "download.scripta"] {
        let output = run(name, &[]);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .starts_with("Usage:")
        );
    }
}

#[test]
fn download_example_saves_binary_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("download.bin");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
        }
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n\x00\xffPK",
            )
            .unwrap();
    });
    assert_eq!(
        success(run(
            "download.scripta",
            &[url.as_ref(), destination.as_os_str()]
        )),
        "Downloaded 4 bytes\n"
    );
    worker.join().unwrap();
    assert_eq!(fs::read(destination).unwrap(), b"\x00\xffPK");
}

#[test]
fn url_encoding_example_executes_with_expected_outputs() {
    assert_eq!(
        success(run("url-encoding.scripta", &[])),
        "https://example.com/search?q=caf%C3%A9%20%26%20tea&page=2\n[[\"tag\",\"rust\"],[\"tag\",\"scripting\"]]\nname=Jane+Doe&message=A%2BB\ntrue\n"
    );
}

#[test]
fn http_timing_example_reports_local_success_and_failed_attempts() {
    let inspect = |text: String, expression: &str| {
        let mut runtime = scr_runtime::Runtime::new();
        scr_stdlib::register(&mut runtime).unwrap();
        runtime
            .set_global("json", scr_runtime::Value::String(text))
            .unwrap();
        runtime
            .execute_source(
                &format!("<?scripta $report = Json::decode($json); $result = {expression};"),
                &mut Vec::new(),
            )
            .unwrap();
        runtime.get_global("result").unwrap().clone()
    };
    let output = success(run("http-timing.scripta", &[]));
    assert_eq!(
        inspect(
            output,
            "$report['operation'] === 'sleep' && type($report['elapsedMs']) === 'float' && $report['elapsedMs'] >= 0.0"
        ),
        scr_runtime::Value::Bool(true)
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let mut buffer = [0; 1024];
                        let count = stream.read(&mut buffer).unwrap();
                        assert!(count > 0, "request ended before its headers");
                        request.extend_from_slice(&buffer[..count]);
                        assert!(request.len() <= 16 * 1024);
                    }
                    assert!(request.starts_with(b"GET / "));
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .unwrap();
                    break;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(2))
                }
                other => panic!("HTTP timing example accept failed: {other:?}"),
            }
        }
    });
    let output = success(run("http-timing.scripta", &[url.as_ref()]));
    worker.join().unwrap();
    assert_eq!(
        inspect(
            output,
            "$report['ok'] && $report['status'] === 200 && type($report['durationMs']) === 'float' && $report['durationMs'] >= 0.0 && $report['elapsedMs'] >= $report['durationMs']"
        ),
        scr_runtime::Value::Bool(true)
    );
    let output = run("http-timing.scripta", &["not-a-url".as_ref()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        inspect(
            String::from_utf8(output.stdout).unwrap(),
            "$report['ok'] === false && type($report['elapsedMs']) === 'float' && $report['elapsedMs'] >= 0.0 && type($report['error']) === 'string'"
        ),
        scr_runtime::Value::Bool(true)
    );
}

#[test]
fn html_example_escapes_untrusted_text_and_quoted_attribute_values() {
    let source = fs::read_to_string(example("html-escaping.scripta")).unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    for (input, escaped) in [
        ("", ""),
        (
            "\"><script>alert('x')</script> & 世界",
            "&quot;&gt;&lt;script&gt;alert(&#039;x&#039;)&lt;/script&gt; &amp; 世界",
        ),
    ] {
        let expected = format!(
            "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>HTML escaping</title><p title=\"{escaped}\">Hello, {escaped}!</p></html>\n"
        );
        assert_eq!(
            success(run("html-escaping.scripta", &[std::ffi::OsStr::new(input)])),
            expected
        );
    }
    assert!(
        success(run("html-escaping.scripta", &[]))
            .contains("Ada &lt;Admin&gt; &amp; &quot;friends&quot;")
    );
}

#[test]
fn password_example_hashes_and_verifies_without_exposing_the_password_or_hash() {
    let source = fs::read_to_string(example("password-hashing.scripta")).unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    assert_eq!(
        success(run("password-hashing.scripta", &[])),
        "{\"algorithm\":\"argon2id\",\"needsRehash\":false,\"verified\":true}\n"
    );
}

#[test]
fn binary_stream_example_copies_bytes_and_refuses_existing_output() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input é.bin");
    let output = directory.path().join("output.bin");
    fs::write(&input, [0, 255, 7]).unwrap();
    assert_eq!(
        success(run(
            "binary-streams.scripta",
            &[input.as_os_str(), output.as_os_str()]
        )),
        "{\"bytesCopied\":3}\n"
    );
    assert_eq!(fs::read(&output).unwrap(), [0, 255, 7]);
    assert!(
        !run(
            "binary-streams.scripta",
            &[input.as_os_str(), output.as_os_str()]
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&output).unwrap(), [0, 255, 7]);
}
#[test]
fn streamed_http_upload_example_runs_against_a_controlled_local_server() {
    use std::io::{BufRead, BufReader};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut socket = loop {
            if let Ok((socket, _)) = listener.accept() {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                break BufReader::new(socket);
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2))
        };
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            socket.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            assert!(!line.is_empty());
            headers.push_str(&line)
        }
        assert!(headers.starts_with("POST /upload "));
        assert!(
            headers
                .to_lowercase()
                .contains("transfer-encoding: chunked")
        );
        let mut bytes = vec![];
        loop {
            let mut line = String::new();
            socket.read_line(&mut line).unwrap();
            let n = usize::from_str_radix(line.trim(), 16).unwrap();
            if n == 0 {
                socket.read_exact(&mut [0; 2]).unwrap();
                break;
            }
            let start = bytes.len();
            bytes.resize(start + n, 0);
            socket.read_exact(&mut bytes[start..]).unwrap();
            socket.read_exact(&mut [0; 2]).unwrap();
        }
        assert_eq!(bytes, [0, 255, 7]);
        socket
            .get_mut()
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.bin");
    fs::write(&input, [0, 255, 7]).unwrap();
    let url = format!("http://{address}/upload");
    let report = success(run(
        "http-stream-upload.scripta",
        &[url.as_ref(), input.as_os_str()],
    ));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&report).unwrap(),
        serde_json::json!({"status":200,"responseBytes":2})
    );
    worker.join().unwrap();
}
#[test]
fn socket_echo_example_runs_with_binary_data_on_an_ephemeral_loopback_port() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let stdout = directory.path().join("stdout");
    let stderr = directory.path().join("stderr");
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg(example("socket-echo.scripta"))
            .arg("0")
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let address = loop {
        let text = fs::read_to_string(&stdout).unwrap();
        if text.ends_with('\n') {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            break format!("127.0.0.1:{}", value["port"].as_u64().unwrap());
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "{}",
            fs::read_to_string(&stderr).unwrap()
        );
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut peer = std::net::TcpStream::connect(address).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    peer.write_all(&[0, 255, 7]).unwrap();
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    let mut bytes = vec![];
    peer.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 255, 7]);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.success(), "{}", fs::read_to_string(&stderr).unwrap());
}

#[test]
fn timezone_example_runs_with_only_bundled_data_and_no_external_programs() {
    let directory = tempfile::tempdir().unwrap();
    let script = example("timezones.scripta");
    let original = fs::read_to_string(&script).unwrap();
    let formatted = scr_cli::formatter::format(&original).unwrap();
    assert_eq!(original, formatted);
    let formatted_path = directory.path().join("timezones.scripta");
    fs::write(&formatted_path, formatted).unwrap();
    for script in [&script, &formatted_path] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_scripta"));
        command
            .arg(script)
            .current_dir(directory.path())
            .env_clear()
            .env("PATH", directory.path())
            .env("TZDIR", directory.path());
        for name in ["SystemRoot", "WINDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        assert_eq!(
            success(command.output().unwrap()),
            "UTC: 2024-07-15 12:34:56 +00:00\nAmerica/New_York: 2024-07-15 08:34:56 -04:00\nAsia/Kathmandu: 2024-07-15 18:19:56 +05:45\n"
        );
    }
}

#[test]
fn discovery_example_filters_files_and_skips_hidden_subtrees() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::create_dir(dir.path().join(".private")).unwrap();
    for name in [
        "a.scripta",
        "nested/é.scripta",
        "nested/other.txt",
        ".private/hidden.scripta",
    ] {
        fs::write(dir.path().join(name), "").unwrap();
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/file-discovery.scripta");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    let formatted = scr_cli::formatter::format(&source).unwrap();
    assert_eq!(formatted, source);
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("run")
        .arg(path)
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(success(output), "a.scripta\nnested/é.scripta\n");
}

#[test]
fn temporary_resource_example_locks_reads_and_cleans_its_owned_paths() {
    let directory = tempfile::tempdir().unwrap();
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/temporary-resources.scripta");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("run")
        .arg(path)
        .arg(directory.path())
        .output()
        .unwrap();
    assert_eq!(
        success(output),
        "Lock: exclusive\nContent: temporary work\nCleaned: true\n"
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn full_url_example_normalizes_hosts_and_resolves_without_network_requests() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/url-handling.scripta");
    let source = fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("run")
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(
        success(output),
        concat!(
            "Base: https://xn--bcher-kva.example/reports/2026/index.html?tag=old#top\n",
            "Resolved: https://xn--bcher-kva.example/reports/summary?tag=rust&tag=scripting#results\n",
            "Host: xn--bcher-kva.example\nPort: 443\n",
            "Tags: [[\"tag\",\"rust\"],[\"tag\",\"scripting\"]]\nValid: true\n"
        )
    );
}
