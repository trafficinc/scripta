use super::*;

fn owned_reference_raw(raw: &RawValue, depth: usize) -> NativeResult<Value> {
    if depth > 128 {
        return Err(error("JSON decoding exceeds 128 nesting levels"));
    }
    let text = raw.get();
    Ok(match text.as_bytes()[0] {
        b'n' => Value::Null,
        b't' => Value::Bool(true),
        b'f' => Value::Bool(false),
        b'"' => Value::String(serde_json::from_str(text).map_err(syntax_error)?),
        b'[' => {
            let items: Vec<Box<RawValue>> = serde_json::from_str(text).map_err(syntax_error)?;
            Value::List(
                items
                    .iter()
                    .map(|v| owned_reference_raw(v, depth + 1))
                    .collect::<NativeResult<_>>()?,
            )
        }
        b'{' => {
            let items: BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(text).map_err(syntax_error)?;
            Value::Map(
                items
                    .into_iter()
                    .map(|(k, v)| {
                        Ok((
                            MapKey::String(k.into()),
                            owned_reference_raw(&v, depth + 1)?,
                        ))
                    })
                    .collect::<NativeResult<_>>()?,
            )
        }
        _ if !text.contains(['.', 'e', 'E']) => Value::Int(
            text.parse()
                .map_err(|_| error("JSON integer is outside i64 range"))?,
        ),
        _ => {
            let value: f64 = text.parse().map_err(|_| error("invalid JSON number"))?;
            if !value.is_finite() {
                return Err(error("JSON number exceeds finite f64 range"));
            }
            Value::Float(value)
        }
    })
}
fn owned_reference_decode(value: &str) -> NativeResult {
    // RawValue validates syntax while retaining exact numeric tokens. Decoding
    // objects separately also preserves keys used internally by serde_json.
    let raw: Box<RawValue> = serde_json::from_str(value).map_err(syntax_error)?;
    owned_reference_raw(&raw, 0)
}

#[test]
fn borrowed_decoding_matches_owned_tokens_keys_duplicates_and_diagnostics() {
    for text in [
        "null",
        "true",
        "false",
        "0",
        "-0",
        "-0.0",
        "1.0",
        "1e2",
        "-9223372036854775808",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775809",
        "18446744073709551616",
        "1e999",
        "[]",
        "{}",
        r#"[1,2.5,"世界",false,null]"#,
        r#"{"z":1,"a":2,"é":3,"":null}"#,
        r#"{"a":1e999,"a":2}"#,
        r#"{"a":2,"a":1e999}"#,
        r#"{"z":9223372036854775808,"a":1e999}"#,
        r#"{"a\u0062":1,"ab":2,"\uD83D\uDE00":"\n\t\u0000"}"#,
        r#"{"$serde_json::private::Number":"123","$serde_json::private::RawValue":2}"#,
        "",
        " ",
        "NaN",
        "[1,]",
        "{\r\n bad}",
        "{\n bad}",
        "true false",
        r#"{"a":"\uD800"}"#,
        r#"{"a":NaN,"a":2}"#,
    ] {
        assert_eq!(decode(text), owned_reference_decode(text), "{text:?}");
    }
    for depth in [63, 64, 65, 127, 128, 129] {
        for (open, close) in [("[", "]"), ("{\"key\":", "}")] {
            let text = format!("{}0{}", open.repeat(depth), close.repeat(depth));
            assert_eq!(
                decode(&text),
                owned_reference_decode(&text),
                "depth {depth}"
            );
        }
    }
}

#[test]
fn decoded_keys_are_sorted_owned_and_keep_last_unescaped_duplicate() {
    let mut text = String::from(r#"{"z":9,"\u0061":1,"a":2,"世界":"value","":null}"#);
    let decoded = decode(&text).unwrap();
    text.clear();
    text.push_str("overwritten input");
    let Value::Map(entries) = &decoded else {
        panic!()
    };
    let keys: Vec<_> = entries
        .iter()
        .map(|(key, _)| match key {
            MapKey::String(text) => text.as_str(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(keys, ["", "a", "z", "世界"]);
    assert_eq!(entries[1].1, Value::Int(2));
    assert_eq!(entries[3].1, Value::String("value".into()));
    assert_eq!(
        encode(&decoded, false).unwrap(),
        r#"{"":null,"a":2,"z":9,"世界":"value"}"#
    );
}

#[test]
fn nonfinite_and_out_of_range_numbers_are_explicit_errors() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(encode(&Value::Float(value), false).is_err());
    }
    for text in ["18446744073709551616", "9223372036854775808", "1e999"] {
        assert!(decode(text).is_err());
    }
}
