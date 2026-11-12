//! Standard library foundation, registered through the public native interface.
#![forbid(unsafe_code)]
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::time::{Duration, Instant};
mod cli;
mod collections;
mod date;
mod environment;
pub mod filesystem;
pub mod http;
mod json;
pub mod log;
mod math;
mod paths;
pub mod process;
mod strings;
mod system;
pub use filesystem::FilesystemPolicy;

#[derive(Debug, Clone, Default)]
pub struct StdlibOptions {
    pub filesystem: FilesystemPolicy,
    pub process: process::ProcessConfig,
    pub log: log::LogConfig,
    pub http: http::HttpConfig,
}

pub fn register(runtime: &mut Runtime) -> NativeResult<()> {
    register_with_options(runtime, StdlibOptions::default())
}

pub fn register_with_filesystem_policy(
    runtime: &mut Runtime,
    policy: FilesystemPolicy,
) -> NativeResult<()> {
    register_with_options(
        runtime,
        StdlibOptions {
            filesystem: policy,
            ..Default::default()
        },
    )
}

pub fn register_with_options(runtime: &mut Runtime, options: StdlibOptions) -> NativeResult<()> {
    for (name, convert, returns) in [
        (
            "str",
            str_value as fn(&[Value]) -> Result<Value, String>,
            NativeType::String,
        ),
        ("int", int_value, NativeType::Int),
        ("float", float_value, NativeType::Float),
        ("bool", bool_value, NativeType::Bool),
    ] {
        runtime.register_native(
            name,
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::Any], returns),
                move |_, args| convert(args).map_err(NativeError::type_error),
            ),
        )?;
    }
    runtime.register_native(
        "type",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Any], NativeType::String),
            |_, args| {
                let [value] = args else {
                    return Err(NativeError::type_error(
                        "type() expects exactly one argument",
                    ));
                };
                Ok(Value::String(value.type_name().into()))
            },
        ),
    )?;
    runtime.register_native(
        "len",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::OneOf(vec![
                    NativeType::String,
                    NativeType::List,
                    NativeType::Map,
                ])],
                NativeType::Int,
            ),
            |_, args| len_value(args),
        ),
    )?;
    runtime.register_native(
        "sleep",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Int], NativeType::Null),
            sleep_value,
        ),
    )?;
    let module_policy = options.filesystem.clone().prepare()?;
    runtime.set_module_policy(move |path| module_policy.check_path(path, false));
    json::register(runtime)?;
    strings::register(runtime)?;
    math::register(runtime)?;
    collections::register(runtime)?;
    http::register(runtime, options.http)?;
    filesystem::register(runtime, options.filesystem.clone())?;
    system::register(runtime, options.filesystem.clone())?;
    date::register(runtime)?;
    log::register(runtime, options.log, options.filesystem)?;
    paths::register(runtime)?;
    process::register(runtime, options.process)?;
    environment::register(runtime)?;
    cli::register(runtime)?;
    Ok(())
}

fn len_value(args: &[Value]) -> NativeResult {
    let length = match args {
        [Value::String(value)] => value.chars().count(),
        [Value::List(value)] => value.len(),
        [Value::Map(value)] => value.len(),
        _ => {
            return Err(NativeError::type_error(
                "len() expects one string, list, or map",
            ));
        }
    };
    i64::try_from(length)
        .map(Value::Int)
        .map_err(|_| NativeError::new("RangeError", "length exceeds i64 range"))
}

fn sleep_value(context: &mut RuntimeContext<'_>, args: &[Value]) -> NativeResult {
    let [Value::Int(milliseconds)] = args else {
        return Err(NativeError::type_error("sleep() expects one int duration"));
    };
    let milliseconds = u64::try_from(*milliseconds)
        .map_err(|_| NativeError::type_error("sleep() duration must be nonnegative"))?;
    context.permissions.require("sleep")?;
    let duration = Duration::from_millis(milliseconds);
    let start = Instant::now();
    loop {
        context.check_cancelled()?;
        let remaining = duration.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Ok(Value::Null);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn argument<'a>(name: &str, args: &'a [Value]) -> Result<&'a Value, String> {
    let [value] = args else {
        return Err(format!("{name}() expects exactly one argument"));
    };
    Ok(value)
}

fn str_value(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(match argument("str", args)? {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_finite() => value.to_string(),
        Value::String(value) => value.clone(),
        _ => return Err("str() requires a scalar value with a finite numeric value".into()),
    }))
}

fn int_value(args: &[Value]) -> Result<Value, String> {
    let invalid =
        || "invalid conversion to int; expected an integral value in i64 range".to_string();
    let value = match argument("int", args)? {
        Value::Int(value) => *value,
        Value::Bool(value) => i64::from(*value),
        Value::Float(value)
            if value.is_finite()
                && value.fract() == 0.0
                && *value >= i64::MIN as f64
                && *value < -(i64::MIN as f64) =>
        {
            *value as i64
        }
        Value::String(value) => value.parse::<i64>().map_err(|_| invalid())?,
        _ => return Err(invalid()),
    };
    Ok(Value::Int(value))
}

fn float_value(args: &[Value]) -> Result<Value, String> {
    let invalid = || "invalid conversion to float; expected a finite numeric value".to_string();
    let value = match argument("float", args)? {
        Value::Float(value) => *value,
        Value::Int(value) => *value as f64,
        Value::Bool(value) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        Value::String(value) => value.parse::<f64>().map_err(|_| invalid())?,
        _ => return Err(invalid()),
    };
    if !value.is_finite() {
        return Err(invalid());
    }
    Ok(Value::Float(value))
}

fn bool_value(args: &[Value]) -> Result<Value, String> {
    let value = match argument("bool", args)? {
        Value::Bool(value) => *value,
        Value::Int(0) => false,
        Value::Int(1) => true,
        Value::Float(value) if *value == 0.0 => false,
        Value::Float(value) if *value == 1.0 => true,
        Value::String(value) if value == "true" => true,
        Value::String(value) if value == "false" => false,
        _ => {
            return Err(
                "invalid conversion to bool; expected bool, 0, 1, or the strings true/false".into(),
            );
        }
    };
    Ok(Value::Bool(value))
}

#[cfg(test)]
mod tests;
