use crate::environment::text;
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use std::io::{BufRead, Read};

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
