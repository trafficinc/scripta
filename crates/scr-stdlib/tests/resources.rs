use scr_runtime::{ContextOptions, Runtime, RuntimeOptions, Value};
#[test]
fn bounded_file_reads_release_the_file_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("data.txt");
    std::fs::write(&file, vec![b'x'; 4096]).unwrap();
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(512),
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
        .set_global("path", Value::String(file.to_str().unwrap().into()))
        .unwrap();
    let error = runtime
        .execute_source("<?scr $value=File::read($path);", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("byte limit"));
    std::fs::remove_file(file).unwrap();
    runtime
        .execute_source("<?scr echo 'recovered';", &mut Vec::new())
        .unwrap();
}
#[test]
fn required_files_obey_the_configured_source_ceiling() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("large.scripta"),
        "<?scr echo 'a fairly large included script';",
    )
    .unwrap();
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_source_bytes: 32,
        context: ContextOptions {
            script_path: Some(directory.path().join("main.scripta")),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    let error = runtime
        .execute_source("<?scr require 'large.scripta';", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("source byte limit"));
}
