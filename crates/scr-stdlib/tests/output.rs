use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value};
use std::io::{self, Write};
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    flushes: Vec<Vec<u8>>,
    fail: bool,
    fail_flush: bool,
}
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.fail {
            return Err(io::Error::other("fixture write failure"));
        }
        let n = b.len().min(2);
        self.bytes.extend_from_slice(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("fixture flush failure"));
        }
        self.flushes.push(self.bytes.clone());
        Ok(())
    }
}
fn runtime(options: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(
    runtime: &mut Runtime,
    source: &str,
    out: &mut impl Write,
) -> Result<u8, scr_runtime::RuntimeError> {
    runtime.execute_source(source, out)
}
#[test]
fn echo_writes_directly_and_flush_is_an_explicit_host_operation() {
    let mut r = runtime(RuntimeOptions::default());
    let mut out = Writer::default();
    run(&mut r, r#"echo "é";flush();echo "finished";"#, &mut out).unwrap();
    assert_eq!(out.flushes, vec!["é".as_bytes().to_vec()]);
    assert_eq!(out.bytes, "éfinished".as_bytes());
    assert!(run(&mut r, "flush(1);", &mut out).is_err());
}
#[test]
fn returned_content_does_not_emit_and_functions_and_modules_use_the_host_writer() {
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("part.scripta");
    std::fs::write(&module, r#"echo "module";"#).unwrap();
    let path = serde_json::to_string(module.to_str().unwrap()).unwrap();
    let mut r = runtime(RuntimeOptions::default());
    let mut out = Writer::default();
    run(&mut r,&format!(r#"function content() {{return "returned";}} function emit() {{echo "function";}} $content=content();emit();require {path};"#),&mut out).unwrap();
    assert_eq!(out.bytes, b"functionmodule");
    assert_eq!(
        r.get_global("content"),
        Some(&Value::String("returned".into()))
    );
    assert!(out.flushes.is_empty());
}
#[test]
fn output_limit_is_exact_and_caught_native_overflow_remains_fatal() {
    let mut r = runtime(RuntimeOptions {
        max_output_bytes: Some(4),
        ..Default::default()
    });
    r.register(
        "Emit::byte",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Null),
            |context, _| {
                context.output.write_all(b"x")?;
                Ok(Value::Null)
            },
        ),
    )
    .unwrap();
    let mut out = Writer::default();
    run(&mut r, r#"echo "1234";flush();"#, &mut out).unwrap();
    assert_eq!(out.bytes, b"1234");
    let mut exceeded = Writer::default();
    assert!(
        run(
            &mut r,
            r#"echo "1234";flush();try {Emit::byte();} catch ($e) {}"#,
            &mut exceeded
        )
        .unwrap_err()
        .message
        .contains("output")
    );
    assert_eq!(exceeded.bytes, b"1234");
    assert_eq!(exceeded.flushes, vec![b"1234".to_vec()]);
    // The budget resets for each execution, never for a flush within an execution.
    run(&mut r, r#"echo "1234";"#, &mut out).unwrap();
    assert_eq!(out.bytes, b"12341234");
}
#[test]
fn partial_writes_are_completed_and_caught_writer_and_flush_failures_remain_fatal() {
    let mut r = runtime(RuntimeOptions::default());
    r.register(
        "Emit::bytes",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Null),
            |context, _| {
                context.output.write_all(b"\0\xffabc")?;
                Ok(Value::Null)
            },
        ),
    )
    .unwrap();
    let mut out = Writer::default();
    run(&mut r, "Emit::bytes();", &mut out).unwrap();
    assert_eq!(out.bytes, b"\0\xffabc");
    out.fail = true;
    assert!(run(&mut r, "try {Emit::bytes();} catch ($e) {}", &mut out).is_err());
    assert_eq!(out.bytes, b"\0\xffabc");
    out.fail = false;
    out.fail_flush = true;
    assert!(run(&mut r, "try {flush();} catch ($e) {}", &mut out).is_err());
    assert!(out.flushes.is_empty());
}
#[test]
fn failures_do_not_retract_bytes_already_written_to_the_host() {
    let mut r = runtime(RuntimeOptions {
        instruction_limit: Some(30),
        ..Default::default()
    });
    let mut out = Writer::default();
    assert!(
        run(&mut r, r#"echo "pending";while (true) {}"#, &mut out)
            .unwrap_err()
            .message
            .contains("instruction")
    );
    assert_eq!(out.bytes, b"pending");
    let mut r = runtime(RuntimeOptions::default());
    let mut out = Writer::default();
    assert!(run(&mut r, r#"echo "visible";throw "failed";"#, &mut out).is_err());
    assert_eq!(out.bytes, b"visible");
}
#[test]
fn removed_buffer_functions_are_undefined_and_failed_assignments_preserve_destinations() {
    let mut r = runtime(RuntimeOptions::default());
    for name in [
        "ob_start",
        "ob_flush",
        "ob_end_flush",
        "ob_clean",
        "ob_end_clean",
        "ob_get_contents",
        "ob_get_clean",
        "ob_get_flush",
        "ob_get_length",
        "ob_get_level",
    ] {
        let failure = run(
            &mut r,
            &format!(r#"$saved="keep";$saved={name}();"#),
            &mut vec![],
        )
        .unwrap_err();
        assert!(
            failure
                .message
                .contains(&format!("undefined function {name}")),
            "{failure}"
        );
        assert_eq!(r.get_global("saved"), Some(&Value::String("keep".into())));
    }
}
