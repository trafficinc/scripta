use std::io::Write;
use std::process::{Command, Stdio};

const CONNECT: &str = "Db::connect('mysql:host=;dbname=unused','fixture','secret');";
fn expected_kind() -> &'static str {
    if cfg!(feature = "mysql") {
        "TypeError"
    } else {
        "DbUnsupportedError"
    }
}
fn assert_error(message: &str) {
    assert!(message.contains(expected_kind()), "{message}");
    assert!(!message.contains("secret"), "{message}");
    if !cfg!(feature = "mysql") {
        assert!(message.contains("--features mysql"), "{message}");
        assert!(message.contains("version --verbose"), "{message}");
    }
}

#[test]
fn database_discovery_uses_registered_descriptor_versions_and_capabilities_without_io() {
    let info = scr_cli::extensions::compiled();
    assert_eq!(
        info.iter().any(|i| i.name == "mysql"),
        cfg!(feature = "mysql")
    );
    assert_eq!(
        info.iter().any(|i| i.name == "sqlite"),
        cfg!(feature = "sqlite")
    );
    #[cfg(feature = "mysql")]
    {
        let mysql = scr_ext_mysql::driver().info();
        let entry = info.iter().find(|i| i.name == mysql.name).unwrap();
        assert_eq!(entry.version, mysql.version);
        assert_eq!(entry.capabilities, mysql.capabilities);
        assert!(entry.capabilities.contains(&"transactions.state"));
        assert!(entry.capabilities.contains(&"tls.verifyIdentity"));
    }
    let mut outputs = Vec::new();
    for binary in [env!("CARGO_BIN_EXE_scripta"), env!("CARGO_BIN_EXE_scr")] {
        let output = Command::new(binary)
            .args(["version", "--verbose"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        let names = if info.is_empty() {
            "none".into()
        } else {
            info.iter().map(|i| i.name).collect::<Vec<_>>().join(", ")
        };
        assert!(
            text.lines()
                .any(|line| line == format!("extensions: {names}")),
            "{text}"
        );
        for item in &info {
            let expected = format!(
                "extension {}: version={} capabilities={}",
                item.name,
                item.version,
                item.capabilities.join(",")
            );
            assert!(text.lines().any(|line| line == expected), "{text}");
        }
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("extension "))
                .count(),
            info.len()
        );
        outputs.push(text);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn database_check_has_signatures_without_claiming_availability_and_embedding_matches_cli() {
    // The DSN is invalid: enabled drivers reject it before opening any socket.
    // Static checking must not validate a DSN, connect, or claim a driver exists.
    scr_cli::pipeline::check(CONNECT).unwrap();
    assert!(scr_cli::pipeline::check("Db::connect(1, 'u', 'p');").is_err());
    for _ in 0..2 {
        let error = scr_cli::pipeline::run(CONNECT, &mut Vec::new()).unwrap_err();
        assert_error(&error.message);
    }
    let error = scr_cli::pipeline::run_with_input(
        CONNECT,
        &mut Vec::new(),
        Default::default(),
        std::io::empty(),
    )
    .unwrap_err();
    assert_error(&error.message);
    let mut runtime = scr_runtime::Runtime::new();
    scr_cli::extensions::register(&mut runtime).unwrap();
    let error = runtime
        .execute_source(CONNECT, &mut Vec::new())
        .unwrap_err();
    assert_error(&error.message);
}

#[test]
fn database_file_stdin_and_compatibility_command_report_the_same_driver_state() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("database.scripta");
    std::fs::write(&file, CONNECT).unwrap();
    for binary in [env!("CARGO_BIN_EXE_scripta"), env!("CARGO_BIN_EXE_scr")] {
        let check = Command::new(binary)
            .arg("check")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let run = Command::new(binary).arg("run").arg(&file).output().unwrap();
        assert_eq!(run.status.code(), Some(1));
        assert_error(&String::from_utf8_lossy(&run.stderr));
        let mut stdin = Command::new(binary)
            .args(["run", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        stdin
            .stdin
            .take()
            .unwrap()
            .write_all(CONNECT.as_bytes())
            .unwrap();
        let output = stdin.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_error(&String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
#[cfg(feature = "mysql")]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn mysql_live_file_stdin_compatibility_and_embedding_execute_real_queries() {
    assert!(
        std::env::var_os("SCR_MYSQL_FIXTURE_CA").is_some(),
        "run mysql_fixture.py"
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mysql-cli.scripta");
    let source = std::fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    for binary in [env!("CARGO_BIN_EXE_scripta"), env!("CARGO_BIN_EXE_scr")] {
        let output = Command::new(binary).arg("run").arg(&path).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"mysql-cli-ok\n");
        let mut child = Command::new(binary)
            .args(["run", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"mysql-cli-ok\n");
    }
    let mut output = Vec::new();
    scr_cli::pipeline::run(&source, &mut output).unwrap();
    assert_eq!(output, b"mysql-cli-ok\n");
    output.clear();
    scr_cli::pipeline::run_with_input(&source, &mut output, Default::default(), std::io::empty())
        .unwrap();
    assert_eq!(output, b"mysql-cli-ok\n");
}

#[test]
#[cfg(feature = "mysql")]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn mysql_live_public_example_runs_with_fixture_credentials() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mysql.scripta");
    let source = std::fs::read_to_string(&path).unwrap();
    scr_cli::pipeline::check(&source).unwrap();
    assert_eq!(scr_cli::formatter::format(&source).unwrap(), source);
    let mut command = Command::new(env!("CARGO_BIN_EXE_scripta"));
    command.arg("run").arg(&path);
    for (target, fixture) in [
        ("SCR_MYSQL_DSN", "SCR_MYSQL_FIXTURE_DSN"),
        ("SCR_MYSQL_USER", "SCR_MYSQL_FIXTURE_USER"),
        ("SCR_MYSQL_PASSWORD", "SCR_MYSQL_FIXTURE_PASSWORD"),
        ("SCR_MYSQL_CA_FILE", "SCR_MYSQL_FIXTURE_CA"),
    ] {
        command.env(
            target,
            std::env::var(fixture).expect("run mysql_fixture.py"),
        );
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"Ada: 2\n");
}

#[test]
fn database_reference_covers_registered_api_in_every_feature_combination() {
    let reference = include_str!("../../../docs/standard-library.md");
    let connector = include_str!("../../../docs/database-connectors.md");
    for name in [
        "connect",
        "getAttribute",
        "setAttribute",
        "prepare",
        "execute",
        "exec",
        "fetch",
        "fetchAll",
        "fetchColumn",
        "rowCount",
        "lastInsertId",
        "beginTransaction",
        "commit",
        "rollBack",
        "inTransaction",
        "closeCursor",
        "closeStatement",
        "errorInfo",
        "close",
    ] {
        let function = format!("Db::{name}");
        assert!(
            reference.contains(&format!("{function}(")),
            "missing {function} in standard-library reference"
        );
        assert!(
            connector.contains(&format!("{function}(")),
            "missing {function} in connector reference"
        );
        let error = scr_cli::pipeline::run(&format!("{function}();"), &mut Vec::new()).unwrap_err();
        assert!(
            error.message.starts_with("TypeError:"),
            "{function}: {error:?}"
        );
        assert!(!error.message.contains("undefined function"), "{function}");
    }
}
