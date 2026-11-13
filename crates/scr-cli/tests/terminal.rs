use std::{
    fs,
    io::IsTerminal,
    process::{Command, Stdio},
};

const QUERY: &str = r#"<?scr
echo str(Cli::isTerminal("stdin")) . ":";
echo str(Cli::isTerminal("stdout")) . ":";
echo str(Cli::isTerminal("stderr")) . "\n";
"#;

#[test]
fn terminal_detection_distinguishes_inherited_and_redirected_handles() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("terminal.scr");
    fs::write(&script, QUERY).unwrap();
    for inherit in [false, true] {
        let output = Command::new(env!("CARGO_BIN_EXE_scr"))
            .arg(&script)
            .stdin(if inherit {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .stderr(if inherit {
                Stdio::inherit()
            } else {
                Stdio::piped()
            })
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "{}:false:{}\n",
                inherit && std::io::stdin().is_terminal(),
                inherit && std::io::stderr().is_terminal()
            )
        );
    }
}

#[test]
fn terminal_detection_returns_false_for_redirected_files() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("terminal.scr");
    let input = directory.path().join("input.txt");
    let output = directory.path().join("output.txt");
    let error = directory.path().join("error.txt");
    fs::write(&script, QUERY).unwrap();
    fs::write(&input, "input remains unread\n").unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(script)
        .stdin(fs::File::open(input).unwrap())
        .stdout(fs::File::create(&output).unwrap())
        .stderr(fs::File::create(&error).unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read(output).unwrap(), b"false:false:false\n");
    assert!(fs::read(error).unwrap().is_empty());
}
