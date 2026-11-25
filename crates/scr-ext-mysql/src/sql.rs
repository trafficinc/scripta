//! Conservative MySQL statement/placeholder scanning. Values are never SQL text.
use crate::settings::MysqlOptions;
use scr_runtime::{MapKey, NativeError, NativeResult, RuntimeContext, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    mem,
};

#[derive(Clone, Copy, Default)]
pub(crate) struct Mode {
    pub no_backslash: bool,
    pub ansi_quotes: bool,
}
#[derive(Clone)]
pub(crate) enum Parameters {
    Positional(usize),
    Named(Vec<String>),
}
impl Parameters {
    pub fn len(&self) -> usize {
        match self {
            Self::Positional(n) => *n,
            Self::Named(names) => names.len(),
        }
    }
    pub fn payload(&self) -> usize {
        match self {
            Self::Positional(_) => 0,
            Self::Named(names) => names
                .iter()
                .map(|n| n.len() + mem::size_of::<String>())
                .sum(),
        }
    }
}
pub(crate) struct Parsed {
    pub sql: String,
    pub parameters: Parameters,
}
fn invalid(message: &str) -> NativeError {
    NativeError::type_error(message)
}
fn unsupported(message: &str) -> NativeError {
    NativeError::new("DbUnsupportedError", message)
}
fn limit(message: &str) -> NativeError {
    NativeError::new("DbLimitError", message)
}
fn word_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}
fn word(b: u8) -> bool {
    word_start(b) || b.is_ascii_digit() || b == b'$' || b >= 128
}

pub(crate) fn parse(
    sql: &str,
    mode: Mode,
    max_params: usize,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<Parsed> {
    if sql.contains('\0') {
        return Err(invalid("SQL contains NUL"));
    }
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut copied = 0;
    let mut normalized = String::with_capacity(sql.len());
    let mut first = None;
    let mut terminated = false;
    let mut positional = 0;
    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    while i < bytes.len() {
        if i % 1024 < 2 {
            checkpoint()?;
        }
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'#'
            || (b == b'-'
                && bytes.get(i + 1) == Some(&b'-')
                && bytes.get(i + 2).is_some_and(|b| *b <= b' ' || *b == 127))
        {
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
                if i % 1024 < 2 {
                    checkpoint()?;
                }
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            if matches!(bytes.get(i + 2), Some(b'!' | b'+'))
                || bytes.get(i + 2..i + 4) == Some(b"M!")
            {
                return Err(unsupported(
                    "executable SQL comments and optimizer hints are not supported",
                ));
            }
            i += 2;
            let mut closed = false;
            while i + 1 < bytes.len() {
                if i % 1024 < 2 {
                    checkpoint()?;
                }
                if bytes.get(i..i + 2) == Some(b"/*") {
                    return Err(unsupported("nested SQL comments are not supported"));
                }
                if bytes.get(i..i + 2) == Some(b"*/") {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(invalid("unterminated SQL comment"));
            }
            continue;
        }
        if terminated {
            return Err(unsupported("only one SQL statement is allowed"));
        }
        if first.is_none() && !word_start(b) {
            return Err(unsupported(
                "expected SELECT, INSERT, UPDATE, DELETE, or REPLACE",
            ));
        }
        if matches!(b, b'\'' | b'"' | b'`') {
            let quote = b;
            let backslash =
                !mode.no_backslash && (quote == b'\'' || (quote == b'"' && !mode.ansi_quotes));
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if i % 1024 < 2 {
                    checkpoint()?;
                }
                if backslash && bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    if bytes.get(i) == Some(&quote) {
                        i += 1;
                        continue;
                    }
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return Err(invalid("unterminated SQL quote"));
            }
            continue;
        }
        if b == b';' {
            terminated = true;
            i += 1;
            continue;
        }
        if b == b'@' {
            return Err(unsupported("SQL session variables are not supported"));
        }
        if b == b'?' {
            if !names.is_empty() {
                return Err(invalid("cannot mix named and positional SQL parameters"));
            }
            positional += 1;
            if positional > max_params {
                return Err(limit("SQL parameter count exceeds host limit"));
            }
            i += 1;
            continue;
        }
        if b == b':' {
            if positional != 0 || !bytes.get(i + 1).is_some_and(|b| word_start(*b)) {
                return Err(invalid("invalid or mixed SQL parameter marker"));
            }
            let start = i;
            i += 2;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if names.len() >= max_params {
                return Err(limit("SQL parameter count exceeds host limit"));
            }
            let name = sql[start + 1..i].to_owned();
            if !seen.insert(name.clone()) {
                return Err(invalid("SQL parameter names must be unique"));
            }
            names.push(name);
            normalized.push_str(&sql[copied..start]);
            normalized.push('?');
            copied = i;
            continue;
        }
        if word(b) {
            let start = i;
            while i < bytes.len() && word(bytes[i]) {
                i += 1;
                if i % 1024 < 2 {
                    checkpoint()?;
                }
            }
            let token = &sql[start..i];
            if first.is_none() {
                let upper = token.to_ascii_uppercase();
                if !matches!(
                    upper.as_str(),
                    "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "REPLACE"
                ) {
                    return Err(unsupported(
                        "statement kind is not supported by prepared execution",
                    ));
                }
                first = Some(upper);
            } else if token.eq_ignore_ascii_case("RETURNING")
                || token.eq_ignore_ascii_case("PROCEDURE")
                || (first.as_deref() == Some("SELECT") && token.eq_ignore_ascii_case("INTO"))
            {
                return Err(unsupported("SQL statement form is not supported"));
            }
            continue;
        }
        if b < 32 || b == 127 {
            return Err(invalid("unsupported SQL control character"));
        }
        i += 1;
    }
    if first.is_none() {
        return Err(invalid("SQL statement is empty"));
    }
    normalized.push_str(&sql[copied..]);
    // mysql's own named-parameter pass is not SQL-mode aware. Never allow it to
    // rewrite a literal/comment or change the SQL we have just validated.
    let client = mysql_common::named_params::ParsedNamedParams::parse(normalized.as_bytes())
        .map_err(|_| {
            unsupported("SQL quoting is incompatible with the client placeholder parser")
        })?;
    if client.query() != normalized.as_bytes() || !client.params().is_empty() {
        return Err(unsupported(
            "SQL quoting is incompatible with the client placeholder parser",
        ));
    }
    checkpoint()?;
    Ok(Parsed {
        sql: normalized,
        parameters: if names.is_empty() {
            Parameters::Positional(positional)
        } else {
            Parameters::Named(names)
        },
    })
}

pub(crate) fn bind(
    parameters: &Parameters,
    supplied: &Value,
    policy: &MysqlOptions,
    context: &RuntimeContext<'_>,
) -> NativeResult<Vec<mysql::Value>> {
    let values: Vec<&Value> = match (parameters, supplied) {
        (Parameters::Positional(n), Value::List(values)) if *n == values.len() => {
            values.iter().collect()
        }
        (Parameters::Named(names), Value::Map(values)) if names.len() == values.len() => {
            let mut indexed = BTreeMap::new();
            for (key, value) in values {
                context.check_cancelled()?;
                let MapKey::String(key) = key else {
                    return Err(invalid("named parameter keys must be strings"));
                };
                if indexed.insert(key.as_str(), value).is_some() {
                    return Err(invalid("duplicate named parameter key"));
                }
            }
            names
                .iter()
                .map(|name| {
                    indexed
                        .get(name.as_str())
                        .copied()
                        .ok_or_else(|| invalid("missing or extra named SQL parameter"))
                })
                .collect::<NativeResult<_>>()?
        }
        _ => {
            return Err(invalid(
                "SQL parameter style or count does not match the statement",
            ));
        }
    };
    if values.len() > policy.max_parameters {
        return Err(limit("SQL parameter count exceeds host limit"));
    }
    let mut size = mem::size_of::<Value>().saturating_mul(values.len() + 1);
    for value in &values {
        context.check_cancelled()?;
        let bytes = match value {
            Value::String(s) => s.len(),
            Value::Bytes(b) => b.len(),
            Value::Null | Value::Bool(_) | Value::Int(_) => 0,
            Value::Float(f) if f.is_finite() => 0,
            _ => {
                return Err(invalid(
                    "SQL parameters require null, bool, int, finite float, string, or bytes",
                ));
            }
        };
        if bytes > policy.max_cell_bytes {
            return Err(limit("SQL parameter cell exceeds byte limit"));
        }
        size = size
            .checked_add(bytes)
            .ok_or_else(|| limit("SQL parameter bytes overflow"))?;
        if size > policy.max_parameter_bytes
            || context.max_value_bytes.is_some_and(|limit| size > limit)
        {
            return Err(limit("SQL parameter collection exceeds byte limit"));
        }
    }
    if size > policy.max_parameter_bytes
        || context.max_value_bytes.is_some_and(|limit| size > limit)
    {
        return Err(limit("SQL parameter collection exceeds byte limit"));
    }
    let mut bound = Vec::new();
    bound
        .try_reserve_exact(values.len())
        .map_err(|_| limit("SQL parameter allocation failed"))?;
    for value in values {
        context.check_cancelled()?;
        bound.push(match value {
            Value::Null => mysql::Value::NULL,
            Value::Bool(b) => mysql::Value::Int(i64::from(*b)),
            Value::Int(i) => mysql::Value::Int(*i),
            Value::Float(f) => mysql::Value::Double(*f),
            Value::String(s) => mysql::Value::Bytes(copy_bytes(s.as_bytes())?),
            Value::Bytes(b) => mysql::Value::Bytes(copy_bytes(b)?),
            _ => unreachable!("values validated before binding"),
        });
    }
    Ok(bound)
}

fn copy_bytes(bytes: &[u8]) -> NativeResult<Vec<u8>> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| limit("SQL parameter allocation failed"))?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse_default(sql: &str) -> NativeResult<Parsed> {
        parse(sql, Mode::default(), 1024, || Ok(()))
    }
    #[test]
    fn strings_identifiers_unicode_and_lf_crlf_comments_preserve_literal_markers() {
        for newline in ["\n", "\r\n"] {
            let sql = format!(
                "/* ? :ignored */ SELECT ':literal', \"?\", `:column`, :Name # :comment ?{newline}FROM `é``table`; -- trailing ?{newline}"
            );
            let parsed = parse_default(&sql).unwrap();
            let Parameters::Named(names) = parsed.parameters else {
                panic!()
            };
            assert_eq!(names, ["Name"]);
            assert_eq!(parsed.sql, sql.replace(":Name", "?"));
        }
    }
    #[test]
    fn stacked_sql_executable_comments_and_session_control_are_rejected() {
        for sql in [
            "SELECT ?; DELETE FROM users",
            "SELECT 1;;",
            "SELECT 1 /*!; DELETE FROM users*/",
            "SELECT 1 /*M! ; DROP TABLE users */",
            "SELECT /*+ SET_VAR(sql_mode='ANSI') */ 1",
            "SELECT /* /* nested */ 1",
            "CALL run()",
            "SET autocommit=0",
            "BEGIN",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "SELECT 1 INTO OUTFILE '/tmp/file'",
            "SELECT @session",
            "SELECT 'unclosed",
            "SELECT 1 /* unclosed",
            "SELECT 1\0",
        ] {
            assert!(parse_default(sql).is_err(), "{sql}");
        }
        assert!(parse_default("SELECT '; DELETE FROM users', ?; /* trailing */").is_ok());
    }
    #[test]
    fn named_markers_require_unique_names_and_one_style_and_obey_exact_limits() {
        for sql in [
            "SELECT :x,:x",
            "SELECT :x,?",
            "SELECT ?,:x",
            "SELECT :1",
            "SELECT :",
            "SELECT ::name",
        ] {
            assert!(parse_default(sql).is_err(), "{sql}");
        }
        let parsed = parse("SELECT :x,:y", Mode::default(), 2, || Ok(())).unwrap();
        assert_eq!(parsed.parameters.len(), 2);
        assert_eq!(
            parse("SELECT :x,:y,:z", Mode::default(), 2, || Ok(()))
                .err()
                .unwrap()
                .kind,
            "DbLimitError"
        );
        assert_eq!(
            parse("SELECT ?,?,?", Mode::default(), 2, || Ok(()))
                .err()
                .unwrap()
                .kind,
            "DbLimitError"
        );
    }
    #[test]
    fn sql_modes_cannot_make_driver_rewrite_quoted_parameters() {
        let no_backslash = Mode {
            no_backslash: true,
            ansi_quotes: false,
        };
        let parsed = parse("SELECT 'a\\', :x", no_backslash, 1, || Ok(())).unwrap();
        assert_eq!(parsed.parameters.len(), 1);
        let ansi = Mode {
            no_backslash: true,
            ansi_quotes: true,
        };
        assert!(parse("SELECT \"a\\\", :x", ansi, 1, || Ok(())).is_ok());
        // The driver's legacy parser misunderstands this valid mode-dependent
        // quoting. Reject it before allowing that parser to rewrite a literal.
        assert!(parse("SELECT 'a\\', ':literal'", no_backslash, 1, || Ok(())).is_err());
    }
    #[test]
    fn scanning_checks_cancellation_inside_long_literals() {
        let sql = format!("SELECT '{}'", "\\x".repeat(8192));
        let mut checkpoints = 0;
        let error = parse(&sql, Mode::default(), 1, || {
            checkpoints += 1;
            if checkpoints > 2 {
                Err(NativeError::new("Interrupted", "execution interrupted"))
            } else {
                Ok(())
            }
        })
        .err()
        .unwrap();
        assert_eq!(error.kind, "Interrupted");
    }
}
