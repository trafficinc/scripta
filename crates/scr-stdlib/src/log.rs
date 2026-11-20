//! Local JSON-lines logging. No network transports or global logger state.
use crate::{date, filesystem::FilesystemPolicy, paths};
use chrono::SecondsFormat;
use scr_runtime::{NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value};
use std::{cell::RefCell, fs::OpenOptions, io::Write, path::PathBuf, rc::Rc};

/// Host defaults. A file replaces stderr. Relative paths use the runtime cwd.
#[derive(Debug, Clone, Default)]
pub struct LogConfig {
    pub file: Option<PathBuf>,
    /// Literal sensitive values to replace, in addition to conservative detection.
    pub redacted_values: Vec<String>,
}
fn sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "api-key",
        "api key",
        "authorization",
        "credential",
        "private_key",
        "private key",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}
fn redact(message: &str, secrets: impl Iterator<Item = String>) -> String {
    // Drop the entire message on credential-shaped content rather than risk
    // retaining part of a quoted/multiline value or URL userinfo.
    let lower = message.to_ascii_lowercase();
    if sensitive_name(message)
        || lower.contains("bearer ")
        || lower.contains("://") && lower.contains('@')
        || lower.contains("-----begin")
    {
        return "[REDACTED]".into();
    }
    let mut result = message.to_owned();
    let mut secrets: Vec<_> = secrets.filter(|value| !value.is_empty()).collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for secret in secrets {
        result = result.replace(&secret, "[REDACTED]");
    }
    result
}
pub(crate) fn register(
    runtime: &mut Runtime,
    config: LogConfig,
    policy: FilesystemPolicy,
) -> NativeResult<()> {
    let policy = Rc::new(policy.prepare()?);
    let destination = Rc::new(RefCell::new(config.file));
    let file_destination = destination.clone();
    let file_policy = policy.clone();
    runtime.register(
        "Log::setFile",
        NativeFunction::new(
            NativeSignature::new(
                vec![NativeType::OneOf(vec![
                    NativeType::String,
                    NativeType::Null,
                ])],
                NativeType::Null,
            ),
            move |context, args| {
                context.permissions.require("log.write")?;
                let path = match &args[0] {
                    Value::Null => None,
                    Value::String(path) => {
                        let path = paths::resolve(context, path)?;
                        file_policy.check(context, &path, true)?;
                        // Validate before switching destinations; append, never truncate.
                        OpenOptions::new().create(true).append(true).open(&path)?;
                        Some(path)
                    }
                    _ => unreachable!(),
                };
                *file_destination.borrow_mut() = path;
                Ok(Value::Null)
            },
        ),
    )?;
    let redacted_values = Rc::new(config.redacted_values);
    for level in ["info", "warn", "error", "debug"] {
        let destination = destination.clone();
        let policy = policy.clone();
        let redacted_values = redacted_values.clone();
        runtime.register(format!("Log::{level}"), NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Null), move |context, args| {
                context.permissions.require("log.write")?;
                let [Value::String(message)] = args else { unreachable!() };
                let environment_secrets = context.environment.iter()
                    .filter(|(key, _)| sensitive_name(&key.to_string_lossy()))
                    .filter_map(|(_, value)| value.to_str().map(str::to_owned));
                let message = redact(message, redacted_values.iter().cloned().chain(environment_secrets));
                let timestamp = date::timestamp(date::now()?)?.to_rfc3339_opts(SecondsFormat::Millis, true);
                let mut record = serde_json::json!({"timestamp": timestamp, "level": level, "message": message}).to_string();
                record.push('\n');
                if let Some(path) = destination.borrow().as_ref() {
                    let path = paths::resolve(context, &paths::text(path.as_os_str())?)?;
                    policy.check(context, &path, true)?;
                    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
                    file.write_all(record.as_bytes())?;
                    file.flush()?;
                } else {
                    context.error.write_all(record.as_bytes())?;
                    context.error.flush()?;
                }
                Ok(Value::Null)
            }))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_removed_before_serialization() {
        for value in [
            "password=hello",
            "{\"api_key\":\"hello\"}",
            "Authorization: Basic abc",
            "Bearer abc",
            "https://user:pass@example.org",
            "-----BEGIN RSA PRIVATE KEY-----",
        ] {
            assert_eq!(redact(value, std::iter::empty()), "[REDACTED]");
        }
        assert_eq!(
            redact("hello abc 世界", ["abc".into()].into_iter()),
            "hello [REDACTED] 世界"
        );
    }
}
