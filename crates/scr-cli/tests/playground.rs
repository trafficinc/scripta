use std::process::Command;

#[test]
fn missing_companion_has_actionable_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .args(["playground", "--no-open"])
        .env(
            "SCR_PLAYGROUND_BIN",
            "/missing-scripta-playground/scr-playground",
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("separate playground download"));
}

#[cfg(unix)]
#[test]
fn launches_external_companion_with_current_interpreter_and_preserves_exit_status() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    let companion = directory.path().join("companion with spaces");
    fs::write(&companion, "#!/bin/sh\nprintf '%s\\n' \"$@\"\nexit 7\n").unwrap();
    fs::set_permissions(&companion, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .args(["playground", "--no-open"])
        .env("SCR_PLAYGROUND_BIN", companion)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    let text = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(&lines[..2], &["--no-open", "--scr"]);
    assert_eq!(
        std::path::Path::new(lines[2]),
        std::path::Path::new(env!("CARGO_BIN_EXE_scr"))
    );
}
