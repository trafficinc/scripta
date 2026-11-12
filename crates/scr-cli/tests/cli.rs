use std::process::Command;

#[test]
fn runs_script_from_disk() {
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/hello.scr"
        ))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"Hello, Scripta!\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn usage_and_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_scr")).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("Usage:"));
}

#[test]
fn source_errors_reach_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/invalid-concat.scr"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid-concat.scr:2:6"));
    assert!(stderr.contains("concatenation requires two strings"));
}

#[test]
fn named_run_matches_shorthand() {
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.scr");
    for args in [vec![file], vec!["run", file], vec!["run", "--", file]] {
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
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
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
            .arg(arg)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        if arg.contains("version") {
            assert_eq!(stdout, format!("scr {}\n", env!("CARGO_PKG_VERSION")));
        } else {
            assert!(stdout.contains("scr check <file>"));
        }
    }
}

#[test]
fn incorrect_usage_exits_two() {
    for args in [
        vec![],
        vec!["run"],
        vec!["check"],
        vec!["check", "one.scr", "two.scr"],
        vec!["help", "extra"],
        vec!["version", "extra"],
        vec!["--unknown"],
        vec!["run", "--unknown"],
        vec!["check", "--unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
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
        "examples/hello.scr",
        "tests/fixtures/statements.scr",
        "tests/fixtures/invalid-concat.scr",
        "tests/fixtures/lexer-only.scr",
    ] {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative);
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("check")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unterminated-comment.scr"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unterminated-comment.scr:2:1"));
    assert!(stderr.contains("unterminated block comment"));
}

#[test]
fn unreadable_file_exits_one() {
    // A regular file cannot contain child paths, even if other test files exist.
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Cargo.toml")
        .join("missing.scr");
    for command in ["run", "check"] {
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("check")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/missing-semicolon.scr"
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
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("check")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/statement-errors.scr"
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
            stderr.contains(&format!("statement-errors.scr:{line}:")),
            "{stderr}"
        );
    }
}

#[test]
fn run_prints_hello_world_and_returns_script_exit_codes() {
    for (fixture, code, stdout) in [
        ("runtime-hello.scr", 0, "Hello World\n"),
        ("runtime-exit.scr", 7, "before\n"),
        ("statements.scr", 0, "Starting...\nSuccess\n1\n2\n3\n"),
    ] {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(fixture);
        for command in ["run", "check"] {
            let output = Command::new(env!("CARGO_BIN_EXE_scr"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg("run")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/runtime-stack-trace.scr"
        ))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("runtime-stack-trace.scr:3:12"), "{stderr}");
    assert!(
        stderr.contains("at inner (") && stderr.contains("runtime-stack-trace.scr:6:12)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("at outer (") && stderr.contains("runtime-stack-trace.scr:8:1)"),
        "{stderr}"
    );
}
