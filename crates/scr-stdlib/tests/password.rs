use scr_runtime::{Runtime, RuntimeOptions, Value};
use scr_stdlib::{StdlibOptions, password::PasswordConfig};
use std::{
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
};

fn low_cost() -> PasswordConfig {
    PasswordConfig {
        memory_kib: 32,
        iterations: 2,
        parallelism: 1,
        ..Default::default()
    }
}
fn runtime(config: PasswordConfig, options: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register_with_options(
        &mut runtime,
        StdlibOptions {
            password: config,
            ..Default::default()
        },
    )
    .unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime
        .execute_source(&format!("<?scripta {source}"), &mut Vec::new())
        .unwrap();
}
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/passwords/php-argon2id.json"
    ))
    .unwrap()
}
fn fixtures(fixture: &serde_json::Value) -> &[serde_json::Value] {
    fixture["fixtures"].as_array().unwrap()
}
fn boolean(runtime: &mut Runtime, expression: &str) -> bool {
    run(runtime, &format!("$result={expression};"));
    let Some(Value::Bool(result)) = runtime.get_global("result") else {
        panic!("expected bool")
    };
    *result
}
#[test]
fn default_policy_uses_random_salts_and_roundtrips_without_automation_permissions() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut runtime = runtime(PasswordConfig::default(), options);
    run(
        &mut runtime,
        r#"$first=Password::hash("public example");$second=Password::hash("public example");$valid=Password::verify("public example",$first);$invalid=Password::verify("different",$first);$rehash=Password::needsRehash($first);"#,
    );
    let Some(Value::String(first)) = runtime.get_global("first") else {
        panic!()
    };
    let Some(Value::String(second)) = runtime.get_global("second") else {
        panic!()
    };
    assert_ne!(first, second);
    assert!(first.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    let first = argon2::PasswordHash::new(first).unwrap();
    let second = argon2::PasswordHash::new(second).unwrap();
    assert_ne!(first.salt, second.salt);
    assert_eq!(first.hash.unwrap().len(), 32);
    assert_eq!(runtime.get_global("valid"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("invalid"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("rehash"), Some(&Value::Bool(false)));
}
#[test]
fn php_fixtures_verify_including_unicode_nul_empty_and_default_costs() {
    let fixture = fixture();
    let mut runtime = runtime(PasswordConfig::default(), RuntimeOptions::default());
    for fixture in fixtures(&fixture) {
        let password = fixture["password"].as_str().unwrap();
        let hash = fixture["hash"].as_str().unwrap();
        runtime
            .set_global("password", Value::String(password.into()))
            .unwrap();
        runtime
            .set_global("hash", Value::String(hash.into()))
            .unwrap();
        assert!(
            boolean(&mut runtime, "Password::verify($password,$hash)"),
            "{}",
            fixture["name"]
        );
        if fixture["name"] != "php-default" {
            runtime
                .set_global("wrong", Value::String(format!("{password}x")))
                .unwrap();
            assert!(!boolean(&mut runtime, "Password::verify($wrong,$hash)"));
        }
        assert!(boolean(&mut runtime, "Password::needsRehash($hash)"));
    }
}
#[test]
fn reference_argon2id_known_answer_and_rehash_policy_are_respected() {
    // Argon2 reference implementation vector, via RustCrypto argon2 0.5.3:
    // tests/phc_strings.rs, adapted from P-H-C/phc-winner-argon2/src/test.c.
    let known =
        "$argon2id$v=19$m=65536,t=2,p=1$c29tZXNhbHQ$CTFhFdXPJO1aFaMaO6Mm5c8y7cJHAph8ArZWb2GRPPc";
    let mut runtime = runtime(PasswordConfig::default(), RuntimeOptions::default());
    runtime
        .set_global("known", Value::String(known.into()))
        .unwrap();
    assert!(boolean(&mut runtime, "Password::verify('password',$known)"));
    let mut custom = runtime_with_costs(32, 3, 2);
    run(&mut custom, "$hash=Password::hash('public');");
    let hash = custom.get_global("hash").unwrap().clone();
    assert!(!boolean(&mut custom, "Password::needsRehash($hash)"));
    let mut older = runtime_with_costs(32, 2, 1);
    older.set_global("hash", hash).unwrap();
    assert!(boolean(&mut older, "Password::needsRehash($hash)"));
    assert!(boolean(&mut older, "Password::verify('public',$hash)"));
}
fn runtime_with_costs(memory_kib: u32, iterations: u32, parallelism: u32) -> Runtime {
    runtime(
        PasswordConfig {
            memory_kib,
            iterations,
            parallelism,
            ..low_cost()
        },
        RuntimeOptions::default(),
    )
}
#[test]
fn malformed_unsupported_and_incomplete_hashes_have_documented_results() {
    let fixture = fixture();
    let valid = fixtures(&fixture)[0]["hash"].as_str().unwrap();
    let mut runtime = runtime(low_cost(), RuntimeOptions::default());
    for hash in [
        "".into(),
        "not a hash".into(),
        "$2y$12$invalid".into(),
        valid.replace("argon2id", "argon2i"),
        valid.replace("v=19", "v=16"),
        valid.replace("v=19", "v=42"),
        valid.replace("$v=19", ""),
        "🔑".into(),
        format!("{}$!!!!", valid.rsplit_once('$').unwrap().0),
        format!("{}$YWJjZGVmZ2hpamts", valid.rsplit_once('$').unwrap().0),
        valid.replace("m=32,t=2,p=1", "m=0,t=2,p=1"),
        valid.replace("m=32,t=2,p=1", "m=32,t=0,p=1"),
        valid.replace("m=32,t=2,p=1", "m=32,t=2,p=0"),
        valid.replace("m=32,t=2,p=1", "m=32,t=2"),
        valid.replace("m=32,t=2,p=1", "m=32,t=2,p=1,keyid=YWJj"),
        valid.replace("m=32,t=2,p=1", "m=32,t=2,p=1,m=32"),
        valid.rsplit_once('$').unwrap().0.into(),
        valid.replace("m=32,t=2,p=1", "m=4294967296,t=2,p=1"),
    ] {
        runtime.set_global("hash", Value::String(hash)).unwrap();
        assert!(!boolean(
            &mut runtime,
            "Password::verify('public-fixture-password',$hash)"
        ));
        assert!(boolean(&mut runtime, "Password::needsRehash($hash)"));
    }
}
#[test]
fn untrusted_costs_are_rejected_before_work_and_failures_keep_the_destination() {
    let fixture = fixture();
    let valid = fixtures(&fixture)[0]["hash"].as_str().unwrap();
    let mut runtime = runtime(low_cost(), RuntimeOptions::default());
    for costs in [
        "m=65537,t=2,p=1",
        "m=32,t=11,p=1",
        "m=64,t=2,p=5",
        "m=65536,t=5,p=1",
        "m=4294967295,t=2,p=1",
    ] {
        let hash = valid.replace("m=32,t=2,p=1", costs);
        runtime.set_global("hash", Value::String(hash)).unwrap();
        runtime.set_global("saved", Value::Int(42)).unwrap();
        for source in [
            "$saved=Password::verify('public-fixture-password',$hash);",
            "$saved=Password::needsRehash($hash);",
        ] {
            let error = runtime
                .execute_source(&format!("<?scripta {source}"), &mut Vec::new())
                .unwrap_err();
            assert!(
                error.message.starts_with("PasswordError:"),
                "{}",
                error.message
            );
            assert!(error.message.contains("costs exceed"));
            assert_eq!(runtime.get_global("saved"), Some(&Value::Int(42)));
        }
    }
    runtime
        .set_global("hash", Value::String("x".repeat(513)))
        .unwrap();
    assert!(
        runtime
            .execute_source("<?scripta Password::needsRehash($hash);", &mut Vec::new())
            .unwrap_err()
            .message
            .contains("hash exceeds host byte limit")
    );
}
#[test]
fn password_and_hash_length_boundaries_and_result_budgets_are_exact() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut runtime = runtime(
        PasswordConfig {
            max_password_bytes: 4,
            ..low_cost()
        },
        options,
    );
    run(
        &mut runtime,
        "$hash=Password::hash('éé');$ok=Password::verify('éé',$hash);",
    );
    assert_eq!(runtime.get_global("ok"), Some(&Value::Bool(true)));
    let Some(Value::String(hash)) = runtime.get_global("hash") else {
        panic!()
    };
    let hash = hash.clone();
    let length = hash.len();
    assert!(
        runtime
            .execute_source("<?scripta Password::hash('ééx');", &mut Vec::new())
            .unwrap_err()
            .message
            .contains("input exceeds")
    );
    assert!(
        runtime
            .execute_source("<?scripta Password::verify('ééx',$hash);", &mut Vec::new())
            .unwrap_err()
            .message
            .contains("input exceeds")
    );
    let mut tight = self::runtime(
        PasswordConfig {
            max_hash_bytes: length,
            ..low_cost()
        },
        RuntimeOptions::default(),
    );
    tight
        .set_global("hash", Value::String(hash.clone()))
        .unwrap();
    assert!(boolean(&mut tight, "Password::verify('éé',$hash)"));
    tight
        .set_global("hash", Value::String(format!("{hash}x")))
        .unwrap();
    assert!(
        tight
            .execute_source("<?scripta Password::verify('éé',$hash);", &mut Vec::new())
            .unwrap_err()
            .message
            .contains("hash exceeds")
    );
    for (limit, success) in [
        (size_of::<Value>() + length, true),
        (size_of::<Value>() + length - 1, false),
    ] {
        let mut runtime = self::runtime(
            low_cost(),
            RuntimeOptions {
                max_value_bytes: Some(limit),
                ..Default::default()
            },
        );
        runtime.set_global("saved", Value::Null).unwrap();
        let result =
            runtime.execute_source("<?scripta $saved=Password::hash('p');", &mut Vec::new());
        assert_eq!(result.is_ok(), success);
        if !success {
            assert!(result.unwrap_err().message.contains("value byte limit"));
            assert_eq!(runtime.get_global("saved"), Some(&Value::Null));
        }
    }
}
#[test]
fn invalid_signatures_and_policy_failures_do_not_expose_input_in_diagnostics() {
    for call in [
        "Password::hash()",
        "Password::hash(1)",
        "Password::hash(null)",
        "Password::hash(Bytes::encodeUtf8('p'))",
        "Password::hash('p',1)",
        "Password::verify('p')",
        "Password::verify([], 'x')",
        "Password::needsRehash(false)",
    ] {
        let mut runtime = runtime(low_cost(), RuntimeOptions::default());
        runtime.set_global("saved", Value::Int(42)).unwrap();
        let source = format!("<?scripta function check() {{return {call};}} $saved=check();");
        let error = runtime
            .execute_source(&source, &mut Vec::new())
            .unwrap_err();
        assert!(
            error.message.starts_with("TypeError:"),
            "{call}: {}",
            error.message
        );
        assert_eq!(runtime.get_global("saved"), Some(&Value::Int(42)));
        assert_eq!(&source[error.span.start..error.span.end], call);
        assert!(error.trace.iter().any(|f| f.function == "check"));
    }
    let mut runtime = runtime(
        PasswordConfig {
            max_password_bytes: 4,
            ..low_cost()
        },
        RuntimeOptions::default(),
    );
    let source = "<?scripta Password::hash('confidential-literal-password');";
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    for rendered in [
        error.to_string(),
        error.render("example.scripta", source),
        format!("{error:?}"),
    ] {
        assert!(!rendered.contains("confidential-literal-password"));
    }
}
#[test]
fn invalid_host_policies_fail_at_registration_and_cancellation_is_fatal() {
    for config in [
        PasswordConfig {
            max_memory_kib: 262_145,
            ..low_cost()
        },
        PasswordConfig {
            max_iterations: 33,
            ..low_cost()
        },
        PasswordConfig {
            max_parallelism: 17,
            ..low_cost()
        },
        PasswordConfig {
            max_work_kib: 0,
            ..low_cost()
        },
        PasswordConfig {
            max_password_bytes: 0,
            ..low_cost()
        },
        PasswordConfig {
            max_hash_bytes: 10,
            ..low_cost()
        },
        PasswordConfig {
            memory_kib: 32,
            parallelism: 5,
            ..low_cost()
        },
        PasswordConfig {
            max_work_kib: 63,
            ..low_cost()
        },
    ] {
        let mut runtime = Runtime::new();
        let error = scr_stdlib::register_with_options(
            &mut runtime,
            StdlibOptions {
                password: config,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, "PasswordError");
    }
    let mut runtime = runtime(
        low_cost(),
        RuntimeOptions {
            interrupt_flag: Some(Arc::new(AtomicBool::new(true))),
            ..Default::default()
        },
    );
    runtime.set_global("saved", Value::Int(42)).unwrap();
    let error = runtime
        .execute_source(
            "<?scripta try {$saved=Password::hash('p');} catch(Error $e) {$saved=0;}",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.contains("execution interrupted"));
    assert_eq!(runtime.get_global("saved"), Some(&Value::Int(42)));
}

#[test]
fn supported_salt_and_digest_boundaries_and_parameter_order_verify() {
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version, password_hash::SaltString};
    let mut runtime = runtime(low_cost(), RuntimeOptions::default());
    for (salt_bytes, digest_bytes) in [(8, 16), (48, 64)] {
        let params = Params::new(32, 2, 1, Some(digest_bytes)).unwrap();
        let salt = SaltString::encode_b64(&vec![7; salt_bytes]).unwrap();
        let hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password(b"public", &salt)
            .unwrap()
            .to_string();
        runtime.set_global("hash", Value::String(hash)).unwrap();
        assert!(boolean(&mut runtime, "Password::verify('public',$hash)"));
        assert!(boolean(&mut runtime, "Password::needsRehash($hash)"));
    }
    let fixture = fixture();
    let reordered = fixtures(&fixture)[0]["hash"]
        .as_str()
        .unwrap()
        .replace("m=32,t=2,p=1", "p=1,m=32,t=2");
    runtime
        .set_global("hash", Value::String(reordered))
        .unwrap();
    assert!(boolean(
        &mut runtime,
        "Password::verify('public-fixture-password',$hash)"
    ));
    assert!(!boolean(&mut runtime, "Password::needsRehash($hash)"));
}
