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
            "exists read write append copy move delete size modified extension",
        ),
        (
            "Dir",
            "exists create createAll files directories remove current change",
        ),
        ("Path", "join filename extension parent normalize absolute"),
        ("Process", "run spawn shell wait poll kill exists currentId"),
        ("Env", "get set has remove all"),
        ("Cli", "args arg has option prompt"),
        (
            "System",
            "platform architecture hostname cpuUsage memoryUsage diskUsage uptime homeDirectory tempDirectory",
        ),
        ("Date", "now format parse sleep"),
        ("Log", "info warn error debug setFile"),
        ("Json", "encode decode pretty"),
        ("Http", "get post request"),
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
    assert_eq!(count, 73);
}
