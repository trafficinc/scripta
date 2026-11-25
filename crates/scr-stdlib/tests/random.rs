use scr_runtime::{Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::{StdlibOptions, random::RandomConfig};
use std::mem::size_of;

fn runtime(config: RandomConfig, options: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register_with_options(
        &mut runtime,
        StdlibOptions {
            random: config,
            ..Default::default()
        },
    )
    .unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), RuntimeError> {
    runtime.execute_source(source, &mut Vec::new()).map(|_| ())
}
fn assert_error(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = run(runtime, source).unwrap_err();
    assert!(
        error.message.starts_with(&format!("{kind}:")),
        "{source}: {error}"
    );
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
}

#[test]
fn os_random_bytes_have_exact_requested_sizes_without_automation_permissions() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut runtime = runtime(RandomConfig::default(), options);
    for count in [0, 1, 32, 16383, 16384, 16385, 65536] {
        run(&mut runtime, &format!("$data=Random::bytes({count}); $kind=type($data); $equal=Bytes::timingSafeEqual($data,$data);")).unwrap();
        let Some(Value::Bytes(data)) = runtime.get_global("data") else {
            panic!("expected bytes")
        };
        assert_eq!(data.len(), count);
        assert_eq!(
            runtime.get_global("kind"),
            Some(&Value::String("bytes".into()))
        );
        assert_eq!(runtime.get_global("equal"), Some(&Value::Bool(true)));
    }
    // Do not assert entropy contents, uniqueness, nonzero bytes, or timing.
}

#[test]
fn host_and_value_limits_preserve_assignments_and_allow_exact_requests() {
    for max_bytes in [0, 16, 1024 * 1024] {
        let mut runtime = runtime(RandomConfig { max_bytes }, RuntimeOptions::default());
        run(&mut runtime, "$result='unchanged';").unwrap();
        assert_error(
            &mut runtime,
            &format!("$result=Random::bytes({});", max_bytes + 1),
            "RandomLimitError",
        );
        run(&mut runtime, &format!("$data=Random::bytes({max_bytes});")).unwrap();
        let Some(Value::Bytes(data)) = runtime.get_global("data") else {
            panic!()
        };
        assert_eq!(data.len(), max_bytes);
    }
    for limit in [31, 32] {
        let mut runtime = runtime(
            RandomConfig::default(),
            RuntimeOptions {
                max_value_bytes: Some(size_of::<Value>() + limit),
                ..Default::default()
            },
        );
        run(&mut runtime, "$result='unchanged';").unwrap();
        if limit == 31 {
            assert_error(
                &mut runtime,
                "$result=Random::bytes(32);",
                "ResourceLimitError",
            );
        } else {
            run(&mut runtime, "$result=Random::bytes(32);").unwrap();
            let Some(Value::Bytes(data)) = runtime.get_global("result") else {
                panic!()
            };
            assert_eq!(data.len(), 32);
        }
    }
    let mut runtime = runtime(
        RandomConfig::default(),
        RuntimeOptions {
            max_value_bytes: Some(size_of::<Value>()),
            ..Default::default()
        },
    );
    run(
        &mut runtime,
        "$empty=Random::bytes(0); $equal=Bytes::timingSafeEqual($empty,$empty);",
    )
    .unwrap();
    assert_eq!(runtime.get_global("equal"), Some(&Value::Bool(true)));
}

#[test]
fn invalid_lengths_types_and_host_configuration_are_explicit_errors() {
    let mut runtime = runtime(RandomConfig::default(), RuntimeOptions::default());
    run(&mut runtime, "$result='unchanged';").unwrap();
    for count in [-1, i64::MIN] {
        runtime.set_global("count", Value::Int(count)).unwrap();
        assert_error(&mut runtime, "$result=Random::bytes($count);", "RangeError");
    }
    let oversized_kind = if usize::BITS < 64 {
        "RangeError"
    } else {
        "RandomLimitError"
    };
    assert_error(
        &mut runtime,
        "$result=Random::bytes(9223372036854775807);",
        oversized_kind,
    );
    for call in [
        "Random::bytes('32')",
        "Random::bytes(1.0)",
        "Random::bytes(true)",
        "Random::bytes(null)",
        "Random::bytes()",
        "Random::bytes(1,2)",
        "Bytes::timingSafeEqual('x','x')",
        "Bytes::timingSafeEqual(Random::bytes(0),'')",
        "Bytes::timingSafeEqual(Random::bytes(0))",
    ] {
        assert_error(&mut runtime, &format!("$result={call};"), "TypeError");
    }
    for max_bytes in [1024 * 1024 + 1, usize::MAX] {
        let mut runtime = Runtime::new();
        let error = scr_stdlib::register_with_options(
            &mut runtime,
            StdlibOptions {
                random: RandomConfig { max_bytes },
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, "RandomError");
    }
}

#[test]
fn timing_safe_equality_handles_binary_data_lengths_and_chunk_boundaries() {
    let mut runtime = runtime(RandomConfig::default(), RuntimeOptions::default());
    for length in [0usize, 1, 255, 16383, 16384, 16385, 32769] {
        let left: Vec<u8> = (0..length).map(|i| (i % 256) as u8).collect();
        runtime
            .set_global("left", Value::Bytes(left.as_slice().into()))
            .unwrap();
        runtime
            .set_global("right", Value::Bytes(left.as_slice().into()))
            .unwrap();
        run(&mut runtime, "$equal=Bytes::timingSafeEqual($left,$right);").unwrap();
        assert_eq!(runtime.get_global("equal"), Some(&Value::Bool(true)));
        for mismatch in [0, length / 2, length.saturating_sub(1)] {
            if length == 0 {
                continue;
            }
            let mut right = left.clone();
            right[mismatch] ^= 1;
            runtime
                .set_global("right", Value::Bytes(right.into()))
                .unwrap();
            run(&mut runtime, "$equal=Bytes::timingSafeEqual($left,$right);").unwrap();
            assert_eq!(runtime.get_global("equal"), Some(&Value::Bool(false)));
        }
        let mut longer = left;
        longer.push(0);
        runtime
            .set_global("right", Value::Bytes(longer.into()))
            .unwrap();
        run(&mut runtime, "$forward=Bytes::timingSafeEqual($left,$right); $reverse=Bytes::timingSafeEqual($right,$left);").unwrap();
        assert_eq!(runtime.get_global("forward"), Some(&Value::Bool(false)));
        assert_eq!(runtime.get_global("reverse"), Some(&Value::Bool(false)));
    }
}

#[test]
fn cancellation_before_calls_preserves_destinations() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = runtime(
        RandomConfig::default(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
    );
    run(
        &mut runtime,
        "$result='unchanged'; $left=Random::bytes(0); $right=Bytes::encodeUtf8('x');",
    )
    .unwrap();
    for call in [
        "Random::bytes(0)",
        "Random::bytes(32)",
        "Bytes::timingSafeEqual($left,$left)",
        "Bytes::timingSafeEqual($left,$right)",
    ] {
        flag.store(true, Ordering::Relaxed);
        let error = run(&mut runtime, &format!("$result={call};")).unwrap_err();
        assert_eq!(error.message, "execution interrupted");
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
        flag.store(false, Ordering::Relaxed);
    }
}
