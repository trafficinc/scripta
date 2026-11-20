use std::process::Command;

#[test]
fn runs_script_from_disk() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/hello.scripta"
        ))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"Hello, Scripta!\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn usage_and_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("Usage:"));
}

#[test]
fn source_errors_reach_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/invalid-concat.scripta"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid-concat.scripta:2:6"));
    assert!(stderr.contains("concatenation requires two strings"));
}

#[test]
fn named_run_matches_shorthand() {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.scripta");
    for args in [vec![file], vec!["run", file], vec!["run", "--", file]] {
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, b"Hello, Scripta!\n");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn named_help_and_version() {
    for arg in ["help", "--help", "-h", "version", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg(arg)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        if arg.contains("version") {
            assert_eq!(stdout, format!("scripta {}\n", env!("CARGO_PKG_VERSION")));
        } else {
            assert!(stdout.contains("scripta check <file>"));
        }
    }
}

#[test]
fn incorrect_usage_exits_two() {
    for args in [
        vec![],
        vec!["run"],
        vec!["check"],
        vec!["check", "one.scripta", "two.scripta"],
        vec!["help", "extra"],
        vec!["version", "extra"],
        vec!["--unknown"],
        vec!["run", "--unknown"],
        vec!["check", "--unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr).unwrap().contains("Usage:"));
    }
}

#[test]
fn check_succeeds_without_output_or_runtime_evaluation() {
    for relative in [
        "examples/hello.scripta",
        "tests/fixtures/statements.scripta",
        "tests/fixtures/invalid-concat.scripta",
        "tests/fixtures/lexer-only.scripta",
    ] {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg("check")
            .arg(file)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn check_rejects_lexical_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("check")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unterminated-comment.scripta"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unterminated-comment.scripta:2:1"));
    assert!(stderr.contains("unterminated block comment"));
}

#[test]
fn unreadable_file_exits_one() {
    // A regular file cannot contain child paths, even if other test files exist.
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Cargo.toml")
        .join("missing.scripta");
    for command in ["run", "check"] {
        let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
            .arg(command)
            .arg(&file)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("error[E")
        );
    }
}

#[test]
fn check_rejects_missing_semicolons() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("check")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/missing-semicolon.scripta"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("expected Semicolon")
    );
}

#[test]
fn check_renders_every_recovered_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("check")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/statement-errors.scripta"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.starts_with("error["))
            .count(),
        4,
        "{stderr}"
    );
    for line in 2..=5 {
        assert!(
            stderr.contains(&format!("statement-errors.scripta:{line}:")),
            "{stderr}"
        );
    }
}

#[test]
fn run_prints_hello_world_and_returns_script_exit_codes() {
    for (fixture, code, stdout) in [
        ("runtime-hello.scripta", 0, "Hello World\n"),
        ("runtime-exit.scripta", 7, "before\n"),
        ("statements.scripta", 0, "Starting...\nSuccess\n1\n2\n3\n"),
    ] {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(fixture);
        for command in ["run", "check"] {
            let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
                .arg(command)
                .arg(&file)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(if command == "run" { code } else { 0 }),
                "{fixture}"
            );
            assert_eq!(
                output.stdout,
                if command == "run" {
                    stdout.as_bytes()
                } else {
                    b""
                }
            );
            assert!(output.stderr.is_empty(), "{fixture}");
        }
    }
}

#[test]
fn run_reports_nested_stack_traces_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .arg("run")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/runtime-stack-trace.scripta"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("runtime-stack-trace.scripta:3:12"),
        "{stderr}"
    );
    assert!(
        stderr.contains("at inner (") && stderr.contains("runtime-stack-trace.scripta:6:12)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("at outer (") && stderr.contains("runtime-stack-trace.scripta:8:1)"),
        "{stderr}"
    );
}

#[test]
fn primary_and_compatibility_commands_accept_all_source_extensions() {
    let dir = tempfile::tempdir().unwrap();
    for binary in [env!("CARGO_BIN_EXE_scripta"), env!("CARGO_BIN_EXE_scr")] {
        let version = Command::new(binary).arg("version").output().unwrap();
        assert!(version.status.success());
        assert_eq!(
            String::from_utf8(version.stdout).unwrap(),
            format!("scripta {}\n", env!("CARGO_PKG_VERSION"))
        );
        for (tag, helper_tag) in [("<?scripta", "<?scr"), ("<?scr", "<?scripta")] {
            std::fs::write(
                dir.path().join("helper.scripta"),
                format!("{helper_tag} function greeting(): string {{ return 'Hello from '; }}"),
            )
            .unwrap();
            for extension in ["scripta", "scta", "scr"] {
                let path = dir.path().join(format!("backup script.{extension}"));
                std::fs::write(&path,format!("{tag} require_once 'helper.scripta';$name=Cli::arg(0);echo greeting().$name;exit 7;")).unwrap();
                let format = Command::new(binary).arg("fmt").arg(&path).output().unwrap();
                assert!(
                    format.status.success(),
                    "{}",
                    String::from_utf8_lossy(&format.stderr)
                );
                for args in [vec!["check"], vec!["fmt", "--check"]] {
                    let output = Command::new(binary).args(args).arg(&path).output().unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                for prefix in [vec![], vec!["run"]] {
                    let output = Command::new(binary)
                        .args(prefix)
                        .arg(&path)
                        .arg("Scripta")
                        .output()
                        .unwrap();
                    assert_eq!(
                        output.status.code(),
                        Some(7),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert_eq!(output.stdout, b"Hello from Scripta");
                }
            }
        }
    }
}
