//! Explicit JSON conversion; object keys never silently change type.
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use serde_json::{Number, Value as Json, value::RawValue};
use std::collections::BTreeMap;
fn error(message: impl Into<String>) -> NativeError {
    NativeError::new("JsonError", message)
}
fn convert(value: &Value, depth: usize) -> NativeResult<Json> {
    if depth > 64 {
        return Err(error("JSON encoding exceeds 64 nesting levels"));
    }
    Ok(match value {
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Int(value) => Json::Number((*value).into()),
        Value::Float(value) => Json::Number(
            Number::from_f64(*value)
                .ok_or_else(|| error("NaN and infinity are not JSON numbers"))?,
        ),
        Value::String(value) => Json::String(value.clone()),
        Value::List(values) => Json::Array(
            values
                .iter()
                .map(|v| convert(v, depth + 1))
                .collect::<NativeResult<_>>()?,
        ),
        Value::Map(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries {
                let MapKey::String(key) = key else {
                    return Err(error(
                        "JSON object keys must be strings; convert integer keys explicitly",
                    ));
                };
                if object.contains_key(key) {
                    return Err(error("duplicate JSON object key"));
                }
                object.insert(key.clone(), convert(value, depth + 1)?);
            }
            Json::Object(object)
        }
        Value::Bytes(_) | Value::NativeObject(_) | Value::Callable(_) => {
            return Err(error(
                "bytes, native objects and callables require explicit conversion before JSON encoding",
            ));
        }
    })
}
fn syntax_error(error: serde_json::Error) -> NativeError {
    self::error(format!(
        "invalid JSON at line {}, column {}: {error}",
        error.line(),
        error.column()
    ))
}
fn from_raw(raw: &RawValue, depth: usize) -> NativeResult<Value> {
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
                    .map(|v| from_raw(v, depth + 1))
                    .collect::<NativeResult<_>>()?,
            )
        }
        b'{' => {
            let items: BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(text).map_err(syntax_error)?;
            Value::Map(
                items
                    .into_iter()
                    .map(|(k, v)| Ok((MapKey::String(k), from_raw(&v, depth + 1)?)))
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
pub(crate) fn decode(value: &str) -> NativeResult {
    // RawValue validates syntax while retaining exact numeric tokens. Decoding
    // objects separately also preserves keys used internally by serde_json.
    let raw: Box<RawValue> = serde_json::from_str(value).map_err(syntax_error)?;
    from_raw(&raw, 0)
}
pub(crate) fn encode(value: &Value, pretty: bool) -> NativeResult<String> {
    let json = convert(value, 0)?;
    (if pretty {
        serde_json::to_string_pretty(&json)
    } else {
        serde_json::to_string(&json)
    })
    .map_err(|e| error(e.to_string()))
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    for name in ["encode", "pretty", "decode"] {
        runtime.register(
            format!("Json::{name}"),
            NativeFunction::new(
                NativeSignature::new(
                    vec![if name == "decode" {
                        NativeType::String
                    } else {
                        NativeType::Any
                    }],
                    if name == "decode" {
                        NativeType::Any
                    } else {
                        NativeType::String
                    },
                ),
                move |_, args| {
                    if name == "decode" {
                        let Value::String(value) = &args[0] else {
                            unreachable!()
                        };
                        decode(value)
                    } else {
                        encode(&args[0], name == "pretty").map(Value::String)
                    }
                },
            ),
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonfinite_and_out_of_range_numbers_are_explicit_errors() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(encode(&Value::Float(value), false).is_err());
        }
        for text in ["18446744073709551616", "9223372036854775808", "1e999"] {
            assert!(decode(text).is_err());
        }
    }
}
