use std::process::Command;

#[test]
fn source_only_linux_arm64_cannot_produce_a_release_archive() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scr-release"))
        .args(["package", "--target", "aarch64-unknown-linux-gnu"])
        .arg("--binary")
        .arg(directory.path().join("missing-binary"))
        .arg("--out-dir")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Linux ARM64 is source-only"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
