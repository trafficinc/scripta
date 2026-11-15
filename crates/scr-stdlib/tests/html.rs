use scr_runtime::{Runtime, RuntimeOptions, Value};
use std::{
    mem::size_of,
    sync::{Arc, atomic::AtomicBool},
};

fn runtime(options: RuntimeOptions) -> Runtime {
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}

#[test]
fn escapes_html_metacharacters_and_preserves_unicode_and_existing_input() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut runtime = runtime(options);
    for (input, expected) in [
        ("", ""),
        ("世界 café 👩‍💻 e\u{301}", "世界 café 👩‍💻 e\u{301}"),
        ("a&b<c>d\"e'f", "a&amp;b&lt;c&gt;d&quot;e&#039;f"),
        ("&amp; &#039; &#x27;", "&amp;amp; &amp;#039; &amp;#x27;"),
        (
            "\"><script>alert('x')</script>",
            "&quot;&gt;&lt;script&gt;alert(&#039;x&#039;)&lt;/script&gt;",
        ),
        ("\0\t\n\r", "\0\t\n\r"),
    ] {
        runtime
            .set_global("input", Value::String(input.into()))
            .unwrap();
        runtime
            .execute_source("<?scripta $result = Html::escape($input);", &mut Vec::new())
            .unwrap();
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String(expected.into()))
        );
        assert_eq!(
            runtime.get_global("input"),
            Some(&Value::String(input.into()))
        );
    }
}

#[test]
fn invalid_signatures_preserve_assignment_and_report_native_call_locations() {
    for call in [
        "Html::escape()",
        "Html::escape('a', 'b')",
        "Html::escape(null)",
        "Html::escape(1)",
        "Html::escape(true)",
        "Html::escape(1.0)",
        "Html::escape([])",
        "Html::escape(Bytes::encodeUtf8('a'))",
    ] {
        let mut runtime = runtime(RuntimeOptions::default());
        runtime.set_global("saved", Value::Int(42)).unwrap();
        let source = format!("<?scripta function render() {{ return {call}; }} $saved = render();");
        let error = runtime
            .execute_source(&source, &mut Vec::new())
            .unwrap_err();
        assert!(error.message.starts_with("TypeError:"), "{call}: {error}");
        assert_eq!(&source[error.span.start..error.span.end], call);
        assert!(error.trace.iter().any(|frame| frame.function == "render"));
        assert_eq!(runtime.get_global("saved"), Some(&Value::Int(42)));
    }
}

#[test]
fn exact_expansion_and_empty_result_budgets_are_enforced_without_partial_assignment() {
    for (input, expected) in [
        ("", ""),
        ("&é'", "&amp;é&#039;"),
        ("<>&\"'", "&lt;&gt;&amp;&quot;&#039;"),
    ] {
        let required = size_of::<Value>() + expected.len();
        for (limit, succeeds) in [(required, true), (required - 1, false)] {
            let mut runtime = runtime(RuntimeOptions {
                max_value_bytes: Some(limit),
                ..Default::default()
            });
            // The input fits even when the expanded output is one byte too large.
            if input.len() + size_of::<Value>() > limit {
                let error = runtime
                    .execute_source("<?scripta $result=Html::escape('');", &mut Vec::new())
                    .unwrap_err();
                assert!(error.message.contains("value byte limit"));
                assert!(runtime.get_global("result").is_none());
                continue;
            }
            runtime
                .set_global("input", Value::String(input.into()))
                .unwrap();
            runtime.set_global("saved", Value::Null).unwrap();
            let result =
                runtime.execute_source("<?scripta $saved = Html::escape($input);", &mut Vec::new());
            if succeeds {
                result.unwrap();
                assert_eq!(
                    runtime.get_global("saved"),
                    Some(&Value::String(expected.into()))
                );
            } else {
                assert!(result.unwrap_err().message.contains("value byte limit"));
                assert_eq!(runtime.get_global("saved"), Some(&Value::Null));
            }
        }
    }
}

#[test]
fn cancellation_cannot_be_swallowed_and_preserves_the_destination() {
    let mut runtime = runtime(RuntimeOptions {
        interrupt_flag: Some(Arc::new(AtomicBool::new(true))),
        ..Default::default()
    });
    runtime.set_global("saved", Value::Int(42)).unwrap();
    let error = runtime
        .execute_source(
            "<?scripta try {$saved=Html::escape('<');} catch(Error $e) {$saved=0;}",
            &mut Vec::new(),
        )
        .unwrap_err();
    assert!(error.message.contains("execution interrupted"));
    assert_eq!(runtime.get_global("saved"), Some(&Value::Int(42)));
}
