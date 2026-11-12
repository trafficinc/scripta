//! Literal Unicode text helpers with preflight logical output budgets.
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::mem::size_of;

fn resource(message: &str) -> NativeError {
    NativeError::new("ResourceLimitError", message)
}

fn size(context: &RuntimeContext<'_>, bytes: Option<usize>) -> NativeResult<usize> {
    let bytes = bytes.ok_or_else(|| resource("string result size overflow"))?;
    if context.max_value_bytes.is_some_and(|limit| bytes > limit) {
        return Err(resource("string result exceeds execution value byte limit"));
    }
    Ok(bytes)
}

fn text_size(context: &RuntimeContext<'_>, bytes: usize) -> NativeResult<()> {
    size(context, bytes.checked_add(size_of::<Value>())).map(|_| ())
}

fn buffer(context: &RuntimeContext<'_>, bytes: usize) -> NativeResult<String> {
    text_size(context, bytes)?;
    let mut result = String::new();
    result
        .try_reserve_exact(bytes)
        .map_err(|_| resource("string result allocation failed"))?;
    Ok(result)
}

fn owned(context: &RuntimeContext<'_>, text: &str) -> NativeResult<Value> {
    let mut result = buffer(context, text.len())?;
    result.push_str(text);
    Ok(Value::String(result))
}

// Native signatures validate the types before entering these callbacks.
fn text(value: &Value) -> &str {
    let Value::String(text) = value else {
        unreachable!("validated string parameter")
    };
    text
}

fn range(message: &str) -> NativeError {
    NativeError::new("RangeError", message)
}

fn nonnegative(value: &Value) -> NativeResult<usize> {
    let Value::Int(value) = value else {
        unreachable!("validated integer parameter")
    };
    usize::try_from(*value).map_err(|_| range("substring offsets must be nonnegative ints"))
}

fn substring(context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult {
    let input = text(&args[0]);
    let count = input.chars().count();
    let start = nonnegative(&args[1])?;
    if start > count {
        return Err(range("substring start is out of bounds"));
    }
    let length = args
        .get(2)
        .map(nonnegative)
        .transpose()?
        .unwrap_or(count - start);
    if length > count - start {
        return Err(range("substring length is out of bounds"));
    }
    let byte_offset = |position| {
        input
            .char_indices()
            .nth(position)
            .map_or(input.len(), |(i, _)| i)
    };
    owned(
        context,
        &input[byte_offset(start)..byte_offset(start + length)],
    )
}

fn split(context: &RuntimeContext<'_>, input: &str, separator: &str) -> NativeResult {
    if separator.is_empty() {
        return Err(range("split separator must not be empty"));
    }
    let mut bytes = size(context, Some(size_of::<Value>()))?;
    let mut fields = Vec::new();
    for field in input.split(separator) {
        context.check_cancelled()?;
        bytes = size(
            context,
            bytes
                .checked_add(size_of::<Value>())
                .and_then(|n| n.checked_add(field.len())),
        )?;
        fields
            .try_reserve(1)
            .map_err(|_| resource("split result allocation failed"))?;
        fields.push(owned(context, field)?);
    }
    Ok(Value::List(fields))
}

fn join(context: &RuntimeContext<'_>, values: &Value, separator: &str) -> NativeResult {
    let Value::List(values) = values else {
        unreachable!("validated list parameter")
    };
    let mut bytes = 0usize;
    for (index, value) in values.iter().enumerate() {
        context.check_cancelled()?;
        let Value::String(value) = value else {
            return Err(NativeError::type_error(
                "String::join requires a list of strings",
            ));
        };
        bytes = bytes
            .checked_add(value.len())
            .and_then(|n| n.checked_add(if index == 0 { 0 } else { separator.len() }))
            .ok_or_else(|| resource("join result size overflow"))?;
        text_size(context, bytes)?;
    }
    let mut result = buffer(context, bytes)?;
    for (index, value) in values.iter().enumerate() {
        context.check_cancelled()?;
        if index != 0 {
            result.push_str(separator);
        }
        result.push_str(text(value));
    }
    Ok(Value::String(result))
}

fn replace(
    context: &RuntimeContext<'_>,
    input: &str,
    search: &str,
    replacement: &str,
) -> NativeResult {
    if search.is_empty() {
        return Err(range("replacement search must not be empty"));
    }
    let mut bytes = 0usize;
    let mut previous = 0;
    for (index, _) in input.match_indices(search) {
        context.check_cancelled()?;
        bytes = bytes
            .checked_add(index - previous)
            .and_then(|n| n.checked_add(replacement.len()))
            .ok_or_else(|| resource("replacement result size overflow"))?;
        text_size(context, bytes)?;
        previous = index + search.len();
    }
    bytes = bytes
        .checked_add(input.len() - previous)
        .ok_or_else(|| resource("replacement result size overflow"))?;
    let mut result = buffer(context, bytes)?;
    previous = 0;
    for (index, _) in input.match_indices(search) {
        context.check_cancelled()?;
        result.push_str(&input[previous..index]);
        result.push_str(replacement);
        previous = index + search.len();
    }
    result.push_str(&input[previous..]);
    Ok(Value::String(result))
}

fn case(context: &RuntimeContext<'_>, input: &str, upper: bool) -> NativeResult {
    let mut bytes = 0usize;
    for (index, ch) in input.chars().enumerate() {
        if index % 4096 == 0 {
            context.check_cancelled()?;
        }
        let mapped_bytes: usize = if upper {
            ch.to_uppercase().map(char::len_utf8).sum()
        } else {
            ch.to_lowercase().map(char::len_utf8).sum()
        };
        bytes = bytes
            .checked_add(mapped_bytes)
            .ok_or_else(|| resource("case conversion result size overflow"))?;
        text_size(context, bytes)?;
    }
    text_size(context, bytes)?;
    // Rust's whole-string lowercase also handles contextual Greek final sigma.
    // Its contextual alternatives have the same UTF-8 size as the scalar mapping.
    Ok(Value::String(if upper {
        input.to_uppercase()
    } else {
        input.to_lowercase()
    }))
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Bool, Int, List, String as Text};
    for (name, parameters, returns) in [
        ("trim", vec![Text], Text),
        ("split", vec![Text, Text], List),
        ("join", vec![List, Text], Text),
        ("contains", vec![Text, Text], Bool),
        ("startsWith", vec![Text, Text], Bool),
        ("endsWith", vec![Text, Text], Bool),
        ("replace", vec![Text, Text, Text], Text),
        ("substring", vec![Text, Int], Text),
        ("lower", vec![Text], Text),
        ("upper", vec![Text], Text),
    ] {
        let mut signature = NativeSignature::new(parameters, returns);
        if name == "substring" {
            signature = signature.with_optional(vec![Int]);
        }
        runtime.register(
            format!("String::{name}"),
            NativeFunction::new(signature, move |context, args| match name {
                "trim" => owned(context, text(&args[0]).trim()),
                "split" => split(context, text(&args[0]), text(&args[1])),
                "join" => join(context, &args[0], text(&args[1])),
                "contains" => Ok(Value::Bool(text(&args[0]).contains(text(&args[1])))),
                "startsWith" => Ok(Value::Bool(text(&args[0]).starts_with(text(&args[1])))),
                "endsWith" => Ok(Value::Bool(text(&args[0]).ends_with(text(&args[1])))),
                "replace" => replace(context, text(&args[0]), text(&args[1]), text(&args[2])),
                "substring" => substring(context, args),
                "lower" => case(context, text(&args[0]), false),
                "upper" => case(context, text(&args[0]), true),
                _ => unreachable!("registered string helper"),
            }),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;

    fn with_context<T>(limit: Option<usize>, test: impl FnOnce(&mut RuntimeContext<'_>) -> T) -> T {
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
            max_value_bytes: limit,
        };
        test(&mut context)
    }

    #[test]
    fn builders_enforce_exact_output_budgets_before_reserving_results() {
        let slot = size_of::<Value>();
        type Operation = Box<dyn Fn(&RuntimeContext<'_>) -> NativeResult>;
        let cases: Vec<(usize, Operation)> = vec![
            (slot + 2, Box::new(|c| owned(c, "é"))),
            (slot, Box::new(|c| owned(c, ""))),
            (4 * slot + 2, Box::new(|c| split(c, "a,,b", ","))),
            (2 * slot, Box::new(|c| split(c, "", ","))),
            (slot, Box::new(|c| join(c, &Value::List(vec![]), ","))),
            (
                slot + 6,
                Box::new(|c| {
                    join(
                        c,
                        &Value::List(vec![Value::String("a".into()), Value::String("b".into())]),
                        "🦀",
                    )
                }),
            ),
            (slot + 12, Box::new(|c| replace(c, "aaa", "a", "🦀"))),
            (slot + 3, Box::new(|c| replace(c, "abc", "missing", "x"))),
            (slot, Box::new(|c| replace(c, "aaa", "a", ""))),
            (slot + 6, Box::new(|c| case(c, "ΐ", true))),
            (slot + 3, Box::new(|c| case(c, "İ", false))),
            (slot + 4, Box::new(|c| case(c, "ΟΣ", false))),
            (slot, Box::new(|c| case(c, "", false))),
            (
                slot + 4,
                Box::new(|c| {
                    substring(
                        c,
                        &[Value::String("🦀x".into()), Value::Int(0), Value::Int(1)],
                    )
                }),
            ),
        ];
        for (required, operation) in cases {
            let expected = with_context(None, |c| operation(c)).unwrap();
            assert_eq!(
                with_context(Some(required), |c| operation(c)).unwrap(),
                expected
            );
            let error = with_context(Some(required - 1), |c| operation(c)).unwrap_err();
            assert_eq!(error.kind, "ResourceLimitError");
            assert!(error.message.contains("byte limit"));
        }
    }

    #[test]
    fn size_overflow_and_cancelled_builds_are_explicit_errors() {
        use std::sync::{Arc, atomic::AtomicBool};
        with_context(None, |context| {
            assert_eq!(
                buffer(context, usize::MAX).unwrap_err().kind,
                "ResourceLimitError"
            );
            assert_eq!(size(context, None).unwrap_err().kind, "ResourceLimitError");
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            for result in [
                split(context, "a,b", ","),
                join(context, &Value::List(vec![Value::String("a".into())]), ","),
                replace(context, "aaa", "a", "b"),
                case(context, "abc", false),
                case(context, "abc", true),
            ] {
                assert_eq!(result.unwrap_err().kind, "Interrupted");
            }
        });
    }
}
