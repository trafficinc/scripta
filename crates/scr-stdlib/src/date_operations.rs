//! Calendar operations deliberately separate from fixed elapsed seconds.
use crate::date::{date_error, range, resolve_timezone};
use chrono::{DateTime, Utc};
use jiff::{Span, Timestamp};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};

fn integer(context: &RuntimeContext<'_>, value: i64) -> NativeResult<Value> {
    context.check_cancelled()?;
    crate::bytes::payload(context, 0)?;
    Ok(Value::Int(value))
}

fn timestamp(value: i64) -> NativeResult<i64> {
    DateTime::<Utc>::from_timestamp(value, 0).ok_or_else(range)?;
    Ok(value)
}

fn add_calendar(
    context: &RuntimeContext<'_>,
    value: i64,
    amount: i64,
    unit: &str,
    timezone: &str,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    let span = match unit {
        "year" => Span::new().try_years(amount),
        "month" => Span::new().try_months(amount),
        "week" => Span::new().try_weeks(amount),
        "day" => Span::new().try_days(amount),
        _ => {
            return Err(date_error(
                "calendar unit must be year, month, week, or day",
            ));
        }
    }
    .map_err(|_| range())?;
    let instant = Timestamp::from_second(value).map_err(|_| range())?;
    let zone = resolve_timezone(timezone)?;
    if amount == 0 {
        return integer(context, value);
    }
    let local = zone.to_datetime(instant);
    // Civil arithmetic clamps month/year ends, but has no DST disambiguation.
    let result = local.checked_add(span).map_err(|_| range())?;
    let candidate = zone.to_ambiguous_timestamp(result);
    // Distinguish representational overflow from a genuine DST gap/overlap.
    if candidate.is_ambiguous() {
        return Err(date_error(
            "calendar result is an ambiguous or nonexistent local time",
        ));
    }
    let result = candidate.unambiguous().map_err(|_| range())?;
    integer(context, result.as_second())
}

fn duration_error() -> NativeError {
    NativeError::new(
        "DurationError",
        "expected whole-second duration [-]P[nD][T[nH][nM][nS]]",
    )
}

fn duration_range() -> NativeError {
    NativeError::new(
        "RangeError",
        "duration exceeds signed 64-bit whole-second range",
    )
}

fn parse_duration(context: &RuntimeContext<'_>, text: &str) -> NativeResult<Value> {
    context.check_cancelled()?;
    if text.len() > 128 {
        return Err(crate::bytes::resource(
            "duration input exceeds 128 UTF-8 bytes",
        ));
    }
    let bytes = text.as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let mut index = usize::from(negative);
    if bytes.get(index) != Some(&b'P') {
        return Err(duration_error());
    }
    index += 1;
    let mut time = false;
    let mut last = 0;
    let mut components = 0;
    let mut time_components = 0;
    let mut total = 0_i128;
    while index < bytes.len() {
        context.check_cancelled()?;
        if bytes[index] == b'T' {
            if time {
                return Err(duration_error());
            }
            time = true;
            index += 1;
            continue;
        }
        let start = index;
        let mut count = 0_i128;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            count = count
                .checked_mul(10)
                .and_then(|n| n.checked_add(i128::from(bytes[index] - b'0')))
                .ok_or_else(duration_range)?;
            index += 1;
        }
        if index == start {
            return Err(duration_error());
        }
        let (order, factor) = match (time, bytes.get(index)) {
            (false, Some(b'D')) => (1, 86400),
            (true, Some(b'H')) => (2, 3600),
            (true, Some(b'M')) => (3, 60),
            (true, Some(b'S')) => (4, 1),
            _ => return Err(duration_error()),
        };
        if order <= last {
            return Err(duration_error());
        }
        last = order;
        index += 1;
        components += 1;
        time_components += usize::from(time);
        total = count
            .checked_mul(factor)
            .and_then(|part| total.checked_add(part))
            .ok_or_else(duration_range)?;
    }
    if components == 0 || (time && time_components == 0) {
        return Err(duration_error());
    }
    if negative {
        total = -total;
    }
    integer(context, i64::try_from(total).map_err(|_| duration_range())?)
}

fn elapsed(
    context: &RuntimeContext<'_>,
    left: i64,
    right: i64,
    difference: bool,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    timestamp(left)?;
    let result = if difference {
        timestamp(right)?.checked_sub(left).ok_or_else(range)?
    } else {
        timestamp(left.checked_add(right).ok_or_else(range)?)?
    };
    integer(context, result)
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Int, String as Text};
    runtime.register(
        "Date::addCalendar",
        NativeFunction::new(
            NativeSignature::new(vec![Int, Int, Text], Int).with_optional(vec![Text]),
            |context, args| {
                let (Value::Int(value), Value::Int(amount), Value::String(unit)) =
                    (&args[0], &args[1], &args[2])
                else {
                    unreachable!()
                };
                let timezone = match args.get(3) {
                    Some(Value::String(zone)) => zone.as_str(),
                    None => "UTC",
                    _ => unreachable!(),
                };
                add_calendar(context, *value, *amount, unit, timezone)
            },
        ),
    )?;
    for (name, difference) in [("Date::addSeconds", false), ("Date::diff", true)] {
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(vec![Int, Int], Int),
                move |context, args| {
                    let (Value::Int(left), Value::Int(right)) = (&args[0], &args[1]) else {
                        unreachable!()
                    };
                    elapsed(context, *left, *right, difference)
                },
            ),
        )?;
    }
    runtime.register(
        "Duration::parse",
        NativeFunction::new(NativeSignature::new(vec![Text], Int), |context, args| {
            let Value::String(text) = &args[0] else {
                unreachable!()
            };
            parse_duration(context, text)
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::{
        mem::size_of,
        sync::{Arc, atomic::AtomicBool},
    };

    #[test]
    fn operations_check_native_cancellation_and_exact_integer_budgets() {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
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
            max_value_bytes: None,
        };
        for limit in [
            size_of::<Value>() - 1,
            size_of::<Value>(),
            size_of::<Value>() + 1,
        ] {
            context.max_value_bytes = Some(limit);
            for result in [
                add_calendar(&context, 0, 1, "day", "UTC"),
                add_calendar(&context, 0, 0, "day", "UTC"),
                parse_duration(&context, "P1D"),
                elapsed(&context, 0, 86400, false),
                elapsed(&context, 0, 86400, true),
            ] {
                if limit < size_of::<Value>() {
                    assert_eq!(result.unwrap_err().kind, "ResourceLimitError");
                } else {
                    assert!(result.is_ok());
                }
            }
        }
        context.max_value_bytes = None;
        context.cancellation = Some(Arc::new(AtomicBool::new(true)));
        for result in [
            add_calendar(&context, 0, 1, "day", "UTC"),
            add_calendar(&context, 0, 0, "day", "local"),
            parse_duration(&context, "P1D"),
            elapsed(&context, 0, 86400, false),
            elapsed(&context, 0, 86400, true),
        ] {
            assert_eq!(result.unwrap_err().kind, "Interrupted");
        }
        assert!(output.is_empty());
    }
}
