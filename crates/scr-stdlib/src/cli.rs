use crate::environment::text;
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::io::{BufRead, Read};
use std::{cell::Cell, mem::size_of, rc::Rc};

fn option(args: &[String], requested: &str) -> NativeResult<(bool, Option<String>)> {
    let requested = requested.trim_start_matches('-');
    if requested.is_empty() || requested.contains('=') {
        return Err(NativeError::type_error(
            "option name must be nonempty without '='",
        ));
    }
    let mut result = (false, None);
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            break;
        }
        if let Some(name) = arg.strip_prefix('-') {
            let name = name.strip_prefix('-').unwrap_or(name);
            let (name, inline) = name
                .split_once('=')
                .map_or((name, None), |(key, value)| (key, Some(value)));
            if name == requested {
                let following = args.get(index + 1).filter(|value| !value.starts_with('-'));
                result = (
                    true,
                    inline.map(str::to_owned).or_else(|| following.cloned()),
                );
            }
            if inline.is_none()
                && args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with('-'))
            {
                index += 1;
            }
        }
        index += 1;
    }
    Ok(result)
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    let input_failed = Rc::new(Cell::new(false));
    let read_failed = input_failed.clone();
    runtime.register(
        "Cli::readLine",
        NativeFunction::new(
            NativeSignature::new(
                vec![],
                NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
            )
            .with_optional(vec![NativeType::Int]),
            move |context, args| {
                context.permissions.require("cli.read")?;
                let maximum = match args.first() {
                    None => 1024 * 1024,
                    Some(Value::Int(n)) => {
                        usize::try_from(*n).ok().filter(|&n| n > 0).ok_or_else(|| {
                            NativeError::new("RangeError", "maxLineBytes must be a positive int")
                        })?
                    }
                    _ => unreachable!("validated signature"),
                };
                if read_failed.get() {
                    return Err(NativeError::new(
                        "InputError",
                        "stdin is unavailable after an input failure",
                    ));
                }
                let result = read_line(context, maximum);
                if result.is_err() {
                    read_failed.set(true);
                }
                result
            },
        ),
    )?;
    for (method, params, returns) in [
        ("args", vec![], NativeType::List),
        (
            "arg",
            vec![NativeType::Int],
            NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
        ),
        ("has", vec![NativeType::String], NativeType::Bool),
        (
            "option",
            vec![NativeType::String],
            NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
        ),
        (
            "prompt",
            vec![NativeType::String],
            NativeType::OneOf(vec![NativeType::String, NativeType::Null]),
        ),
    ] {
        let input_failed = input_failed.clone();
        runtime.register(
            format!("Cli::{method}"),
            NativeFunction::new(
                NativeSignature::new(params, returns),
                move |context, args| match method {
                    "args" => context
                        .cli_arguments
                        .iter()
                        .map(|value| text(value).map(Value::String))
                        .collect::<NativeResult<Vec<_>>>()
                        .map(Value::List),
                    "arg" => {
                        let [Value::Int(index)] = args else {
                            return Err(NativeError::type_error("Cli::arg requires one int index"));
                        };
                        let index = usize::try_from(*index).map_err(|_| {
                            NativeError::type_error("CLI argument index must be nonnegative")
                        })?;
                        context
                            .cli_arguments
                            .get(index)
                            .map_or(Ok(Value::Null), |value| text(value).map(Value::String))
                    }
                    "has" | "option" => {
                        let [Value::String(name)] = args else {
                            return Err(NativeError::type_error(
                                "CLI option name must be a string",
                            ));
                        };
                        let arguments = context
                            .cli_arguments
                            .iter()
                            .map(|value| text(value))
                            .collect::<NativeResult<Vec<_>>>()?;
                        let (has, value) = option(&arguments, name)?;
                        Ok(if method == "has" {
                            Value::Bool(has)
                        } else {
                            value.map_or(Value::Null, Value::String)
                        })
                    }
                    "prompt" => {
                        context.permissions.require("cli.prompt")?;
                        if input_failed.get() {
                            return Err(NativeError::new(
                                "InputError",
                                "stdin is unavailable after an input failure",
                            ));
                        }
                        let [Value::String(message)] = args else {
                            return Err(NativeError::type_error("prompt message must be a string"));
                        };
                        context.check_cancelled()?;
                        context.output.write_all(message.as_bytes())?;
                        context.output.flush()?;
                        let mut bytes = Vec::new();
                        (&mut *context.input)
                            .take(65537)
                            .read_until(b'\n', &mut bytes)?;
                        context.check_cancelled()?;
                        if bytes.len() > 65536 {
                            return Err(NativeError::new(
                                "InputLimitError",
                                "prompt input exceeds 64 KiB",
                            ));
                        }
                        if bytes.is_empty() {
                            return Ok(Value::Null);
                        }
                        if bytes.last() == Some(&b'\n') {
                            bytes.pop();
                            if bytes.last() == Some(&b'\r') {
                                bytes.pop();
                            }
                        }
                        String::from_utf8(bytes).map(Value::String).map_err(|_| {
                            NativeError::new("EncodingError", "prompt input is not valid UTF-8")
                        })
                    }
                    _ => Err(NativeError::new("NativeError", "unknown CLI operation")),
                },
            ),
        )?;
    }
    Ok(())
}

fn read_line(context: &mut RuntimeContext<'_>, maximum: usize) -> NativeResult {
    let resource = |message| NativeError::new("ResourceLimitError", message);
    let payload_limit = context
        .max_value_bytes
        .map(|limit| {
            limit
                .checked_sub(size_of::<Value>())
                .ok_or_else(|| resource("line result exceeds execution value byte limit"))
        })
        .transpose()?;
    let mut bytes = Vec::new();
    loop {
        context.check_cancelled()?;
        let available = context.input.fill_buf()?;
        if available.is_empty() {
            break;
        }
        let available = &available[..available.len().min(8192)];
        let newline = available.iter().position(|&b| b == b'\n');
        let count = newline.map_or(available.len(), |i| i + 1);
        let length = bytes
            .len()
            .checked_add(count)
            .ok_or_else(|| resource("line size overflow"))?;
        if length > maximum {
            return Err(resource("line exceeds maxLineBytes"));
        }
        if payload_limit.is_some_and(|limit| length > limit.saturating_add(2)) {
            return Err(resource("line result exceeds execution value byte limit"));
        }
        bytes
            .try_reserve_exact(count)
            .map_err(|_| resource("line allocation failed"))?;
        bytes.extend_from_slice(&available[..count]);
        context.input.consume(count);
        if newline.is_some() {
            break;
        }
    }
    context.check_cancelled()?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if payload_limit.is_some_and(|limit| bytes.len() > limit) {
        return Err(resource("line result exceeds execution value byte limit"));
    }
    String::from_utf8(bytes)
        .map(Value::String)
        .map_err(|_| NativeError::new("EncodingError", "stdin line is not valid UTF-8"))
}
