//! Small regex surface and explicit grapheme operations.
use crate::bytes::{budget, grow, payload, range, resource};
use regex::RegexBuilder;
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use std::mem::size_of;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Bool, Int, List, String as Text};
    for (name, returns) in [("isMatch", Bool), ("findAll", List), ("replace", Text)] {
        let mut parameters = vec![Text, Text];
        if name == "replace" {
            parameters.push(Text);
        }
        runtime.register(
            format!("Regex::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| {
                    let Value::String(input) = &args[0] else {
                        unreachable!()
                    };
                    let Value::String(pattern) = &args[1] else {
                        unreachable!()
                    };
                    if pattern.len() > 65536 {
                        return Err(NativeError::new(
                            "RegexError",
                            "pattern exceeds 65536 bytes",
                        ));
                    }
                    context.check_cancelled()?;
                    let regex = RegexBuilder::new(pattern)
                        .size_limit(1024 * 1024)
                        .dfa_size_limit(1024 * 1024)
                        .build()
                        .map_err(|e| NativeError::new("RegexError", e.to_string()))?;
                    if name == "isMatch" {
                        return Ok(Value::Bool(regex.is_match(input)));
                    }
                    let mut size = size_of::<Value>();
                    budget(context, size)?;
                    if name == "findAll" {
                        let mut values = Vec::new();
                        for found in regex.find_iter(input) {
                            context.check_cancelled()?;
                            grow(context, &mut size, size_of::<Value>() + found.len())?;
                            values
                                .try_reserve(1)
                                .map_err(|_| resource("match allocation failed"))?;
                            values.push(Value::String(found.as_str().into()));
                        }
                        return Ok(Value::List(values));
                    }
                    let Value::String(replacement) = &args[2] else {
                        unreachable!()
                    };
                    // Literal replacement deliberately avoids exposing captures in this API.
                    let mut result = String::new();
                    let mut end = 0;
                    for found in regex.find_iter(input) {
                        context.check_cancelled()?;
                        let prefix = &input[end..found.start()];
                        grow(context, &mut size, prefix.len())?;
                        grow(context, &mut size, replacement.len())?;
                        result
                            .try_reserve(prefix.len() + replacement.len())
                            .map_err(|_| resource("replacement allocation failed"))?;
                        result.push_str(prefix);
                        result.push_str(replacement);
                        end = found.end();
                    }
                    grow(context, &mut size, input.len() - end)?;
                    result
                        .try_reserve(input.len() - end)
                        .map_err(|_| resource("replacement allocation failed"))?;
                    result.push_str(&input[end..]);
                    Ok(Value::String(result))
                },
            ),
        )?;
    }
    runtime.register(
        "String::graphemeLength",
        NativeFunction::new(NativeSignature::new(vec![Text], Int), |context, args| {
            let Value::String(input) = &args[0] else {
                unreachable!()
            };
            let mut count = 0i64;
            for _ in input.graphemes(true) {
                context.check_cancelled()?;
                count = count
                    .checked_add(1)
                    .ok_or_else(|| NativeError::new("RangeError", "length exceeds i64"))?;
            }
            Ok(Value::Int(count))
        }),
    )?;
    runtime.register(
        "String::graphemeSubstring",
        NativeFunction::new(
            NativeSignature::new(vec![Text, Int], Text).with_optional(vec![Int]),
            |context, args| {
                let Value::String(input) = &args[0] else {
                    unreachable!()
                };
                let mut count = 0;
                for _ in input.graphemes(true) {
                    context.check_cancelled()?;
                    count += 1;
                }
                let positions = range(args, count)?;
                let mut start = input.len();
                let mut end = input.len();
                for (index, (byte, _)) in input.grapheme_indices(true).enumerate() {
                    context.check_cancelled()?;
                    if index == positions.start {
                        start = byte;
                    }
                    if index == positions.end {
                        end = byte;
                        break;
                    }
                }
                payload(context, end - start)?;
                Ok(Value::String(input[start..end].into()))
            },
        ),
    )?;
    Ok(())
}
