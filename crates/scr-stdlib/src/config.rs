//! Strict, bounded configuration-to-value conversion.
use crate::bytes;
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{collections::BTreeSet, mem::size_of};

#[path = "config_yaml.rs"]
mod yaml;

struct Limits {
    input: usize,
    output: usize,
    values: usize,
    depth: usize,
}
impl Limits {
    fn parse(value: Option<&Value>) -> NativeResult<Self> {
        let mut result = Self {
            input: 1048576,
            output: 16777216,
            values: 10000,
            depth: 64,
        };
        if let Some(Value::Map(entries)) = value {
            for (key, value) in entries {
                let MapKey::String(key) = key else {
                    return Err(NativeError::type_error(
                        "configuration option keys must be strings",
                    ));
                };
                let (target, maximum) = match key.as_str() {
                    "maxInputBytes" => (&mut result.input, 1048576),
                    "maxOutputBytes" => (&mut result.output, 16777216),
                    "maxValues" => (&mut result.values, 10000),
                    "maxDepth" => (&mut result.depth, 64),
                    _ => return Err(NativeError::type_error("unknown configuration option")),
                };
                let Value::Int(n) = value else {
                    return Err(NativeError::type_error("configuration limits must be ints"));
                };
                *target = usize::try_from(*n)
                    .ok()
                    .filter(|n| *n <= maximum)
                    .ok_or_else(|| {
                        NativeError::new(
                            "RangeError",
                            format!("{key} must be between 0 and {maximum}"),
                        )
                    })?;
            }
        }
        Ok(result)
    }
}
struct Budget<'a, 'ctx> {
    context: &'a RuntimeContext<'ctx>,
    limits: Limits,
    bytes: usize,
    values: usize,
}
impl Budget<'_, '_> {
    fn add(&mut self, extra: usize) -> NativeResult<()> {
        self.context.check_cancelled()?;
        bytes::grow(self.context, &mut self.bytes, extra)?;
        if self.bytes > self.limits.output {
            return Err(bytes::resource("configuration exceeds maxOutputBytes"));
        }
        Ok(())
    }
    fn value(&mut self, depth: usize, payload: usize) -> NativeResult<()> {
        self.context.check_cancelled()?;
        if depth > self.limits.depth {
            return Err(bytes::resource("configuration exceeds maxDepth"));
        }
        if self.values >= self.limits.values {
            return Err(bytes::resource("configuration exceeds maxValues"));
        }
        self.values += 1;
        self.add(size_of::<Value>())?;
        self.add(payload)
    }
    fn key(&mut self, key: &str) -> NativeResult<()> {
        self.add(size_of::<MapKey>())?;
        self.add(key.len())
    }
}
fn error(format: &str, message: &str) -> NativeError {
    NativeError::new(format!("{format}Error"), message)
}
fn located(format: &str, line: usize, column: usize, message: &str) -> NativeError {
    error(
        format,
        &format!("{message} at line {line}, column {column}"),
    )
}
fn position(text: &str, byte: usize) -> (usize, usize) {
    let mut end = byte.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &text[..end];
    (
        prefix.bytes().filter(|b| *b == b'\n').count() + 1,
        prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
    )
}
fn toml_value(
    budget: &mut Budget<'_, '_>,
    value: toml::Value,
    depth: usize,
) -> NativeResult<Value> {
    budget.value(
        depth,
        if let toml::Value::String(s) = &value {
            s.len()
        } else {
            0
        },
    )?;
    Ok(match value {
        toml::Value::String(s) => Value::String(s),
        toml::Value::Integer(n) => Value::Int(n),
        toml::Value::Float(n) if n.is_finite() => Value::Float(n),
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Datetime(_) => {
            return Err(error(
                "Toml",
                "datetime values require explicit quoted strings",
            ));
        }
        toml::Value::Float(_) => return Err(error("Toml", "nonfinite floats are unsupported")),
        toml::Value::Array(items) => {
            let mut values = Vec::new();
            for item in items {
                let value = toml_value(budget, item, depth + 1)?;
                values.push(value);
            }
            Value::List(values)
        }
        toml::Value::Table(items) => {
            let mut values = Vec::new();
            for (key, item) in items {
                budget.key(&key)?;
                let value = toml_value(budget, item, depth + 1)?;
                values.push((MapKey::String(key), value));
            }
            Value::Map(values)
        }
    })
}
fn toml_parse(budget: &mut Budget<'_, '_>, text: &str) -> NativeResult<Value> {
    let result = toml::from_str::<toml::Table>(text);
    budget.context.check_cancelled()?;
    let table = result.map_err(|e| {
        let (line, column) = position(text, e.span().map_or(0, |s| s.start));
        located("Toml", line, column, "invalid TOML")
    })?;
    toml_value(budget, toml::Value::Table(table), 0)
}
fn ini_parse(budget: &mut Budget<'_, '_>, text: &str) -> NativeResult<Value> {
    budget.value(0, 0)?;
    budget.key("")?;
    budget.value(1, 0)?;
    let mut sections = vec![(MapKey::String(String::new()), Value::Map(Vec::new()))];
    let mut section_names = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        budget.context.check_cancelled()?;
        let raw = if let Some(raw) = raw.strip_suffix('\n') {
            raw.strip_suffix('\r').unwrap_or(raw)
        } else {
            raw
        };
        if let Some((column, _)) = raw
            .chars()
            .enumerate()
            .find(|(_, c)| c.is_control() && *c != '\t')
        {
            return Err(located(
                "Ini",
                index + 1,
                column + 1,
                "invalid control character",
            ));
        }
        let line = raw.trim_matches([' ', '\t']);
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            let name = name
                .strip_suffix(']')
                .map(|s| s.trim_matches([' ', '\t']))
                .filter(|s| !s.is_empty() && !s.contains(['[', ']']))
                .ok_or_else(|| located("Ini", index + 1, 1, "invalid section"))?;
            if !section_names.insert(name) {
                return Err(located("Ini", index + 1, 1, "duplicate section"));
            }
            budget.key(name)?;
            budget.value(1, 0)?;
            sections.push((MapKey::String(name.into()), Value::Map(Vec::new())));
            keys.clear();
        } else {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| located("Ini", index + 1, 1, "expected key=value"))?;
            let key = key.trim_matches([' ', '\t']);
            if key.is_empty() || !keys.insert(key) {
                return Err(located("Ini", index + 1, 1, "empty or duplicate key"));
            }
            let value = value.trim_matches([' ', '\t']);
            budget.key(key)?;
            budget.value(2, value.len())?;
            let Value::Map(items) = &mut sections.last_mut().unwrap().1 else {
                unreachable!()
            };
            items.push((MapKey::String(key.into()), Value::String(value.into())));
        }
    }
    Ok(Value::Map(sections))
}
fn parse(
    context: &RuntimeContext<'_>,
    format: &str,
    text: &str,
    options: Option<&Value>,
) -> NativeResult<Value> {
    context.check_cancelled()?;
    let limits = Limits::parse(options)?;
    if text.len() > limits.input {
        return Err(bytes::resource("configuration exceeds maxInputBytes"));
    }
    let mut budget = Budget {
        context,
        limits,
        bytes: 0,
        values: 0,
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value = match format {
        "Toml" => toml_parse(&mut budget, text),
        "Yaml" => yaml::parse(&mut budget, text),
        "Ini" => ini_parse(&mut budget, text),
        _ => unreachable!(),
    }?;
    context.check_cancelled()?;
    Ok(value)
}
pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    for format in ["Toml", "Yaml", "Ini"] {
        runtime.register(
            format!("{format}::parse"),
            NativeFunction::new(
                NativeSignature::new(
                    vec![NativeType::String],
                    if format == "Yaml" {
                        NativeType::Any
                    } else {
                        NativeType::Map
                    },
                )
                .with_optional(vec![NativeType::Map]),
                move |context, args| {
                    let Value::String(text) = &args[0] else {
                        unreachable!()
                    };
                    parse(context, format, text, args.get(1))
                },
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: None,
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }
    #[test]
    fn configuration_output_counts_exact_logical_bytes_keys_and_slots() {
        with_context(|context| {
            for (format, source, size) in [
                (
                    "Toml",
                    "a=['é']",
                    3 * size_of::<Value>() + size_of::<MapKey>() + 3,
                ),
                (
                    "Yaml",
                    "a: [é]",
                    3 * size_of::<Value>() + size_of::<MapKey>() + 3,
                ),
                (
                    "Ini",
                    "a=é",
                    3 * size_of::<Value>() + 2 * size_of::<MapKey>() + 3,
                ),
                ("Yaml", "null", size_of::<Value>()),
                ("Toml", "", size_of::<Value>()),
                ("Ini", "", 2 * size_of::<Value>() + size_of::<MapKey>()),
            ] {
                let options = |n| {
                    Value::Map(vec![(
                        MapKey::String("maxOutputBytes".into()),
                        Value::Int(n as i64),
                    )])
                };
                context.max_value_bytes = Some(size);
                let expected = parse(context, format, source, Some(&options(size))).unwrap();
                assert_eq!(parse(context, format, source, None).unwrap(), expected);
                context.max_value_bytes = Some(size - 1);
                assert_eq!(
                    parse(context, format, source, None).unwrap_err().kind,
                    "ResourceLimitError"
                );
                context.max_value_bytes = None;
                assert_eq!(
                    parse(context, format, source, Some(&options(size - 1)))
                        .unwrap_err()
                        .kind,
                    "ResourceLimitError"
                );
            }
        });
    }
    #[test]
    fn configuration_cancellation_keeps_its_reserved_error_kind() {
        with_context(|context| {
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            for (format, source) in [("Toml", "a=1"), ("Yaml", "a: 1"), ("Ini", "a=1")] {
                assert_eq!(
                    parse(context, format, source, None).unwrap_err().kind,
                    "Interrupted"
                );
            }
        });
    }
}
