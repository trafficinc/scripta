//! Explicit binary values and UTF-8 conversion.
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::mem::size_of;
use subtle::{Choice, ConstantTimeEq};

fn timing_safe_equal(
    left: &[u8],
    right: &[u8],
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<bool> {
    checkpoint()?;
    // Lengths are public. Accumulate opaque Choices across every equal-length
    // chunk; never branch on the comparison result until the scan is complete.
    if left.len() != right.len() {
        return Ok(false);
    }
    let mut equal = Choice::from(1);
    for (left, right) in left.chunks(16 * 1024).zip(right.chunks(16 * 1024)) {
        checkpoint()?;
        equal &= left.ct_eq(right);
    }
    checkpoint()?;
    Ok(bool::from(equal))
}

fn compare(context: &RuntimeContext<'_>, left: &[u8], right: &[u8]) -> NativeResult<Value> {
    context.check_cancelled()?;
    payload(context, 0)?;
    timing_safe_equal(left, right, || context.check_cancelled()).map(Value::Bool)
}

pub(crate) fn budget(context: &RuntimeContext<'_>, size: usize) -> NativeResult<()> {
    if context.max_value_bytes.is_some_and(|limit| size > limit) {
        return Err(resource("result exceeds execution value byte limit"));
    }
    Ok(())
}
pub(crate) fn resource(message: &str) -> NativeError {
    NativeError::new("ResourceLimitError", message)
}
pub(crate) fn grow(
    context: &RuntimeContext<'_>,
    size: &mut usize,
    extra: usize,
) -> NativeResult<()> {
    *size = size
        .checked_add(extra)
        .ok_or_else(|| resource("result size overflow"))?;
    budget(context, *size)
}
pub(crate) fn payload(context: &RuntimeContext<'_>, size: usize) -> NativeResult<()> {
    let mut total = size_of::<Value>();
    grow(context, &mut total, size)
}
pub(crate) fn offset(value: &Value) -> NativeResult<usize> {
    let Value::Int(value) = value else {
        unreachable!()
    };
    usize::try_from(*value)
        .map_err(|_| NativeError::new("RangeError", "offset and length must be nonnegative ints"))
}
pub(crate) fn range(args: &[Value], count: usize) -> NativeResult<std::ops::Range<usize>> {
    let start = offset(&args[1])?;
    if start > count {
        return Err(NativeError::new("RangeError", "start is out of bounds"));
    }
    let length = args
        .get(2)
        .map(offset)
        .transpose()?
        .unwrap_or(count - start);
    if length > count - start {
        return Err(NativeError::new("RangeError", "length is out of bounds"));
    }
    Ok(start..start + length)
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Bool, Bytes, Int, String as Text};
    runtime.register(
        "Bytes::timingSafeEqual",
        NativeFunction::new(
            NativeSignature::new(vec![Bytes, Bytes], Bool),
            |context, args| {
                let (Value::Bytes(left), Value::Bytes(right)) = (&args[0], &args[1]) else {
                    unreachable!()
                };
                compare(context, left, right)
            },
        ),
    )?;
    runtime.register(
        "Bytes::encodeUtf8",
        NativeFunction::new(NativeSignature::new(vec![Text], Bytes), |context, args| {
            let Value::String(text) = &args[0] else {
                unreachable!()
            };
            payload(context, text.len())?;
            Ok(Value::Bytes(text.as_bytes().into()))
        }),
    )?;
    runtime.register(
        "Bytes::decodeUtf8",
        NativeFunction::new(NativeSignature::new(vec![Bytes], Text), |context, args| {
            let Value::Bytes(bytes) = &args[0] else {
                unreachable!()
            };
            let text = std::str::from_utf8(bytes)
                .map_err(|e| NativeError::new("EncodingError", format!("invalid UTF-8: {e}")))?;
            payload(context, bytes.len())?;
            Ok(Value::String(text.into()))
        }),
    )?;
    runtime.register(
        "Bytes::slice",
        NativeFunction::new(
            NativeSignature::new(vec![Bytes, Int], Bytes).with_optional(vec![Int]),
            |context, args| {
                let Value::Bytes(bytes) = &args[0] else {
                    unreachable!()
                };
                let range = range(args, bytes.len())?;
                payload(context, range.len())?;
                Ok(Value::Bytes(bytes[range].into()))
            },
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_comparison_checks_exact_result_budget_and_cancellation() {
        use scr_runtime::ContextOptions;
        use std::sync::{Arc, atomic::AtomicBool};
        let mut settings = ContextOptions::default();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        let mut context = RuntimeContext {
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
            max_value_bytes: Some(size_of::<Value>()),
        };
        for (left, right, equal) in [
            (b"".as_slice(), b"".as_slice(), true),
            (b"x".as_slice(), b"y".as_slice(), false),
            (b"x".as_slice(), b"".as_slice(), false),
        ] {
            context.max_value_bytes = Some(size_of::<Value>());
            assert_eq!(compare(&context, left, right).unwrap(), Value::Bool(equal));
            context.max_value_bytes = Some(size_of::<Value>() - 1);
            assert_eq!(
                compare(&context, left, right).unwrap_err().kind,
                "ResourceLimitError"
            );
            context.max_value_bytes = None;
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            assert_eq!(
                compare(&context, left, right).unwrap_err().kind,
                "Interrupted"
            );
            context.cancellation = None;
        }
    }

    #[test]
    fn comparison_scans_all_chunks_after_early_and_late_mismatches() {
        let left = vec![0xa5; 32769];
        for mismatch in [None, Some(0), Some(16383), Some(16384), Some(32768)] {
            let mut right = left.clone();
            if let Some(index) = mismatch {
                right[index] ^= 1;
            }
            let mut calls = 0;
            let equal = timing_safe_equal(&left, &right, || {
                calls += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(equal, mismatch.is_none());
            // Entry, three chunk checkpoints, completion: no mismatch shortcut.
            assert_eq!(calls, 5);
        }
    }

    #[test]
    fn comparison_cancellation_is_observed_even_after_a_mismatch() {
        let left = vec![0xa5; 32769];
        let mut right = left.clone();
        right[0] ^= 1;
        for stop in 0..5 {
            let mut calls = 0;
            let result = timing_safe_equal(&left, &right, || {
                let current = calls;
                calls += 1;
                if current == stop {
                    Err(NativeError::new("Interrupted", "execution interrupted"))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.unwrap_err().kind, "Interrupted");
            assert_eq!(calls, stop + 1);
        }
        for (left, right) in [
            (b"".as_slice(), b"".as_slice()),
            (b"x".as_slice(), b"".as_slice()),
        ] {
            assert_eq!(
                timing_safe_equal(left, right, || Err(NativeError::new(
                    "Interrupted",
                    "execution interrupted"
                )))
                .unwrap_err()
                .kind,
                "Interrupted"
            );
        }
    }
}
