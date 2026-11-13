//! Explicit binary values and UTF-8 conversion.
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::mem::size_of;

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
    use NativeType::{Bytes, Int, String as Text};
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
