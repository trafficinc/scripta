//! Bounded, owned failure snapshots; no handles or backend messages are retained.
use scr_runtime::{MapKey, NativeError, NativeResult, Value};
use std::mem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Connection,
    Authentication,
    Constraint,
    Query,
    Timeout,
    State,
    Unsupported,
    Limit,
    Type,
}
impl ErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::Authentication => "authentication",
            Self::Constraint => "constraint",
            Self::Query => "query",
            Self::Timeout => "timeout",
            Self::State => "state",
            Self::Unsupported => "unsupported",
            Self::Limit => "limit",
            Self::Type => "type",
        }
    }
    fn from_kind(kind: &str) -> Self {
        match kind {
            "DbTimeoutError" => Self::Timeout,
            "DbStateError" => Self::State,
            "DbUnsupportedError" => Self::Unsupported,
            "DbLimitError" => Self::Limit,
            "DbTypeError" => Self::Type,
            _ => Self::Query,
        }
    }
}
/// Sanitized driver details passed separately from native diagnostic messages.
/// Supply codes, never server text, SQL, DSNs, or credentials. Invalid/oversized
/// codes are omitted from snapshots. Drivers must sanitize NativeError messages.
#[derive(Debug, Clone)]
pub struct ErrorDetails {
    pub category: ErrorCategory,
    pub sql_state: Option<String>,
    pub driver_code: Option<String>,
}

pub(super) struct Record {
    operation: &'static str,
    category: ErrorCategory,
    sql_state: Option<String>,
    driver_code: Option<String>,
    message: String,
}
impl Record {
    pub fn new(
        operation: &'static str,
        error: &NativeError,
        details: Option<ErrorDetails>,
    ) -> Self {
        let details = details
            .filter(|_| error.kind == "DbError")
            .unwrap_or(ErrorDetails {
                category: ErrorCategory::from_kind(&error.kind),
                sql_state: None,
                driver_code: None,
            });
        let mut end = error.message.len().min(1024);
        while !error.message.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            operation,
            category: details.category,
            sql_state: details.sql_state.filter(|s| {
                s.len() == 5
                    && s.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            }),
            driver_code: details.driver_code.filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            }),
            message: error.message[..end].to_owned(),
        }
    }
    pub fn payload(&self) -> usize {
        mem::size_of::<Self>()
            + self.message.len()
            + self.sql_state.as_ref().map_or(0, String::len)
            + self.driver_code.as_ref().map_or(0, String::len)
    }
    pub fn value(&self, budget: Option<usize>) -> NativeResult {
        let fields = [
            ("operation", Some(self.operation)),
            ("category", Some(self.category.as_str())),
            ("sqlState", self.sql_state.as_deref()),
            ("driverCode", self.driver_code.as_deref()),
            ("message", Some(self.message.as_str())),
        ];
        let size = mem::size_of::<Value>()
            + fields
                .iter()
                .map(|(key, value)| {
                    mem::size_of::<MapKey>()
                        + mem::size_of::<Value>()
                        + key.len()
                        + value.map_or(0, str::len)
                })
                .sum::<usize>();
        if budget.is_some_and(|limit| size > limit) {
            return Err(limit());
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(fields.len())
            .map_err(|_| limit())?;
        for (key, value) in fields {
            entries.push((
                MapKey::String(copy(key)?),
                match value {
                    Some(s) => Value::String(copy(s)?),
                    None => Value::Null,
                },
            ));
        }
        Ok(Value::Map(entries))
    }
}
fn limit() -> NativeError {
    NativeError::new(
        "DbLimitError",
        "database error information exceeds value limit",
    )
}
fn copy(text: &str) -> NativeResult<String> {
    let mut value = String::new();
    value.try_reserve_exact(text.len()).map_err(|_| limit())?;
    value.push_str(text);
    Ok(value)
}
pub(super) fn ignored(error: &NativeError) -> bool {
    matches!(
        error.kind.as_str(),
        "TypeError" | "IndexError" | "PermissionError" | "Interrupted" | "DbEndOfResultsError"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_utf8_snapshots_omit_invalid_codes_and_preflight_output_size() {
        let record = Record::new(
            "execute",
            &NativeError::new("DbError", "🦀".repeat(1000)),
            Some(ErrorDetails {
                category: ErrorCategory::Constraint,
                sql_state: Some("invalid".into()),
                driver_code: Some("x".repeat(65)),
            }),
        );
        assert_eq!(record.message.len(), 1024);
        assert_eq!(record.sql_state, None);
        assert_eq!(record.driver_code, None);
        let output = record.value(None).unwrap();
        let size = 6 * mem::size_of::<Value>() + 5 * mem::size_of::<MapKey>() + 42 + 7 + 10 + 1024;
        for (budget, succeeds) in [(size, true), (size - 1, false)] {
            let mut runtime = scr_runtime::Runtime::with_options(scr_runtime::RuntimeOptions {
                max_value_bytes: Some(budget),
                ..Default::default()
            });
            assert_eq!(runtime.set_global("info", output.clone()).is_ok(), succeeds);
        }
        assert_eq!(record.value(Some(size)).unwrap(), output);
        assert_eq!(
            record.value(Some(size - 1)).unwrap_err().kind,
            "DbLimitError"
        );
    }
}
