//! UTC dates use signed Unix milliseconds throughout the public API.
use chrono::{
    DateTime, Utc,
    format::{Item, StrftimeItems},
};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use std::{
    fmt::Write,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn now() -> NativeResult<i64> {
    let milliseconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_millis()).map_err(|_| range())?,
        Err(error) => {
            -i128::try_from(error.duration().as_nanos().div_ceil(1_000_000)).map_err(|_| range())?
        }
    };
    i64::try_from(milliseconds).map_err(|_| range())
}
fn range() -> NativeError {
    NativeError::new("RangeError", "timestamp is outside the supported range")
}
pub(crate) fn timestamp(value: i64) -> NativeResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(value).ok_or_else(range)
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Date::now",
        NativeFunction::new(NativeSignature::new(vec![], NativeType::Int), |_, _| {
            now().map(Value::Int)
        }),
    )?;
    runtime.register(
        "Date::format",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Int, NativeType::String],
                NativeType::String,
            ),
            |_, args| {
                let [Value::Int(value), Value::String(format)] = args else {
                    unreachable!()
                };
                let date = timestamp(*value)?;
                let items: Vec<_> = StrftimeItems::new(format).collect();
                if items.iter().any(|item| matches!(item, Item::Error)) {
                    return Err(NativeError::new(
                        "DateError",
                        "invalid date format directive",
                    ));
                }
                let mut output = String::new();
                write!(&mut output, "{}", date.format_with_items(items.iter())).map_err(|_| {
                    NativeError::new("DateError", "date directive cannot be formatted")
                })?;
                Ok(Value::String(output))
            },
        ),
    )?;
    runtime.register(
        "Date::parse",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Int),
            |_, args| {
                let [Value::String(value)] = args else {
                    unreachable!()
                };
                let date = DateTime::parse_from_rfc3339(value).map_err(|_| {
                    NativeError::new("DateError", "expected RFC 3339 date with explicit timezone")
                })?;
                // Unix timestamps do not represent leap seconds.
                if date.timestamp_subsec_nanos() >= 1_000_000_000 {
                    return Err(NativeError::new(
                        "DateError",
                        "leap seconds are not supported",
                    ));
                }
                Ok(Value::Int(date.timestamp_millis()))
            },
        ),
    )?;
    runtime.register(
        "Date::sleep",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Null),
            crate::sleep_value,
        ),
    )?;
    Ok(())
}
