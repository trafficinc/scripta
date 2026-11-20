use scr_runtime::{Runtime, RuntimeOptions, SourceFile, Value};
use std::sync::Arc;
#[test]
fn cached_program_keeps_source_and_request_state_separate() {
    let text = "<?scripta function fail() { echo $missing; } $value=1; fail();";
    let program = scr_parser::parse(&scr_lexer::lex(text).unwrap()).unwrap();
    let source = Arc::new(SourceFile {
        path: std::env::current_dir().unwrap().join("cached.scripta"),
        text: text.into(),
    });
    for _ in 0..2 {
        let mut runtime = Runtime::new();
        let error = runtime
            .execute_program_with_source(&program, source.clone(), &mut vec![])
            .unwrap_err();
        assert_eq!(error.source.as_ref().unwrap().text, text);
        assert!(error.render("wrong", "wrong").contains("cached.scripta"));
        assert!(error.trace.iter().any(|f| f.function == "fail"));
        assert_eq!(runtime.get_global("value"), Some(&Value::Int(1)));
        // Cached execution restores the previous source association.
        let next = runtime
            .execute_source("<?scripta echo $another;", &mut vec![])
            .unwrap_err();
        assert_eq!(
            next.source.as_ref().unwrap().text,
            "<?scripta echo $another;"
        );
    }
}
#[test]
fn cached_source_obeys_each_runtimes_source_ceiling_before_execution() {
    let text = "<?scripta echo \"ok\";";
    let program = scr_parser::parse(&scr_lexer::lex(text).unwrap()).unwrap();
    let source = Arc::new(SourceFile {
        path: "cached.scripta".into(),
        text: text.into(),
    });
    let mut exact = Runtime::with_options(RuntimeOptions {
        max_source_bytes: text.len(),
        ..Default::default()
    });
    let mut output = vec![];
    exact
        .execute_program_with_source(&program, source.clone(), &mut output)
        .unwrap();
    assert_eq!(output, b"ok");
    let mut small = Runtime::with_options(RuntimeOptions {
        max_source_bytes: text.len() - 1,
        ..Default::default()
    });
    let mut output = vec![];
    assert!(
        small
            .execute_program_with_source(&program, source, &mut output)
            .unwrap_err()
            .message
            .contains("source byte limit")
    );
    assert!(output.is_empty());
}
