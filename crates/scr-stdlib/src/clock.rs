//! Process-local elapsed time, independent of the adjustable wall clock.
use scr_runtime::{NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

static ORIGIN: OnceLock<Instant> = OnceLock::new();

pub(crate) fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Clock::monotonicMs",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Float),
            |context, _| {
                context.check_cancelled()?;
                Ok(Value::Float(milliseconds(
                    ORIGIN.get_or_init(Instant::now).elapsed(),
                )))
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliseconds_preserve_submillisecond_precision_and_remain_finite() {
        assert_eq!(milliseconds(Duration::ZERO), 0.0);
        assert_eq!(milliseconds(Duration::from_micros(250)), 0.25);
        assert!((milliseconds(Duration::from_nanos(1_234_567)) - 1.234567).abs() < 1e-12);
        assert!(milliseconds(Duration::MAX).is_finite());
    }
}
