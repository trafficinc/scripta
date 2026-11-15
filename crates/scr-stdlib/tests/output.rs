use scr_runtime::{Runtime, RuntimeOptions, Value};
use std::io::{self, Write};
#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    flushes: Vec<Vec<u8>>,
    fail: bool,
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
    s: &str,
    out: &mut impl Write,
) -> Result<u8, scr_runtime::RuntimeError> {
    runtime.execute_source(&format!("<?scripta {s}"), out)
}
#[test]
fn nested_buffers_are_binary_and_transport_flush_does_not_drain_them() {
    let mut runtime = runtime(RuntimeOptions::default());
    let mut output = Writer::default();
    run(
        &mut runtime,
        r#"echo "a";ob_start();echo "b";ob_start();Webless::noop();"#,
        &mut output,
    )
    .unwrap_err();
    assert_eq!(output.bytes, b"a");
    run(&mut runtime,r#"$initial=ob_get_level();ob_start();echo "é";ob_start();echo "c";$length=ob_get_length();$copy=ob_get_flush();$level=ob_get_level();$data=ob_get_contents();flush();ob_flush();flush();echo "d";"#,&mut output).unwrap();
    assert_eq!(runtime.get_global("initial"), Some(&Value::Int(0)));
    assert_eq!(runtime.get_global("length"), Some(&Value::Int(1)));
    assert_eq!(runtime.get_global("level"), Some(&Value::Int(1)));
    assert_eq!(
        runtime.get_global("copy"),
        Some(&Value::Bytes(b"c".to_vec().into()))
    );
    assert_eq!(
        runtime.get_global("data"),
        Some(&Value::Bytes("éc".as_bytes().to_vec().into()))
    );
    assert_eq!(
        output.flushes,
        vec![b"a".to_vec(), "aéc".as_bytes().to_vec()]
    );
    assert_eq!(output.bytes, "aécd".as_bytes());
}
#[test]
fn cleanup_retrieval_empty_states_and_signatures_preserve_buffers() {
    let mut runtime = runtime(RuntimeOptions::default());
    let mut out = vec![];
    run(&mut runtime,r#"$none=ob_get_contents();$length=ob_get_length();ob_start();echo "discard";ob_clean();echo "kept";$bytes=ob_get_clean();ob_start();echo "gone";ob_end_clean();echo "ok";"#,&mut out).unwrap();
    assert_eq!(out, b"ok");
    assert_eq!(runtime.get_global("none"), Some(&Value::Null));
    assert_eq!(runtime.get_global("length"), Some(&Value::Null));
    assert_eq!(
        runtime.get_global("bytes"),
        Some(&Value::Bytes(b"kept".to_vec().into()))
    );
    for name in [
        "ob_flush",
        "ob_end_flush",
        "ob_clean",
        "ob_end_clean",
        "ob_get_clean",
        "ob_get_flush",
    ] {
        assert!(
            run(&mut runtime, &format!("{name}();"), &mut vec![])
                .unwrap_err()
                .message
                .contains("no active output buffer")
        );
    }
    assert!(run(&mut runtime, "ob_start(1);", &mut vec![]).is_err());
    assert!(run(&mut runtime, "flush(1);", &mut vec![]).is_err());
}
#[test]
fn cumulative_output_counts_discarded_bytes_and_failures_are_sticky() {
    let mut runtime = runtime(RuntimeOptions {
        max_output_bytes: Some(4),
        max_output_buffer_bytes: 4,
        ..Default::default()
    });
    let mut out = vec![];
    run(
        &mut runtime,
        "ob_start();echo \"1234\";ob_end_clean();",
        &mut out,
    )
    .unwrap();
    assert!(out.is_empty());
    let failure = run(
        &mut runtime,
        r#"ob_start();echo "1234";ob_clean();try {echo "x";} catch ($e) {}"#,
        &mut out,
    )
    .unwrap_err();
    assert!(failure.message.contains("output"));
    assert!(out.is_empty());
    let mut runtime = super_runtime_with_small_buffer();
    assert!(run(&mut runtime, "ob_start();echo \"12345\";", &mut vec![]).is_err());
}
fn super_runtime_with_small_buffer() -> Runtime {
    runtime(RuntimeOptions {
        max_output_buffer_bytes: 4,
        ..Default::default()
    })
}
#[test]
fn partial_writes_are_completed_and_writer_errors_do_not_publish_buffered_output() {
    let mut runtime = runtime(RuntimeOptions::default());
    let mut output = Writer::default();
    run(
        &mut runtime,
        "ob_start();echo \"abcdef\";ob_end_flush();",
        &mut output,
    )
    .unwrap();
    assert_eq!(output.bytes, b"abcdef");
    output.fail = true;
    assert!(run(&mut runtime, "ob_start();echo \"failed\";", &mut output).is_err());
    assert_eq!(output.bytes, b"abcdef");
}
#[test]
fn buffer_results_and_nesting_use_exact_limits_without_consuming_on_failure() {
    let slot = size_of::<Value>();
    for (limit, expected) in [(slot + 4, true), (slot + 3, false)] {
        let mut runtime = runtime(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        let result = run(
            &mut runtime,
            r#"ob_start();echo "aa";echo "bb";$saved=null;$saved=ob_get_clean();"#,
            &mut vec![],
        );
        assert_eq!(result.is_ok(), expected);
        assert_eq!(
            runtime.get_global("saved"),
            Some(&if expected {
                Value::Bytes(b"aabb".to_vec().into())
            } else {
                Value::Null
            })
        );
    }
    let mut runtime = runtime(RuntimeOptions {
        max_output_buffers: 1,
        max_output_buffer_bytes: 4,
        ..Default::default()
    });
    let mut out = vec![];
    run(
        &mut runtime,
        r#"ob_start();echo "1234";try {ob_start();} catch ($e) {} $level=ob_get_level();"#,
        &mut out,
    )
    .unwrap();
    assert_eq!(out, b"1234");
    assert_eq!(runtime.get_global("level"), Some(&Value::Int(1)));
}
#[test]
fn arbitrary_binary_native_output_is_captured_and_caught_writer_failures_remain_fatal() {
    use scr_runtime::{NativeFunction, NativeSignature, NativeType};
    let mut runtime = runtime(RuntimeOptions::default());
    runtime
        .register(
            "Emit::bytes",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Null),
                |context, _| {
                    context.output.write_all(b"\0\xff")?;
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
    let mut output = Writer::default();
    run(
        &mut runtime,
        "ob_start();Emit::bytes();$captured=ob_get_clean();",
        &mut output,
    )
    .unwrap();
    assert!(output.bytes.is_empty());
    assert_eq!(
        runtime.get_global("captured"),
        Some(&Value::Bytes(b"\0\xff".to_vec().into()))
    );
    output.fail = true;
    assert!(
        run(
            &mut runtime,
            "try {Emit::bytes();} catch ($e) {}",
            &mut output
        )
        .is_err()
    );
    assert!(output.bytes.is_empty());
}
#[test]
fn functions_and_required_modules_share_buffers_and_fatal_limits_discard_them() {
    let directory = tempfile::tempdir().unwrap();
    let module = directory.path().join("part.scripta");
    std::fs::write(&module, "<?scripta echo \"module\";").unwrap();
    let path = serde_json::to_string(module.to_str().unwrap()).unwrap();
    let mut runtime = runtime(RuntimeOptions::default());
    let mut out = vec![];
    run(&mut runtime,&format!("function part() {{echo \"function\";}} ob_start();part();require {path};$bytes=ob_get_clean();"),&mut out).unwrap();
    assert!(out.is_empty());
    assert_eq!(
        runtime.get_global("bytes"),
        Some(&Value::Bytes(b"functionmodule".to_vec().into()))
    );
    let mut runtime = super_runtime_with_small_buffer();
    // Use an independently budgeted execution to exercise fatal cleanup rather
    // than depending on a writer failure to discard the buffer.
    let options = RuntimeOptions {
        instruction_limit: Some(30),
        ..Default::default()
    };
    let mut limited = Runtime::with_options(options);
    scr_stdlib::register(&mut limited).unwrap();
    assert!(
        run(
            &mut limited,
            "ob_start();echo \"pending\";while (true) {}",
            &mut out
        )
        .unwrap_err()
        .message
        .contains("instruction")
    );
    assert!(out.is_empty());
    assert!(run(&mut runtime, "ob_start();echo \"12345\";", &mut out).is_err());
}
