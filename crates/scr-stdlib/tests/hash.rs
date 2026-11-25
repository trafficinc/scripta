use scr_runtime::{NativeObject, Runtime, RuntimeError, RuntimeOptions, Value};
use std::rc::Rc;

fn runtime() -> Runtime {
    let mut runtime = Runtime::new();
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), RuntimeError> {
    runtime.execute_source(source, &mut Vec::new()).map(|_| ())
}
fn bytes(data: &[u8]) -> Value {
    Value::Bytes(data.into())
}
fn assert_result(runtime: &Runtime, name: &str, expected: &str) {
    assert_eq!(
        runtime.get_global(name),
        Some(&Value::String(expected.into())),
        "{name}"
    );
}

#[test]
fn sha256_published_vectors_and_empty_payload() {
    // SHA-256 byte-aligned vectors from RFC 6234 section 8.5.
    let mut runtime = runtime();
    for (data, expected) in [
        (
            Vec::new(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc".to_vec(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".to_vec(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            vec![b'a'; 1_000_000],
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
        ),
    ] {
        runtime.set_global("data", bytes(&data)).unwrap();
        run(&mut runtime, "$raw=Hash::sha256($data); $hex=Hex::encode($raw); $kind=type($raw); $length=len($raw);").unwrap();
        assert_result(&runtime, "hex", expected);
        assert_result(&runtime, "kind", "bytes");
        assert_eq!(runtime.get_global("length"), Some(&Value::Int(32)));
    }
}

#[test]
fn rfc4231_hmac_vectors_and_empty_keys_and_messages() {
    // https://www.rfc-editor.org/rfc/rfc4231.html sections 4.2–4.5, 4.7–4.8.
    let mut runtime = runtime();
    for (key, data, expected) in [
        (vec![0x0b; 20], b"Hi There".to_vec(), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"),
        (b"Jefe".to_vec(), b"what do ya want for nothing?".to_vec(), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"),
        (vec![0xaa; 20], vec![0xdd; 50], "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"),
        ((1..=25).collect(), vec![0xcd; 50], "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b"),
        (vec![0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"),
        (vec![0xaa; 131], b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.".to_vec(), "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2"),
        (Vec::new(), Vec::new(), "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad"),
    ] {
        runtime.set_global("key", bytes(&key)).unwrap();
        runtime.set_global("data", bytes(&data)).unwrap();
        run(&mut runtime, "$hex=Hex::encode(Hash::hmacSha256($key,$data)); $h=Hash::hmacSha256Init($key); Hash::update($h,$data); $incremental=Hex::encode(Hash::finalize($h));").unwrap();
        assert_result(&runtime, "hex", expected);
        assert_result(&runtime, "incremental", expected);
    }
}

#[test]
fn binary_chunks_block_boundaries_and_long_keys_match_one_shot() {
    let mut runtime = runtime();
    let input: Vec<u8> = (0..50000).map(|i| (i % 256) as u8).collect();
    for key_len in [0, 1, 63, 64, 65, 131, 16385] {
        let key = vec![0xff; key_len];
        runtime.set_global("key", bytes(&key)).unwrap();
        runtime.set_global("data", bytes(&input)).unwrap();
        run(&mut runtime, "$sha=Hash::sha256Init(); $hmac=Hash::hmacSha256Init($key); $expectedSha=Hash::sha256($data); $expectedHmac=Hash::hmacSha256($key,$data);").unwrap();
        let mut at = 0;
        for length in [0, 1, 54, 1, 7, 1, 1, 16318, 16384, 16385, 848] {
            let end = (at + length).min(input.len());
            runtime.set_global("chunk", bytes(&input[at..end])).unwrap();
            run(
                &mut runtime,
                "Hash::update($sha,$chunk); Hash::update($hmac,$chunk);",
            )
            .unwrap();
            at = end;
        }
        assert_eq!(at, input.len());
        run(
            &mut runtime,
            "$actualSha=Hash::finalize($sha); $actualHmac=Hash::finalize($hmac);",
        )
        .unwrap();
        assert_eq!(
            runtime.get_global("actualSha"),
            runtime.get_global("expectedSha")
        );
        assert_eq!(
            runtime.get_global("actualHmac"),
            runtime.get_global("expectedHmac")
        );
    }
}

#[test]
fn aliases_share_finalization_and_close_is_idempotent() {
    let mut runtime = runtime();
    for init in [
        "Hash::sha256Init()",
        "Hash::hmacSha256Init(Bytes::encodeUtf8('key'))",
    ] {
        run(&mut runtime, &format!("$h={init}; $alias=$h; $open=$h->closed; $algorithm=$h->algorithm; $result=Hash::finalize($h); $closed=$alias->closed; Hash::close($alias); Hash::close($h);")).unwrap();
        assert_eq!(runtime.get_global("open"), Some(&Value::Bool(false)));
        assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
        let saved = runtime.get_global("result").unwrap().clone();
        for call in [
            "Hash::update($alias,Bytes::encodeUtf8('x'))",
            "Hash::finalize($h)",
        ] {
            let error = run(&mut runtime, &format!("$result={call};")).unwrap_err();
            assert!(error.message.starts_with("HashError:"), "{error}");
            assert_eq!(runtime.get_global("result"), Some(&saved));
        }
        run(&mut runtime, &format!("$h={init}; Hash::close($h);")).unwrap();
        assert!(
            run(&mut runtime, "Hash::finalize($h);")
                .unwrap_err()
                .message
                .starts_with("HashError:")
        );
    }
}

#[derive(Debug)]
struct Forged;
impl NativeObject for Forged {
    fn type_name(&self) -> &str {
        "HashContext"
    }
    fn get_property(&self, _: &str) -> Result<Value, String> {
        Ok(Value::Null)
    }
}

#[test]
fn handles_reject_foreign_and_forged_values_and_close_on_runtime_drop() {
    let mut first = runtime();
    let mut other = runtime();
    run(&mut first, "$h=Hash::hmacSha256Init(Bytes::encodeUtf8('private key')); Hash::update($h,Bytes::encodeUtf8('data'));").unwrap();
    let handle = first.get_global("h").unwrap().clone();
    other.set_global("h", handle.clone()).unwrap();
    run(&mut other, "$result='unchanged';").unwrap();
    for call in [
        "Hash::update($h,Bytes::encodeUtf8('x'))",
        "Hash::finalize($h)",
        "Hash::close($h)",
    ] {
        let error = run(&mut other, &format!("$result={call};")).unwrap_err();
        assert!(
            error.message.starts_with("TypeError:")
                && error.message.contains("another registration")
        );
        assert_result(&other, "result", "unchanged");
    }
    let Value::NativeObject(object) = &handle else {
        panic!()
    };
    assert_eq!(object.get_property("closed").unwrap(), Value::Bool(false));
    assert!(!format!("{object:?}").contains("private key"));
    drop(first);
    assert_eq!(object.get_property("closed").unwrap(), Value::Bool(true));
    other
        .set_global("h", Value::NativeObject(Rc::new(Forged)))
        .unwrap();
    for call in [
        "Hash::close($h)",
        "Hash::finalize($h)",
        "Hash::update($h,Bytes::encodeUtf8(''))",
    ] {
        assert!(
            run(&mut other, &format!("{call};"))
                .unwrap_err()
                .message
                .starts_with("TypeError:")
        );
    }
}

#[test]
fn strict_signatures_and_handle_ceiling_preserve_destinations() {
    let mut runtime = runtime();
    run(&mut runtime, "$h=Hash::sha256Init(); $result='unchanged';").unwrap();
    for call in [
        "Hash::sha256('abc')",
        "Hash::hmacSha256('key',Bytes::encodeUtf8('data'))",
        "Hash::hmacSha256(Bytes::encodeUtf8('key'),'data')",
        "Hash::sha256Init(1)",
        "Hash::hmacSha256Init('key')",
        "Hash::update($h,'data')",
        "Hash::finalize(null)",
        "Hash::close(1)",
        "Hash::sha256()",
    ] {
        assert!(
            run(&mut runtime, &format!("$result={call};"))
                .unwrap_err()
                .message
                .starts_with("TypeError:")
        );
        assert_result(&runtime, "result", "unchanged");
    }
    run(&mut runtime, "$empty=Hex::encode(Hash::finalize($h));").unwrap();
    assert_result(
        &runtime,
        "empty",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
    for index in 0..64 {
        run(&mut runtime, &format!("$h{index}=Hash::sha256Init();")).unwrap();
    }
    let error = run(
        &mut runtime,
        "$result=Hash::hmacSha256Init(Bytes::encodeUtf8('key'));",
    )
    .unwrap_err();
    assert!(error.message.starts_with("HashLimitError:"));
    assert_result(&runtime, "result", "unchanged");
    // One-shot calls do not consume a handle slot. Finalization and close both
    // release a slot immediately, even if aliases still hold the closed object.
    run(&mut runtime, "Hash::sha256(Bytes::encodeUtf8('x')); Hash::finalize($h0); $new=Hash::sha256Init(); Hash::close($new); $new=Hash::hmacSha256Init(Bytes::encodeUtf8('key'));").unwrap();
    run(&mut runtime, "$h1=null; $next=Hash::sha256Init();").unwrap();
}

#[test]
fn digest_budget_is_exact_and_failure_preserves_assignment() {
    for allowance in [31, 32] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(std::mem::size_of::<Value>() + allowance),
            ..Default::default()
        });
        scr_stdlib::register(&mut runtime).unwrap();
        run(
            &mut runtime,
            "$data=Bytes::encodeUtf8(''); $result='unchanged';",
        )
        .unwrap();
        for call in ["Hash::sha256($data)", "Hash::hmacSha256($data,$data)"] {
            let result = run(&mut runtime, &format!("$result={call};"));
            if allowance == 32 {
                result.unwrap();
                let Value::Bytes(result) = runtime.get_global("result").unwrap() else {
                    panic!()
                };
                assert_eq!(result.len(), 32);
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .message
                        .starts_with("ResourceLimitError:")
                );
                assert_result(&runtime, "result", "unchanged");
            }
        }
    }
}

#[test]
fn cancellation_preserves_destination_and_prior_open_state() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    run(
        &mut runtime,
        "$h=Hash::sha256Init(); Hash::update($h,Bytes::encodeUtf8('abc')); $result='unchanged';",
    )
    .unwrap();
    for call in [
        "Hash::sha256Init()",
        "Hash::hmacSha256Init(Bytes::encodeUtf8('key'))",
        "Hash::sha256(Bytes::encodeUtf8('abc'))",
        "Hash::hmacSha256(Bytes::encodeUtf8('key'),Bytes::encodeUtf8('abc'))",
        "Hash::update($h,Bytes::encodeUtf8('x'))",
        "Hash::finalize($h)",
        "Hash::close($h)",
    ] {
        flag.store(true, Ordering::Relaxed);
        let error = run(&mut runtime, &format!("$result={call};")).unwrap_err();
        assert_eq!(error.message, "execution interrupted");
        assert_result(&runtime, "result", "unchanged");
        flag.store(false, Ordering::Relaxed);
    }
    run(&mut runtime, "$digest=Hex::encode(Hash::finalize($h));").unwrap();
    assert_result(
        &runtime,
        "digest",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
}

#[test]
fn closed_state_errors_keep_native_spans_and_script_stack_frames() {
    let mut runtime = runtime();
    let source = "function finishAgain($h) { return Hash::finalize($h); }\n$h=Hash::sha256Init(); Hash::close($h);\n$result='unchanged'; $result=finishAgain($h);";
    let error = run(&mut runtime, source).unwrap_err();
    assert!(error.message.starts_with("HashError:"));
    assert_eq!(error.span.start, source.find("Hash::finalize").unwrap());
    assert!(
        error
            .trace
            .iter()
            .any(|frame| frame.function == "finishAgain")
    );
    assert!(
        error
            .trace
            .iter()
            .any(|frame| frame.function == "Hash::finalize")
    );
    assert_result(&runtime, "result", "unchanged");
}
