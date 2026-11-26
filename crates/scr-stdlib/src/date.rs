//! Unix seconds with UTC-compatible formatting and bundled timezone rules.
use chrono::{
    DateTime, FixedOffset, Utc,
    format::{Fixed, Item, StrftimeItems},
};
use jiff::{
    Timestamp,
    tz::{TimeZone, TimeZoneDatabase},
};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    fmt,
    mem::size_of,
    time::{SystemTime, UNIX_EPOCH},
};

// Internal logging retains subsecond precision; public Date timestamps use seconds.
pub(crate) fn now_millis() -> NativeResult<i64> {
    let milliseconds = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_millis()).map_err(|_| range())?,
        Err(error) => {
            -i128::try_from(error.duration().as_nanos().div_ceil(1_000_000)).map_err(|_| range())?
        }
    };
    i64::try_from(milliseconds).map_err(|_| range())
}
pub(crate) fn range() -> NativeError {
    NativeError::new("RangeError", "timestamp is outside the supported range")
}
pub(crate) fn timestamp_millis(value: i64) -> NativeResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(value).ok_or_else(range)
}
pub(crate) fn date_error(message: &str) -> NativeError {
    NativeError::new("DateError", message)
}

pub(crate) fn resolve_timezone(name: &str) -> NativeResult<TimeZone> {
    if name.is_empty() || name.len() > 255 || name.contains('\0') {
        return Err(date_error("expected local, UTC, or an IANA timezone name"));
    }
    let zone = if name.eq_ignore_ascii_case("UTC") {
        TimeZone::UTC
    } else if name == "local" {
        TimeZone::try_system().map_err(|_| date_error("could not determine the local timezone"))?
    } else {
        // Always use bundled rules, even if another dependency enables system data.
        TimeZoneDatabase::bundled()
            .get(name)
            .map_err(|_| date_error("unknown IANA timezone name"))?
    };
    if zone.is_unknown() {
        return Err(date_error("could not determine the timezone"));
    }
    Ok(zone)
}

// Count the exact formatted output before allocating it. write_to avoids the
// unbounded temporary String used by Chrono's Display implementation.
struct DateWriter<'a, 'b> {
    context: &'a RuntimeContext<'b>,
    total: usize,
    output: Option<String>,
    error: Option<NativeError>,
}
impl fmt::Write for DateWriter<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let checked = self
            .context
            .check_cancelled()
            .and_then(|()| crate::bytes::grow(self.context, &mut self.total, text.len()));
        if let Err(error) = checked {
            self.error = Some(error);
            return Err(fmt::Error);
        }
        if let Some(output) = &mut self.output {
            output.push_str(text);
        }
        Ok(())
    }
}

fn format_date(
    context: &RuntimeContext<'_>,
    value: i64,
    format: &str,
    timezone: Option<&str>,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    let utc = DateTime::from_timestamp(value, 0).ok_or_else(range)?;
    for (index, item) in StrftimeItems::new(format).enumerate() {
        if index % 1024 == 0 {
            context.check_cancelled()?;
        }
        if matches!(item, Item::Error) {
            return Err(date_error("invalid date format directive"));
        }
    }
    let zone;
    let info;
    let (offset, abbreviation) = if timezone.is_none_or(|name| name.eq_ignore_ascii_case("UTC")) {
        (FixedOffset::east_opt(0).unwrap(), "UTC")
    } else {
        zone = resolve_timezone(timezone.unwrap())?;
        let instant = Timestamp::from_second(value).map_err(|_| range())?;
        info = zone.to_offset_info(instant);
        (
            FixedOffset::east_opt(info.offset().seconds()).ok_or_else(range)?,
            info.abbreviation(),
        )
    };
    // Keep Chrono's established strftime semantics, including %s and %+, but
    // replace %Z with the abbreviation at this instant (not the numeric offset).
    let date = utc.with_timezone(&offset);
    let items = StrftimeItems::new(format).map(|item| match item {
        Item::Fixed(Fixed::TimezoneName) => Item::Literal(abbreviation),
        other => other,
    });
    let formatted = date.format_with_items(items);
    let mut writer = DateWriter {
        context,
        total: size_of::<Value>(),
        output: None,
        error: None,
    };
    crate::bytes::budget(context, writer.total)?;
    formatted.write_to(&mut writer).map_err(|_| {
        writer
            .error
            .take()
            .unwrap_or_else(|| date_error("date directive cannot be formatted"))
    })?;
    context.check_cancelled()?;
    let mut output = String::new();
    output
        .try_reserve_exact(writer.total - size_of::<Value>())
        .map_err(|_| crate::bytes::resource("date result allocation failed"))?;
    writer.total = size_of::<Value>();
    writer.output = Some(output);
    formatted.write_to(&mut writer).map_err(|_| {
        writer
            .error
            .take()
            .unwrap_or_else(|| date_error("date directive cannot be formatted"))
    })?;
    context.check_cancelled()?;
    Ok(Value::String(writer.output.unwrap()))
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Date::now",
        NativeFunction::new(NativeSignature::new(vec![], NativeType::Int), |_, _| {
            now_millis().map(|milliseconds| Value::Int(milliseconds.div_euclid(1000)))
        }),
    )?;
    runtime.register(
        "Date::format",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::Int, NativeType::String],
                NativeType::String,
            )
            .with_optional(vec![NativeType::String]),
            |context, args| {
                let (Value::Int(value), Value::String(format)) = (&args[0], &args[1]) else {
                    unreachable!()
                };
                let timezone = args.get(2).map(|argument| {
                    let Value::String(name) = argument else {
                        unreachable!()
                    };
                    name.as_str()
                });
                format_date(context, *value, format, timezone)
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
                Ok(Value::Int(date.timestamp()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn date_formatting_preflights_exact_budgets_and_checks_native_cancellation() {
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
        for (format, zone, expected) in [
            ("", None, ""),
            ("世界 %Y %Z", None, "世界 1970 UTC"),
            (
                "%Y-%m-%d %H:%M %:z %Z",
                Some("America/New_York"),
                "1969-12-31 19:00 -05:00 EST",
            ),
            (
                "%c%c%c",
                Some("Asia/Tokyo"),
                "Thu Jan  1 09:00:00 1970Thu Jan  1 09:00:00 1970Thu Jan  1 09:00:00 1970",
            ),
        ] {
            let required = size_of::<Value>() + expected.len();
            context.max_value_bytes = Some(required);
            assert_eq!(
                format_date(&context, 0, format, zone).unwrap(),
                Value::String(expected.into())
            );
            context.max_value_bytes = Some(required - 1);
            assert_eq!(
                format_date(&context, 0, format, zone).unwrap_err().kind,
                "ResourceLimitError"
            );
            context.max_value_bytes = Some(required + 1);
            assert_eq!(
                format_date(&context, 0, format, zone).unwrap(),
                Value::String(expected.into())
            );
        }
        context.max_value_bytes = None;
        context.cancellation = Some(Arc::new(AtomicBool::new(true)));
        for zone in [None, Some("UTC"), Some("local"), Some("Asia/Tokyo")] {
            assert_eq!(
                format_date(&context, 0, "%Y", zone).unwrap_err().kind,
                "Interrupted"
            );
        }
        assert!(output.is_empty());
    }
}
