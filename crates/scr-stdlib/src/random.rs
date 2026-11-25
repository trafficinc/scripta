//! Bounded OS entropy. No deterministic fallback or script-selectable backend.
use crate::bytes::{payload, resource};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use zeroize::Zeroizing;

const CHUNK_BYTES: usize = 16 * 1024;
const MAX_BYTES: usize = 1024 * 1024;

/// Trusted host ceiling for each Random::bytes request, independent of the
/// entropy used internally by password hashing or web sessions.
#[derive(Clone, Debug)]
pub struct RandomConfig {
    /// Zero permits only empty requests; the maximum supported ceiling is 1 MiB.
    pub max_bytes: usize,
}
impl Default for RandomConfig {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024,
        }
    }
}
impl RandomConfig {
    fn validate(&self) -> NativeResult<()> {
        if self.max_bytes > MAX_BYTES {
            return Err(NativeError::new(
                "RandomError",
                "invalid host random byte limit",
            ));
        }
        Ok(())
    }
}

// Injection is private and used only by unit tests; the public registrar always
// supplies getrandom::fill and never retries with a weaker entropy source.
fn generate(
    context: &RuntimeContext<'_>,
    count: i64,
    config: &RandomConfig,
    mut fill: impl FnMut(&mut [u8]) -> Result<(), getrandom::Error>,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    let count = usize::try_from(count).map_err(|_| {
        NativeError::new(
            "RangeError",
            "random byte count must be a nonnegative int in platform range",
        )
    })?;
    if count > config.max_bytes {
        return Err(NativeError::new(
            "RandomLimitError",
            "random byte count exceeds host limit",
        ));
    }
    payload(context, count)?;
    let mut result = Zeroizing::new(Vec::new());
    result
        .try_reserve_exact(count)
        .map_err(|_| resource("random bytes allocation failed"))?;
    result.resize(count, 0);
    for chunk in result.chunks_mut(CHUNK_BYTES) {
        context.check_cancelled()?;
        fill(chunk).map_err(|_| NativeError::new("RandomError", "OS entropy generation failed"))?;
    }
    context.check_cancelled()?;
    Ok(Value::Bytes(result.as_slice().into()))
}

pub(crate) fn register(runtime: &mut Runtime, config: RandomConfig) -> NativeResult<()> {
    register_with_entropy(runtime, config, getrandom::fill)
}

fn register_with_entropy(
    runtime: &mut Runtime,
    config: RandomConfig,
    fill: impl Fn(&mut [u8]) -> Result<(), getrandom::Error> + 'static,
) -> NativeResult<()> {
    config.validate()?;
    runtime.register(
        "Random::bytes",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Bytes),
            move |context, args| {
                let Value::Int(count) = args[0] else {
                    unreachable!()
                };
                generate(context, count, &config, &fill)
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::{
        cell::Cell,
        mem::size_of,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
        let mut settings = ContextOptions::default();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: None,
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }

    #[test]
    fn exact_host_and_value_limits_precede_entropy_calls_including_empty_results() {
        with_context(|context| {
            for count in [
                0,
                1,
                CHUNK_BYTES - 1,
                CHUNK_BYTES,
                CHUNK_BYTES + 1,
                MAX_BYTES,
            ] {
                let config = RandomConfig { max_bytes: count };
                let calls = Cell::new(0);
                let backend = |chunk: &mut [u8]| {
                    assert!(!chunk.is_empty() && chunk.len() <= CHUNK_BYTES);
                    calls.set(calls.get() + 1);
                    chunk.fill(0xa5);
                    Ok(())
                };
                context.max_value_bytes = Some(size_of::<Value>() + count);
                let output = generate(context, count as i64, &config, backend).unwrap();
                assert_eq!(output, Value::Bytes(vec![0xa5; count].into()));
                assert_eq!(calls.get(), count.div_ceil(CHUNK_BYTES));
                calls.set(0);
                context.max_value_bytes = Some(size_of::<Value>() + count - 1);
                assert_eq!(
                    generate(context, count as i64, &config, backend)
                        .unwrap_err()
                        .kind,
                    "ResourceLimitError"
                );
                assert_eq!(calls.get(), 0);
                context.max_value_bytes = None;
                assert_eq!(
                    generate(context, count as i64 + 1, &config, backend)
                        .unwrap_err()
                        .kind,
                    "RandomLimitError"
                );
                assert_eq!(calls.get(), 0);
            }
            for count in [-1, i64::MIN] {
                assert_eq!(
                    generate(context, count, &RandomConfig::default(), |_| panic!(
                        "invalid count must not request entropy"
                    ))
                    .unwrap_err()
                    .kind,
                    "RangeError"
                );
            }
        });
    }

    #[test]
    fn entropy_failure_never_returns_partial_bytes_or_overwrites_assignment() {
        for fail_at in [0, 1, 2] {
            let mut runtime = Runtime::new();
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            register_with_entropy(&mut runtime, RandomConfig::default(), move |chunk| {
                let current = observed.get();
                observed.set(current + 1);
                // Model a backend that writes some output before reporting failure.
                chunk.fill(0xa5);
                if current == fail_at {
                    Err(getrandom::Error::UNSUPPORTED)
                } else {
                    Ok(())
                }
            })
            .unwrap();
            runtime
                .execute_source(
                    "$result='unchanged'; function randomData() { return Random::bytes(32769); }",
                    &mut Vec::new(),
                )
                .unwrap();
            let error = runtime
                .execute_source("$result=randomData();", &mut Vec::new())
                .unwrap_err();
            assert_eq!(error.message, "RandomError: OS entropy generation failed");
            assert!(error.trace.iter().any(|f| f.function == "Random::bytes"));
            assert!(error.trace.iter().any(|f| f.function == "randomData"));
            assert_eq!(
                runtime.get_global("result"),
                Some(&Value::String("unchanged".into()))
            );
            assert_eq!(calls.get(), fail_at + 1);
            // A failing entropy source is never used for an empty request.
            runtime
                .execute_source("$empty=Random::bytes(0);", &mut Vec::new())
                .unwrap();
            assert_eq!(calls.get(), fail_at + 1);
            assert_eq!(
                runtime.get_global("empty"),
                Some(&Value::Bytes(Vec::<u8>::new().into()))
            );
        }
    }

    #[test]
    fn cancellation_before_between_and_after_entropy_chunks_returns_no_partial_result() {
        with_context(|context| {
            let config = RandomConfig::default();
            let flag = Arc::new(AtomicBool::new(true));
            context.cancellation = Some(flag.clone());
            for count in [0, 1, CHUNK_BYTES as i64 + 1] {
                assert_eq!(
                    generate(context, count, &config, |_| panic!(
                        "cancelled before entropy"
                    ))
                    .unwrap_err()
                    .kind,
                    "Interrupted"
                );
            }
            for cancel_after in 1..=3 {
                flag.store(false, Ordering::Relaxed);
                let mut calls = 0;
                let result = generate(context, 32769, &config, |chunk| {
                    chunk.fill(0xa5);
                    calls += 1;
                    if calls == cancel_after {
                        flag.store(true, Ordering::Relaxed);
                    }
                    Ok(())
                });
                assert_eq!(result.unwrap_err().kind, "Interrupted");
                assert_eq!(calls, cancel_after);
            }
        });
        // Exercise the actual native callback and assignment behavior as well.
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(scr_runtime::RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        });
        register_with_entropy(&mut runtime, RandomConfig::default(), move |chunk| {
            chunk.fill(0xa5);
            flag.store(true, Ordering::Relaxed);
            Ok(())
        })
        .unwrap();
        runtime
            .execute_source(
                "$result='unchanged'; $caught=false; $cleaned=false;",
                &mut Vec::new(),
            )
            .unwrap();
        let error = runtime
            .execute_source("try { $result=Random::bytes(1); } catch (Error $e) { $caught=true; } finally { $cleaned=true; }", &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.message, "Interrupted: execution interrupted");
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
        assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(false)));
        assert_eq!(runtime.get_global("cleaned"), Some(&Value::Bool(false)));
    }
}
