use std::process::Command;

#[test]
fn sqlite_feature_controls_cli_registration() {
    let version = Command::new(env!("CARGO_BIN_EXE_scr"))
        .args(["version", "--verbose"])
        .output()
        .unwrap();
    let expected = if cfg!(feature = "sqlite") {
        "extensions: sqlite"
    } else {
        "extensions: none"
    };
    assert!(
        String::from_utf8_lossy(&version.stdout)
            .lines()
            .any(|line| line == expected)
    );
    let script = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        script.path(),
        "<?scr $db = Sqlite::open(':memory:'); echo 'enabled'; Sqlite::close($db);",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(script.path())
        .output()
        .unwrap();
    if cfg!(feature = "sqlite") {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"enabled");
    } else {
        assert_eq!(output.status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("undefined function Sqlite::open")
        );
    }
}

#[test]
#[cfg(feature = "sqlite")]
fn sqlite_example_and_repeated_pipeline_runs() {
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/sqlite.scr"
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, "1: Ada\n2: 世界\n".as_bytes());
    for _ in 0..2 {
        let mut output = Vec::new();
        scr_cli::pipeline::run("<?scr $db = Sqlite::open(':memory:'); echo Sqlite::query($db, 'SELECT ? AS value', ['hello'])[0]['value']; Sqlite::close($db);", &mut output).unwrap();
        assert_eq!(output, b"hello");
    }
}
