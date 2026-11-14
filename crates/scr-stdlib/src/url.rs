//! Explicit UTF-8 component, query, and form codecs; no network access.
use crate::bytes::{grow, payload, resource};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::mem::size_of;

fn encoding() -> NativeError {
    NativeError::new("EncodingError", "invalid percent escape or decoded UTF-8")
}
fn checkpoint(context: &RuntimeContext<'_>, index: usize) -> NativeResult<()> {
    if index % 1024 == 0 {
        context.check_cancelled()?;
    }
    Ok(())
}
fn safe(byte: u8, form: bool) -> bool {
    byte.is_ascii_alphanumeric() || b"-._".contains(&byte) || byte == if form { b'*' } else { b'~' }
}
fn encoded(
    context: &RuntimeContext<'_>,
    text: &str,
    form: bool,
    mut emit: impl FnMut(&[u8]) -> NativeResult<()>,
) -> NativeResult<()> {
    const HEX: &[u8] = b"0123456789ABCDEF";
    for (index, byte) in text.bytes().enumerate() {
        checkpoint(context, index)?;
        if safe(byte, form) {
            emit(&[byte])?;
        } else if form && byte == b' ' {
            emit(b"+")?;
        } else {
            emit(&[b'%', HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]])?;
        }
    }
    Ok(())
}
fn decoded(
    context: &RuntimeContext<'_>,
    text: &str,
    form: bool,
    mut emit: impl FnMut(u8) -> NativeResult<()>,
) -> NativeResult<()> {
    let input = text.as_bytes();
    let mut index = 0;
    let mut steps = 0;
    while index < input.len() {
        checkpoint(context, steps)?;
        steps += 1;
        let byte = input[index];
        if byte == b'%' {
            let hex = input.get(index + 1..index + 3).ok_or_else(encoding)?;
            let high = (hex[0] as char).to_digit(16).ok_or_else(encoding)?;
            let low = (hex[1] as char).to_digit(16).ok_or_else(encoding)?;
            emit((high * 16 + low) as u8)?;
            index += 3;
        } else {
            emit(if form && byte == b'+' { b' ' } else { byte })?;
            index += 1;
        }
    }
    Ok(())
}
fn decode(context: &RuntimeContext<'_>, text: &str, form: bool) -> NativeResult<String> {
    let mut total = size_of::<Value>();
    decoded(context, text, form, |_| grow(context, &mut total, 1))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total - size_of::<Value>())
        .map_err(|_| resource("decoded string allocation failed"))?;
    decoded(context, text, form, |byte| {
        bytes.push(byte);
        Ok(())
    })?;
    String::from_utf8(bytes).map_err(|_| encoding())
}
fn buffer(context: &RuntimeContext<'_>, bytes: usize) -> NativeResult<String> {
    payload(context, bytes)?;
    let mut result = String::new();
    result
        .try_reserve_exact(bytes)
        .map_err(|_| resource("encoded string allocation failed"))?;
    Ok(result)
}
fn append(
    context: &RuntimeContext<'_>,
    result: &mut String,
    text: &str,
    form: bool,
) -> NativeResult<()> {
    encoded(context, text, form, |bytes| {
        // The encoder emits only ASCII.
        for byte in bytes {
            result.push(char::from(*byte));
        }
        Ok(())
    })
}
fn pairs(
    context: &RuntimeContext<'_>,
    parameters: &Value,
    mut visit: impl FnMut(&str, &str) -> NativeResult<()>,
) -> NativeResult<()> {
    let invalid = || {
        NativeError::type_error(
            "parameters must be a map of string keys/values or a list of two-string pairs",
        )
    };
    match parameters {
        Value::Map(entries) => {
            for (key, value) in entries {
                context.check_cancelled()?;
                let (MapKey::String(key), Value::String(value)) = (key, value) else {
                    return Err(invalid());
                };
                visit(key, value)?;
            }
        }
        Value::List(entries) => {
            for entry in entries {
                context.check_cancelled()?;
                let Value::List(pair) = entry else {
                    return Err(invalid());
                };
                let [Value::String(key), Value::String(value)] = pair.as_slice() else {
                    return Err(invalid());
                };
                visit(key, value)?;
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
fn build(context: &RuntimeContext<'_>, parameters: &Value, form: bool) -> NativeResult<Value> {
    let mut total = size_of::<Value>();
    let mut first = true;
    pairs(context, parameters, |key, value| {
        grow(context, &mut total, if first { 1 } else { 2 })?; // '=' and optional '&'
        first = false;
        for text in [key, value] {
            encoded(context, text, form, |bytes| {
                grow(context, &mut total, bytes.len())
            })?;
        }
        Ok(())
    })?;
    let mut result = buffer(context, total - size_of::<Value>())?;
    let mut first = true;
    pairs(context, parameters, |key, value| {
        if !first {
            result.push('&');
        }
        first = false;
        append(context, &mut result, key, form)?;
        result.push('=');
        append(context, &mut result, value, form)
    })?;
    Ok(Value::String(result))
}
fn parse(context: &RuntimeContext<'_>, text: &str, form: bool) -> NativeResult<Value> {
    let text = if form {
        text
    } else {
        text.strip_prefix('?').unwrap_or(text)
    };
    let mut total = size_of::<Value>();
    let mut count = 0usize;
    for field in text.split('&') {
        context.check_cancelled()?;
        if field.is_empty() {
            continue;
        }
        let (key, value) = field.split_once('=').unwrap_or((field, ""));
        grow(context, &mut total, 3 * size_of::<Value>())?;
        count = count
            .checked_add(1)
            .ok_or_else(|| resource("query pair count overflow"))?;
        for field in [key, value] {
            decoded(context, field, form, |_| grow(context, &mut total, 1))?;
        }
    }
    crate::bytes::budget(context, total)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| resource("query allocation failed"))?;
    for field in text.split('&') {
        context.check_cancelled()?;
        if field.is_empty() {
            continue;
        }
        let (key, value) = field.split_once('=').unwrap_or((field, ""));
        let mut pair = Vec::new();
        pair.try_reserve_exact(2)
            .map_err(|_| resource("query pair allocation failed"))?;
        pair.push(Value::String(decode(context, key, form)?));
        pair.push(Value::String(decode(context, value, form)?));
        result.push(Value::List(pair));
    }
    Ok(Value::List(result))
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Any, List, String as Text};
    for (name, input, output) in [
        ("Url::encodeComponent", Text, Text),
        ("Url::decodeComponent", Text, Text),
        ("Url::buildQuery", Any, Text),
        ("Url::parseQuery", Text, List),
        ("Form::encode", Any, Text),
        ("Form::decode", Text, List),
    ] {
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(vec![input], output),
                move |context, args| invoke(name, context, args),
            ),
        )?;
    }
    Ok(())
}

fn invoke(name: &str, context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult<Value> {
    context.check_cancelled()?;
    let text = || match &args[0] {
        Value::String(s) => s.as_str(),
        _ => unreachable!(),
    };
    match name {
        "Url::encodeComponent" => {
            let mut total = size_of::<Value>();
            encoded(context, text(), false, |bytes| {
                grow(context, &mut total, bytes.len())
            })?;
            let mut result = buffer(context, total - size_of::<Value>())?;
            append(context, &mut result, text(), false)?;
            Ok(Value::String(result))
        }
        "Url::decodeComponent" => Ok(Value::String(decode(context, text(), false)?)),
        "Url::buildQuery" => build(context, &args[0], false),
        "Url::parseQuery" => parse(context, text(), false),
        "Form::encode" => build(context, &args[0], true),
        "Form::decode" => parse(context, text(), true),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn exact_output_budgets_and_cancellation_cover_all_codecs() {
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
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        };
        let text = |s: &str| Value::String(s.into());
        let slot = size_of::<Value>();
        for (name, argument, required) in [
            ("Url::encodeComponent", text("&é"), slot + 9),
            ("Url::decodeComponent", text("%C3%A9"), slot + 2),
            (
                "Url::buildQuery",
                Value::Map(vec![(MapKey::String("&".into()), text("&"))]),
                slot + 7,
            ),
            (
                "Form::encode",
                Value::List(vec![Value::List(vec![text("&"), text("&")])]),
                slot + 7,
            ),
            ("Url::parseQuery", text("a=b&a=c"), slot * 7 + 4),
            ("Form::decode", text("a=b&a=c"), slot * 7 + 4),
            ("Url::parseQuery", text("&&"), slot),
            ("Form::encode", Value::List(vec![]), slot),
        ] {
            context.max_value_bytes = None;
            let args = [argument];
            let expected = invoke(name, &context, &args).unwrap();
            context.max_value_bytes = Some(required);
            assert_eq!(invoke(name, &context, &args).unwrap(), expected, "{name}");
            context.max_value_bytes = Some(required - 1);
            assert_eq!(
                invoke(name, &context, &args).unwrap_err().kind,
                "ResourceLimitError",
                "{name}"
            );
            context.max_value_bytes = None;
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            assert_eq!(
                invoke(name, &context, &args).unwrap_err().kind,
                "Interrupted",
                "{name}"
            );
            context.cancellation = None;
        }
        // Long runs of separators and percent escapes remain interruptible.
        context.cancellation = Some(Arc::new(AtomicBool::new(true)));
        assert!(parse(&context, &"&".repeat(4096), false).is_err());
        assert!(decode(&context, &"%61".repeat(4096), false).is_err());
    }
}
