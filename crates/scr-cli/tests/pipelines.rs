use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

fn piped(args: &[&str], input: &[u8], cwd: &std::path::Path, limit: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scr"));
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(limit) = limit {
        command.env("SCR_MAX_SOURCE_BYTES", limit);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn command_help_is_successful_but_script_arguments_and_escaped_paths_remain_literal() {
    for topic in ["run", "check", "fmt"] {
        for args in [
            vec!["help", topic],
            vec![topic, "--help"],
            vec![topic, "-h"],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_scr"))
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
            assert!(output.stderr.is_empty());
            assert!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .contains(&format!("Usage: scr {topic}"))
            );
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("--help"), "<?scr echo Cli::arg(0);").unwrap();
    let output = piped(&["run", "--", "--help", "--help"], b"", dir.path(), None);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"--help");
    std::fs::write(dir.path().join("-"), "<?scr echo \"literal\";").unwrap();
    let output = piped(&["run", "--", "-"], b"", dir.path(), None);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"literal");
}

#[test]
fn stdin_source_supports_arguments_includes_eof_and_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("helper.scr"),
        "<?scr function hello() { return \"hello\"; }",
    )
    .unwrap();
    let source = b"<?scr require_once \"helper.scr\"; echo hello() . Cli::arg(0); if (Cli::readLine() !== null) exit 9; exit 7;";
    let output = piped(&["run", "-", "--help"], source, dir.path(), None);
    assert_eq!(output.status.code(), Some(7), "{:?}", output);
    assert_eq!(output.stdout, b"hello--help");
    assert!(output.stderr.is_empty());
    let output = piped(&["run", "-"], b"<?scr echo $missing;", dir.path(), None);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("<stdin>:1:"));
}

#[test]
fn stdin_source_enforces_tags_utf8_and_source_budget() {
    let dir = tempfile::tempdir().unwrap();
    for source in [b"echo 1;".as_slice(), b"<?scr \xff", b"<?scr echo"] {
        let output = piped(&["run", "-"], source, dir.path(), None);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
    let source = b"<?scr echo \"ok\";";
    for (limit, expected) in [(source.len(), 0), (source.len() - 1, 1)] {
        let output = piped(&["run", "-"], source, dir.path(), Some(&limit.to_string()));
        assert_eq!(output.status.code(), Some(expected));
        if expected == 1 {
            assert!(String::from_utf8_lossy(&output.stderr).contains("source byte limit"));
        }
    }
}

#[test]
fn file_script_consumes_piped_records_and_streaming_example_matches_file_mode() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("playground/streaming.scr");
    let data = root.join("playground/sample-events.txt");
    let output = piped(
        &["run", script.to_str().unwrap(), "-"],
        &std::fs::read(&data).unwrap(),
        &root,
        None,
    );
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let file = Command::new(env!("CARGO_BIN_EXE_scr"))
        .args(["run", script.to_str().unwrap(), data.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(file.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(file.stdout)
            .unwrap()
            .replace("Stream closed: true\n", "")
    );
}
