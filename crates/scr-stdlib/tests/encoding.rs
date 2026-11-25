use scr_runtime::{Runtime, RuntimeError, RuntimeOptions, Value};

fn runtime(limit: Option<usize>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}

fn run(runtime: &mut Runtime, source: &str) -> Result<(), RuntimeError> {
    runtime.execute_source(source, &mut Vec::new()).map(|_| ())
}

#[test]
fn standard_vectors_include_empty_inputs_and_explicit_text_conversion() {
    let mut runtime = runtime(None);
    for (raw, base64, hex) in [
        ("", "", ""),
        ("f", "Zg==", "66"),
        ("fo", "Zm8=", "666f"),
        ("foo", "Zm9v", "666f6f"),
        ("foob", "Zm9vYg==", "666f6f62"),
        ("fooba", "Zm9vYmE=", "666f6f6261"),
        ("foobar", "Zm9vYmFy", "666f6f626172"),
        ("é", "w6k=", "c3a9"),
    ] {
        runtime
            .set_global("raw", Value::String(raw.into()))
            .unwrap();
        for (encoder, decoder, encoded) in [
            ("Base64::encode", "Base64::decode", base64),
            (
                "Base64::encodeUrl",
                "Base64::decodeUrl",
                base64.trim_end_matches('='),
            ),
            ("Hex::encode", "Hex::decode", hex),
        ] {
            runtime
                .set_global("encoded", Value::String(encoded.into()))
                .unwrap();
            run(
                &mut runtime,
                &format!(
                    "$bytes = Bytes::encodeUtf8($raw); $encodedResult = {encoder}($bytes); \
                 $decoded = {decoder}($encoded); $text = Bytes::decodeUtf8($decoded);"
                ),
            )
            .unwrap();
            assert_eq!(
                runtime.get_global("encodedResult"),
                Some(&Value::String(encoded.into()))
            );
            assert_eq!(
                runtime.get_global("decoded"),
                Some(&Value::Bytes(raw.as_bytes().into()))
            );
            assert_eq!(runtime.get_global("text"), Some(&Value::String(raw.into())));
        }
    }
    run(&mut runtime, "$result = Hex::decode('aAbBcCdDeEfF');").unwrap();
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::Bytes(
            vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff].into()
        ))
    );
}

#[test]
fn binary_data_and_chunk_boundaries_roundtrip() {
    let mut runtime = runtime(None);
    runtime
        .set_global("binary", Value::Bytes(vec![0xfb, 0xff, 0x00, 0xff].into()))
        .unwrap();
    run(&mut runtime, "$standard = Base64::encode($binary); $url = Base64::encodeUrl($binary); $hex = Hex::encode($binary);").unwrap();
    assert_eq!(
        runtime.get_global("standard"),
        Some(&Value::String("+/8A/w==".into()))
    );
    assert_eq!(
        runtime.get_global("url"),
        Some(&Value::String("-_8A_w".into()))
    );
    assert_eq!(
        runtime.get_global("hex"),
        Some(&Value::String("fbff00ff".into()))
    );
    for length in [
        256, 2047, 2048, 2049, 3071, 3072, 3073, 3074, 6144, 6145, 16385,
    ] {
        let data: Vec<u8> = (0..length).map(|i| (i % 256) as u8).collect();
        let expected = Value::Bytes(data.into());
        runtime.set_global("binary", expected.clone()).unwrap();
        for (encoder, decoder) in [
            ("Base64::encode", "Base64::decode"),
            ("Base64::encodeUrl", "Base64::decodeUrl"),
            ("Hex::encode", "Hex::decode"),
        ] {
            run(
                &mut runtime,
                &format!("$result = {decoder}({encoder}($binary));"),
            )
            .unwrap();
            assert_eq!(
                runtime.get_global("result"),
                Some(&expected),
                "{encoder}: {length}"
            );
        }
    }
}

#[test]
fn strict_decoders_reject_noncanonical_input_and_preserve_destinations() {
    let mut runtime = runtime(None);
    run(&mut runtime, "$result = 'unchanged';").unwrap();
    for (decoder, malformed) in [
        (
            "Base64::decode",
            vec![
                "Zg", "Zg=", "Zg===", "=Zg=", "Z=g=", "Zg==AA==", "Zh==", "Zm9=", "-_8=", "Z g==",
                "Zg==\n", "Zg==\r\n", "Zg==\t", "Zg==\0", "éAAA",
            ],
        ),
        (
            "Base64::decodeUrl",
            vec![
                "Z", "Zg=", "Zg==", "Zh", "Zm9", "+/8", "Z g", "Zg\n", "Zg\r\n", "Zg\0", "éAA",
            ],
        ),
        (
            "Hex::decode",
            vec![
                "0", "abc", "0x00", "gg", "00 0", "00\n0", "00\r\n", "00\t0", "00\0a", "é",
            ],
        ),
    ] {
        for input in malformed {
            runtime
                .set_global("input", Value::String(input.into()))
                .unwrap();
            let error = run(&mut runtime, &format!("$result = {decoder}($input);")).unwrap_err();
            assert!(
                error.message.starts_with("EncodingError:"),
                "{decoder}: {input:?}: {error}"
            );
            assert_eq!(
                runtime.get_global("result"),
                Some(&Value::String("unchanged".into()))
            );
        }
    }
    // A padded quartet at a backend chunk boundary must not terminate an
    // otherwise longer input, even though each chunk would decode separately.
    for input in [
        format!("{}Zg==AAAA", "AAAA".repeat(1023)),
        format!("{}!AAA", "AAAA".repeat(1024)),
    ] {
        runtime.set_global("input", Value::String(input)).unwrap();
        assert!(
            run(&mut runtime, "$result = Base64::decode($input);")
                .unwrap_err()
                .message
                .starts_with("EncodingError:")
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
}

#[test]
fn signatures_are_strict_and_failed_growth_preserves_assignment() {
    let mut runtime = runtime(Some(std::mem::size_of::<Value>() + 7));
    run(
        &mut runtime,
        "$result = 'old'; $data = Bytes::encodeUtf8('abcdef');",
    )
    .unwrap();
    for name in ["Base64::encode", "Base64::encodeUrl", "Hex::encode"] {
        let error = run(&mut runtime, &format!("$result = {name}($data);")).unwrap_err();
        assert!(
            error.message.starts_with("ResourceLimitError:"),
            "{name}: {error}"
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("old".into()))
        );
    }
    for call in [
        "Base64::encode('a')",
        "Base64::encodeUrl(1)",
        "Hex::encode('ab')",
        "Base64::decode($data)",
        "Base64::decodeUrl(null)",
        "Hex::decode($data)",
        "Hex::encode()",
        "Base64::decode('','')",
    ] {
        assert!(
            run(&mut runtime, &format!("$result = {call};"))
                .unwrap_err()
                .message
                .starts_with("TypeError:")
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("old".into()))
        );
    }
}

#[test]
fn cancellation_preserves_assignment_destinations() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(Arc::clone(&flag)),
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    run(
        &mut runtime,
        "$result = 'unchanged'; $data = Bytes::encodeUtf8('foo');",
    )
    .unwrap();
    for call in [
        "Base64::encode($data)",
        "Base64::decode('Zm9v')",
        "Base64::encodeUrl($data)",
        "Base64::decodeUrl('Zm9v')",
        "Hex::encode($data)",
        "Hex::decode('666f6f')",
    ] {
        flag.store(true, Ordering::Relaxed);
        assert_eq!(
            run(&mut runtime, &format!("$result = {call};"))
                .unwrap_err()
                .message,
            "execution interrupted"
        );
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
        flag.store(false, Ordering::Relaxed);
        run(&mut runtime, &format!("{call};")).unwrap();
    }
}
