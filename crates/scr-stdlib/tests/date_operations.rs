use scr_runtime::{Runtime, RuntimeOptions, Value};

fn runtime() -> Runtime {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}

fn evaluate(runtime: &mut Runtime, expression: &str) -> Value {
    runtime.execute_source(&format!("$result = {expression};"), &mut Vec::new()).unwrap();
    runtime.get_global("result").unwrap().clone()
}

fn failure(runtime: &mut Runtime, expression: &str, kind: &str) {
    runtime.set_global("result", Value::Int(42)).unwrap();
    let error = runtime.execute_source(&format!("$result = {expression};"), &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with(&format!("{kind}:")), "{expression}: {error}");
    assert_eq!(runtime.get_global("result"), Some(&Value::Int(42)));
    assert!(error.span.end > error.span.start);
}

#[test]
fn calendar_arithmetic_clamps_month_ends_and_preserves_wall_time() {
    let mut runtime = runtime();
    for (input, amount, unit, expected) in [
        ("2024-01-31T12:34:56Z", 1, "month", "2024-02-29T12:34:56Z"),
        ("2023-01-31T12:34:56Z", 1, "month", "2023-02-28T12:34:56Z"),
        ("2024-02-29T12:34:56Z", 1, "year", "2025-02-28T12:34:56Z"),
        ("2024-03-31T12:34:56Z", -1, "month", "2024-02-29T12:34:56Z"),
        ("2024-12-31T12:34:56Z", 1, "day", "2025-01-01T12:34:56Z"),
        ("1970-01-02T12:34:56Z", -1, "week", "1969-12-26T12:34:56Z"),
        ("2024-01-31T12:34:56Z", 2, "month", "2024-03-31T12:34:56Z"),
    ] {
        let value = evaluate(&mut runtime, &format!(
            "Date::format(Date::addCalendar(Date::parse(\"{input}\"), {amount}, \"{unit}\"), \"%Y-%m-%dT%H:%M:%SZ\")"));
        assert_eq!(value, Value::String(expected.into()));
    }
    // The selected local date can differ from its UTC date at month end.
    assert_eq!(evaluate(&mut runtime, r#"Date::format(Date::addCalendar(Date::parse("2024-03-01T01:00:00Z"), 1, "month", "America/New_York"), "%Y-%m-%d %H:%M:%S %:z", "America/New_York")"#),
        Value::String("2024-03-29 20:00:00 -04:00".into()));
    assert_eq!(evaluate(&mut runtime, r#"Date::addCalendar(-1, 0, "day", "uTc")"#), Value::Int(-1));
}

#[test]
fn calendar_days_and_weeks_are_distinct_from_elapsed_durations() {
    let mut runtime = runtime();
    for (input, amount, unit, expected) in [
        ("2024-03-09T12:00:00-05:00", 1, "day", 23 * 3600),
        ("2024-11-02T12:00:00-04:00", 1, "day", 25 * 3600),
        ("2024-03-03T12:00:00-05:00", 1, "week", 167 * 3600),
        ("2024-03-10T12:00:00-04:00", -1, "day", -23 * 3600),
    ] {
        assert_eq!(evaluate(&mut runtime, &format!(
            "Date::diff(Date::parse(\"{input}\"), Date::addCalendar(Date::parse(\"{input}\"), {amount}, \"{unit}\", \"America/New_York\"))")), Value::Int(expected));
    }
    assert_eq!(evaluate(&mut runtime, r#"Date::format(Date::addSeconds(Date::parse("2024-03-09T12:00:00-05:00"), Duration::parse("P1D")), "%H:%M %:z", "America/New_York")"#), Value::String("13:00 -04:00".into()));
    assert_eq!(evaluate(&mut runtime, "Date::diff(1, -1)"), Value::Int(-2));
    assert_eq!(evaluate(&mut runtime, "Date::addSeconds(0, -1)"), Value::Int(-1));
    assert_eq!(evaluate(&mut runtime, "Date::diff(-1, -1)"), Value::Int(0));
}

#[test]
fn calendar_rejects_dst_gaps_overlaps_and_skipped_dates_without_guessing() {
    let mut runtime = runtime();
    for (input, zone) in [
        ("2024-03-09T02:30:00-05:00", "America/New_York"),
        ("2024-11-02T01:30:00-04:00", "America/New_York"),
        ("2024-10-05T02:15:00+10:30", "Australia/Lord_Howe"),
        ("2011-12-29T12:00:00-10:00", "Pacific/Apia"),
    ] {
        failure(&mut runtime, &format!("Date::addCalendar(Date::parse(\"{input}\"), 1, \"day\", \"{zone}\")"), "DateError");
    }
    // Zero preserves either known instant in a repeated wall-clock hour.
    for offset in ["-04:00", "-05:00"] {
        assert_eq!(evaluate(&mut runtime, &format!("Date::diff(Date::parse(\"2024-11-03T01:30:00{offset}\"), Date::addCalendar(Date::parse(\"2024-11-03T01:30:00{offset}\"), 0, \"day\", \"America/New_York\"))")), Value::Int(0));
    }
}

#[test]
fn durations_are_strict_signed_whole_seconds_and_cover_full_integer_range() {
    let mut runtime = runtime();
    for (text, expected) in [
        ("PT0S", 0), ("-P0D", 0), ("P1D", 86400),
        ("P2DT3H4M5S", 183845), ("-PT90M", -5400), ("PT0001S", 1),
        ("PT9223372036854775807S", i64::MAX),
        ("-PT9223372036854775808S", i64::MIN),
    ] {
        runtime.set_global("text", Value::String(text.into())).unwrap();
        assert_eq!(evaluate(&mut runtime, "Duration::parse($text)"), Value::Int(expected));
    }
    for text in ["", "P", "PT", "P1DT", "P1Y", "P1M", "P1W", "PT1.0S", "PT0.001S", "+PT1S", "PT-1S", " PT1S", "PT1S\n", "pt1s", "PT1M1H", "PT1S1S", "P1D1D", "P1HT1S", "PT1S1D", "PT1ST1S", "P１D", "P1D\0", "PT1", "P1DTT1S"] {
        runtime.set_global("text", Value::String(text.into())).unwrap();
        failure(&mut runtime, "Duration::parse($text)", "DurationError");
    }
    for text in ["PT9223372036854775808S", "-PT9223372036854775809S", "P9223372036854775807D", "PT999999999999999999999999999999999999999999S"] {
        runtime.set_global("text", Value::String(text.into())).unwrap();
        failure(&mut runtime, "Duration::parse($text)", "RangeError");
    }
    let exact = format!("PT{}1S", "0".repeat(124));
    assert_eq!(exact.len(), 128);
    runtime.set_global("text", Value::String(exact.clone())).unwrap();
    assert_eq!(evaluate(&mut runtime, "Duration::parse($text)"), Value::Int(1));
    runtime.set_global("text", Value::String(format!("{exact}S"))).unwrap();
    failure(&mut runtime, "Duration::parse($text)", "ResourceLimitError");
}

#[test]
fn timestamp_ranges_are_checked_at_exact_boundaries() {
    let mut runtime = runtime();
    for bound in [chrono::DateTime::<chrono::Utc>::MIN_UTC.timestamp(), chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp()] {
        assert_eq!(evaluate(&mut runtime, &format!("Date::addSeconds({bound}, 0)")), Value::Int(bound));
        assert_eq!(evaluate(&mut runtime, &format!("Date::diff({bound}, {bound})")), Value::Int(0));
    }
    for (bound, step) in [(chrono::DateTime::<chrono::Utc>::MIN_UTC.timestamp(), -1), (chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp(), 1)] {
        failure(&mut runtime, &format!("Date::addSeconds({bound}, {step})"), "RangeError");
        failure(&mut runtime, &format!("Date::diff({}, 0)", bound + step), "RangeError");
        failure(&mut runtime, &format!("Date::diff(0, {})", bound + step), "RangeError");
    }
    for (bound, step) in [(jiff::Timestamp::MIN.as_second(), -1), (jiff::Timestamp::MAX.as_second(), 1)] {
        assert_eq!(evaluate(&mut runtime, &format!("Date::addCalendar({bound}, 0, \"day\")")), Value::Int(bound));
        failure(&mut runtime, &format!("Date::addCalendar({}, 0, \"day\")", bound + step), "RangeError");
        failure(&mut runtime, &format!("Date::addCalendar({bound}, {step}, \"day\")"), "RangeError");
    }
    for expression in ["Date::addSeconds(0, 9223372036854775807)", "Date::addSeconds(1, 9223372036854775807)", "Date::addCalendar(0, 9223372036854775807, \"year\")", "Date::addCalendar(0, -9223372036854775807 - 1, \"week\")"] {
        failure(&mut runtime, expression, "RangeError");
    }
}

#[test]
fn signatures_zones_and_errors_are_strict_and_keep_traces() {
    let mut runtime = runtime();
    for expression in ["Date::addCalendar(0, 1.0, \"day\")", "Date::addCalendar(0, 1, \"day\", null)", "Date::addCalendar(0, 1)", "Date::addSeconds(0, 1.0)", "Date::diff(0)", "Date::diff(0, 1, 2)", "Duration::parse(1)", "Duration::parse(\"PT1S\", [])"] {
        failure(&mut runtime, expression, "TypeError");
    }
    for zone in ["", "No/Such_Zone", "+05:30", "LOCAL", "bad\0zone", &"a".repeat(256)] {
        runtime.set_global("zone", Value::String(zone.into())).unwrap();
        failure(&mut runtime, "Date::addCalendar(0, 0, \"day\", $zone)", "DateError");
    }
    failure(&mut runtime, "Date::addCalendar(0, 1, \"second\")", "DateError");
    runtime.execute_source(r#"
        function helper() { return Duration::parse("PT0.1S"); }
        try { helper(); } catch ($error) {
            $kind = $error->kind;
        }
    "#,&mut Vec::new()).unwrap();
    assert_eq!(runtime.get_global("kind"), Some(&Value::String("DurationError".into())));
    let error = runtime.execute_source("helper();", &mut Vec::new()).unwrap_err();
    let diagnostic = format!("{error:?}");
    assert!(diagnostic.contains("helper"), "{diagnostic}");
    assert!(diagnostic.contains("Duration::parse"), "{diagnostic}");
}
