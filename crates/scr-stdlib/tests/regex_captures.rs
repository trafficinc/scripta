use scr_runtime::{MapKey, Runtime, RuntimeError, RuntimeOptions, Value};
use std::mem::size_of;

fn runtime(limit: Option<usize>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), RuntimeError> {
    runtime.execute_source(source, &mut Vec::new()).map(|_| ())
}
fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (MapKey::String(key.into()), value))
            .collect(),
    )
}
fn group(text: &str, start: i64, end: i64) -> Value {
    map(vec![
        ("text", Value::String(text.into())),
        ("start", Value::Int(start)),
        ("end", Value::Int(end)),
    ])
}
fn record(groups: Vec<Value>, named: Vec<(&str, Value)>) -> Value {
    map(vec![("groups", Value::list(groups)), ("named", map(named))])
}
fn evaluate(runtime: &mut Runtime, input: &str, pattern: &str, all: bool) -> Value {
    runtime
        .set_global("input", Value::String(input.into()))
        .unwrap();
    runtime
        .set_global("pattern", Value::String(pattern.into()))
        .unwrap();
    let method = if all { "capturesAll" } else { "captures" };
    run(
        runtime,
        &format!("$result=Regex::{method}($input,$pattern);"),
    )
    .unwrap();
    runtime.get_global("result").unwrap().clone()
}

#[test]
fn numbered_named_optional_and_empty_groups_have_explicit_records() {
    let mut runtime = runtime(None);
    assert_eq!(
        evaluate(
            &mut runtime,
            "prefix abc suffix",
            r"(?P<word>a(b)c)(z)?()",
            false
        ),
        record(
            vec![
                group("abc", 7, 10),
                group("abc", 7, 10),
                group("b", 8, 9),
                Value::Null,
                group("", 10, 10)
            ],
            vec![("word", group("abc", 7, 10))],
        )
    );
    assert_eq!(
        evaluate(&mut runtime, "b", r"(?P<absent>a)?(?P<empty>)(b)", false),
        record(
            vec![
                group("b", 0, 1),
                Value::Null,
                group("", 0, 0),
                group("b", 0, 1)
            ],
            vec![("absent", Value::Null), ("empty", group("", 0, 0))],
        )
    );
    assert_eq!(
        evaluate(&mut runtime, "ab a", r"(?P<letter>a)(b)?", true),
        Value::list(vec![
            record(
                vec![group("ab", 0, 2), group("a", 0, 1), group("b", 1, 2)],
                vec![("letter", group("a", 0, 1))]
            ),
            record(
                vec![group("a", 3, 4), group("a", 3, 4), Value::Null],
                vec![("letter", group("a", 3, 4))]
            ),
        ])
    );
    assert_eq!(
        evaluate(&mut runtime, "aaa", r"(a)+", false),
        record(vec![group("aaa", 0, 3), group("a", 2, 3)], vec![])
    );
    assert_eq!(
        evaluate(&mut runtime, "é", r"(?P<名>é)", false),
        record(
            vec![group("é", 0, 1), group("é", 0, 1)],
            vec![("名", group("é", 0, 1))]
        )
    );
}

#[test]
fn scalar_offsets_cover_multibyte_combining_emoji_nested_and_repeated_matches() {
    let mut runtime = runtime(None);
    let input = "👍🏽 e\u{301}東京 é";
    assert_eq!(
        evaluate(&mut runtime, input, r"(?P<outer>e(\p{M})東京)", false),
        record(
            vec![
                group("e\u{301}東京", 3, 7),
                group("e\u{301}東京", 3, 7),
                group("\u{301}", 4, 5)
            ],
            vec![("outer", group("e\u{301}東京", 3, 7))],
        )
    );
    let matches = evaluate(&mut runtime, input, r"(\p{L})", true);
    assert_eq!(
        matches,
        Value::list(vec![
            record(vec![group("e", 3, 4), group("e", 3, 4)], vec![]),
            record(vec![group("東", 5, 6), group("東", 5, 6)], vec![]),
            record(vec![group("京", 6, 7), group("京", 6, 7)], vec![]),
            record(vec![group("é", 8, 9), group("é", 8, 9)], vec![]),
        ])
    );
}

#[test]
fn zero_width_matches_follow_existing_find_all_and_no_match_is_null_or_empty_list() {
    let mut runtime = runtime(None);
    for (input, pattern, positions) in [
        ("é👍🏽", "", vec![0, 1, 2, 3]),
        ("é", "(?P<empty>)", vec![0, 1]),
        ("", "()", vec![0]),
        ("x\r\né", "(?m)^", vec![0, 3]),
    ] {
        let Value::List(matches) = evaluate(&mut runtime, input, pattern, true) else {
            panic!()
        };
        assert_eq!(matches.len(), positions.len());
        for (matched, index) in matches.iter().zip(positions) {
            let Value::Map(entries) = matched else {
                panic!()
            };
            let Value::List(groups) = &entries[0].1 else {
                panic!()
            };
            assert_eq!(groups[0], group("", index, index));
        }
    }
    for pattern in ["z", "(?P<missing>z)"] {
        assert_eq!(evaluate(&mut runtime, "abc", pattern, false), Value::Null);
        assert_eq!(
            evaluate(&mut runtime, "abc", pattern, true),
            Value::list(vec![])
        );
    }
    for pattern in ["", "a*", "a?", "^|$", "(?:a|)"] {
        runtime
            .set_global("pattern", Value::String(pattern.into()))
            .unwrap();
        run(
            &mut runtime,
            "$old=Regex::findAll('aé',$pattern); $new=Regex::capturesAll('aé',$pattern);",
        )
        .unwrap();
        let Value::List(old) = runtime.get_global("old").unwrap() else {
            panic!()
        };
        let Value::List(new) = runtime.get_global("new").unwrap() else {
            panic!()
        };
        let texts: Vec<Value> = new
            .iter()
            .map(|value| {
                let Value::Map(entries) = value else { panic!() };
                let Value::List(groups) = &entries[0].1 else {
                    panic!()
                };
                let Value::Map(group) = &groups[0] else {
                    panic!()
                };
                group[0].1.clone()
            })
            .collect();
        assert_eq!(old, &texts, "{pattern}");
    }
}

#[test]
fn exact_nested_budgets_include_named_duplicates_keys_slots_and_nulls() {
    let single = 16 * size_of::<Value>() + 12 * MapKey::LOGICAL_BYTES + 54;
    for (all, input, required) in [
        (false, "é", single),
        (true, "é é", size_of::<Value>() + single * 2),
    ] {
        for limit in [required - 1, required] {
            let mut runtime = runtime(Some(limit));
            runtime
                .set_global("input", Value::String(input.into()))
                .unwrap();
            run(
                &mut runtime,
                "$result='unchanged'; Regex::isMatch('é','(?P<n>é)(z)?');",
            )
            .unwrap();
            let method = if all { "capturesAll" } else { "captures" };
            let result = run(
                &mut runtime,
                &format!("$result=Regex::{method}($input,'(?P<n>é)(z)?');"),
            );
            if limit == required {
                result.unwrap();
                // Host-boundary measurement independently agrees with native preflight.
                runtime
                    .set_global("copy", runtime.get_global("result").unwrap().clone())
                    .unwrap();
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .message
                        .starts_with("ResourceLimitError:")
                );
                assert_eq!(
                    runtime.get_global("result"),
                    Some(&Value::String("unchanged".into()))
                );
            }
        }
    }
    let mut runtime = runtime(Some(size_of::<Value>() + 1));
    assert_eq!(evaluate(&mut runtime, "x", "z", false), Value::Null);
    assert_eq!(evaluate(&mut runtime, "x", "z", true), Value::list(vec![]));
}

#[test]
fn first_capture_stays_bounded_while_nested_and_repeated_output_can_exceed_budget() {
    let single = 7 * size_of::<Value>() + 5 * MapKey::LOGICAL_BYTES + 24;
    let mut first = runtime(Some(single));
    first
        .set_global("input", Value::String("x ".repeat(16)))
        .unwrap();
    run(
        &mut first,
        "$first=Regex::captures($input,'x'); $result='unchanged';",
    )
    .unwrap();
    assert_eq!(
        first.get_global("first"),
        Some(&record(vec![group("x", 0, 1)], vec![]))
    );
    assert!(
        run(&mut first, "$result=Regex::capturesAll($input,'x');")
            .unwrap_err()
            .message
            .starts_with("ResourceLimitError:")
    );
    assert_eq!(
        first.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
    let mut nested = runtime(Some(8192));
    nested
        .set_global("input", Value::String("é".repeat(1024)))
        .unwrap();
    nested
        .set_global(
            "pattern",
            Value::String(format!("{}é+{}", "(".repeat(50), ")".repeat(50))),
        )
        .unwrap();
    run(&mut nested, "$result='unchanged';").unwrap();
    for method in ["captures", "capturesAll"] {
        assert!(
            run(
                &mut nested,
                &format!("$result=Regex::{method}($input,$pattern);")
            )
            .unwrap_err()
            .message
            .starts_with("ResourceLimitError:")
        );
        assert_eq!(
            nested.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
}

#[test]
fn malformed_patterns_strict_signatures_and_errors_preserve_assignment_and_stack() {
    let mut runtime = runtime(None);
    run(&mut runtime, "$result='unchanged';").unwrap();
    for method in ["captures", "capturesAll"] {
        for pattern in ["[", "(?=x)", "(?P<x>a)(?P<x>b)"] {
            runtime
                .set_global("pattern", Value::String(pattern.into()))
                .unwrap();
            let error = run(
                &mut runtime,
                &format!("$result=Regex::{method}('x',$pattern);"),
            )
            .unwrap_err();
            assert!(error.message.starts_with("RegexError:"));
            assert_eq!(
                runtime.get_global("result"),
                Some(&Value::String("unchanged".into()))
            );
        }
        for args in ["1,'x'", "'x',null", "'x'", "'x','x','extra'"] {
            assert!(
                run(&mut runtime, &format!("$result=Regex::{method}({args});"))
                    .unwrap_err()
                    .message
                    .starts_with("TypeError:")
            );
            assert_eq!(
                runtime.get_global("result"),
                Some(&Value::String("unchanged".into()))
            );
        }
    }
    runtime
        .set_global("pattern", Value::String("a".repeat(65537)))
        .unwrap();
    for method in ["captures", "capturesAll"] {
        assert!(
            run(
                &mut runtime,
                &format!("$result=Regex::{method}('x',$pattern);")
            )
            .unwrap_err()
            .message
            .starts_with("RegexError:")
        );
    }
    let source =
        "function invalidCapture() { return Regex::captures('x','['); } $result=invalidCapture();";
    let error = run(&mut runtime, source).unwrap_err();
    assert_eq!(error.span.start, source.find("Regex::captures").unwrap());
    assert!(
        error
            .trace
            .iter()
            .any(|frame| frame.function == "invalidCapture")
    );
    assert!(
        error
            .trace
            .iter()
            .any(|frame| frame.function == "Regex::captures")
    );
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("unchanged".into()))
    );
}

#[test]
fn cancellation_works_with_warm_cache_and_preserves_destinations() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let flag = Arc::new(AtomicBool::new(false));
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(flag.clone()),
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    run(
        &mut runtime,
        "Regex::isMatch('é','(?P<n>é)'); $result='unchanged';",
    )
    .unwrap();
    for method in ["captures", "capturesAll"] {
        flag.store(true, Ordering::Relaxed);
        let error = run(
            &mut runtime,
            &format!("$result=Regex::{method}('é','(?P<n>é)');"),
        )
        .unwrap_err();
        assert_eq!(error.message, "execution interrupted");
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
        flag.store(false, Ordering::Relaxed);
        run(&mut runtime, &format!("Regex::{method}('é','(?P<n>é)');")).unwrap();
    }
}
