//! Conservative MySQL statement/placeholder scanning. Values are never SQL text.
use crate::settings::PostgresOptions;
use scr_runtime::{MapKey, NativeError, NativeResult, RuntimeContext, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    mem,
};

#[derive(Clone, Copy, Default)]
pub(crate) struct Mode;
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
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Select,
    Dml,
    Ddl,
}
pub(crate) struct Parsed {
    pub kind: Kind,
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

pub(crate) struct Parameter {
    pub data: Option<Vec<u8>>,
    pub binary: bool,
}
pub(crate) fn parse(
    sql: &str,
    _mode: Mode,
    max: usize,
    checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<Parsed> {
    scan(sql, max, false, checkpoint)
}
pub(crate) fn parse_exec(
    sql: &str,
    _mode: Mode,
    checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<Parsed> {
    scan(sql, 0, true, checkpoint)
}
fn scan(
    sql: &str,
    max: usize,
    exec: bool,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<Parsed> {
    if sql.contains('\0') {
        return Err(invalid("SQL contains NUL"));
    }
    let b = sql.as_bytes();
    let mut i = 0;
    let mut copied = 0;
    let mut out = String::with_capacity(sql.len());
    let mut names = Vec::new();
    let mut seen = BTreeSet::new();
    let mut positional = 0;
    let mut tokens = Vec::new();
    let mut terminated = false;
    let mut returning = false;
    let mut previous_string = false;
    while i < b.len() {
        checkpoint()?;
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b.get(i..i + 2) == Some(b"--") {
            while i < b.len() && !matches!(b[i], b'\n' | b'\r') {
                i += 1;
                if i % 1024 == 0 {
                    checkpoint()?;
                }
            }
            continue;
        }
        if b.get(i..i + 2) == Some(b"/*") {
            let mut depth = 1usize;
            i += 2;
            while i < b.len() && depth > 0 {
                if i % 1024 == 0 {
                    checkpoint()?;
                }
                if b.get(i..i + 2) == Some(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b.get(i..i + 2) == Some(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth != 0 {
                return Err(invalid("unterminated SQL comment"));
            }
            continue;
        }
        let continuation = previous_string && b[i] == b'\'';
        previous_string = false;
        if continuation {
            return Err(unsupported(
                "adjacent SQL string literals require an explicit concatenation operator",
            ));
        }
        if terminated {
            return Err(unsupported("multiple SQL statements are not supported"));
        }
        if b[i] == b';' {
            terminated = true;
            i += 1;
            continue;
        }
        if b[i] == b'$' {
            let start = i;
            let mut end = i + 1;
            if b.get(end) != Some(&b'$') {
                if !b.get(end).is_some_and(|c| word_start(*c)) {
                    return Err(invalid("native dollar parameters are not supported"));
                }
                while end < b.len() && (word_start(b[end]) || b[end].is_ascii_digit()) {
                    end += 1;
                }
            }
            if b.get(end) != Some(&b'$') {
                return Err(invalid("invalid dollar quote"));
            }
            let tag = &sql[start..=end];
            i = end + 1;
            let mut found = false;
            while i < b.len() {
                if i % 1024 == 0 {
                    checkpoint()?;
                }
                if b[i..].starts_with(tag.as_bytes()) {
                    i += tag.len();
                    found = true;
                    break;
                }
                i += 1;
            }
            if !found {
                return Err(invalid("unterminated dollar quote"));
            }
            continue;
        }
        let escape = matches!(b[i], b'E' | b'e') && b.get(i + 1) == Some(&b'\'');
        if escape || matches!(b[i], b'\'' | b'"') {
            if escape {
                i += 1;
            }
            let quote = b[i];
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if i % 1024 == 0 {
                    checkpoint()?;
                }
                if escape && b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == quote {
                    i += 1;
                    if b.get(i) == Some(&quote) {
                        i += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            if !closed {
                return Err(invalid("unterminated SQL quote"));
            }
            previous_string = quote == b'\'';
            continue;
        }
        if b.get(i..i + 2) == Some(b"::") {
            i += 2;
            continue;
        }
        if b.get(i..i + 2) == Some(b"@?") {
            i += 2;
            continue;
        }
        if b[i] == b'?' {
            if b.get(i + 1) == Some(&b'?') {
                out.push_str(&sql[copied..i]);
                out.push('?');
                i += 2;
                copied = i;
                continue;
            }
            if matches!(b.get(i + 1), Some(b'|' | b'&')) {
                i += 2;
                continue;
            }
            if !names.is_empty() {
                return Err(invalid("cannot mix named and positional parameters"));
            }
            positional += 1;
            if positional > max {
                return Err(limit("SQL parameter count exceeds host limit"));
            }
            out.push_str(&sql[copied..i]);
            out.push('$');
            out.push_str(&positional.to_string());
            i += 1;
            copied = i;
            continue;
        }
        if b[i] == b':' {
            let start = i;
            if positional != 0 || !b.get(i + 1).is_some_and(|c| word_start(*c)) {
                return Err(invalid("invalid or mixed parameter marker"));
            }
            i += 2;
            while i < b.len() && (word_start(b[i]) || b[i].is_ascii_digit()) {
                i += 1;
            }
            let name = sql[start + 1..i].to_owned();
            if !seen.insert(name.clone()) {
                return Err(invalid("SQL parameter names must be unique"));
            }
            names.push(name);
            if names.len() > max {
                return Err(limit("SQL parameter count exceeds host limit"));
            }
            out.push_str(&sql[copied..start]);
            out.push('$');
            out.push_str(&names.len().to_string());
            copied = i;
            continue;
        }
        if word_start(b[i]) || b[i] >= 128 {
            let start = i;
            while i < b.len() && word(b[i]) {
                i += 1;
                if i % 1024 == 0 {
                    checkpoint()?;
                }
            }
            let token = sql[start..i].to_ascii_uppercase();
            if token == "RETURNING" {
                returning = true;
            }
            if token == "INTO" && tokens.first().is_some_and(|s| s == "SELECT") {
                return Err(unsupported("SELECT INTO is not supported"));
            }
            if tokens.len() < 3 {
                tokens.push(token);
            }
            continue;
        }
        if b[i] == b'`' || b[i] < 32 || b[i] == 127 {
            return Err(invalid("unsupported SQL character"));
        }
        i += 1;
    }
    let first = tokens
        .first()
        .map(String::as_str)
        .ok_or_else(|| invalid("empty SQL"))?;
    let kind = match first {
        "SELECT" => Kind::Select,
        "INSERT" | "UPDATE" | "DELETE" => Kind::Dml,
        "CREATE" | "ALTER" | "DROP"
            if exec
                && (tokens
                    .get(1)
                    .is_some_and(|s| matches!(s.as_str(), "TABLE" | "INDEX"))
                    || (first == "CREATE"
                        && tokens.get(1).is_some_and(|s| s == "UNIQUE")
                        && tokens.get(2).is_some_and(|s| s == "INDEX"))) =>
        {
            Kind::Ddl
        }
        _ => {
            return Err(unsupported(
                "statement kind is not supported by this operation",
            ));
        }
    };
    if exec && (kind == Kind::Select || returning) {
        return Err(unsupported("exec does not accept result-returning SQL"));
    }
    out.push_str(&sql[copied..]);
    checkpoint()?;
    Ok(Parsed {
        kind,
        sql: out,
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
    policy: &PostgresOptions,
    context: &RuntimeContext<'_>,
) -> NativeResult<Vec<Parameter>> {
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
            Value::Null => Parameter {
                data: None,
                binary: false,
            },
            Value::Bytes(b) => Parameter {
                data: Some(copy_bytes(b)?),
                binary: true,
            },
            value => Parameter {
                data: Some(match value {
                    Value::Bool(b) => {
                        if *b {
                            b"true".to_vec()
                        } else {
                            b"false".to_vec()
                        }
                    }
                    Value::Int(i) => i.to_string().into_bytes(),
                    Value::Float(f) => f.to_string().into_bytes(),
                    Value::String(s) => copy_bytes(s.as_bytes())?,
                    _ => unreachable!("validated values"),
                }),
                binary: false,
            },
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
    fn scan(sql: &str) -> NativeResult<Parsed> {
        parse(sql, Mode, 4, || Ok(()))
    }
    #[test]
    fn placeholders_respect_pg_quotes_casts_nested_comments_and_line_endings() {
        for newline in ["\n", "\r\n"] {
            let sql = format!(
                "SELECT :n::bigint, $$?:x$$, $t$?:y$t$, E'\\\\?', \"?:z\" /* ? /* :x */ */ --?{newline}, :tail"
            );
            let parsed = scan(&sql).unwrap();
            assert_eq!(
                parsed.sql,
                sql.replace(":n::", "$1::").replace(":tail", "$2")
            );
            let Parameters::Named(names) = parsed.parameters else {
                panic!()
            };
            assert_eq!(names, ["n", "tail"]);
        }
        assert_eq!(
            scan("SELECT ?::int, ?::text, a$b FROM t").unwrap().sql,
            "SELECT $1::int, $2::text, a$b FROM t"
        );
    }
    #[test]
    fn question_operators_and_escapes_do_not_add_parameters() {
        let p = scan("SELECT j ?? :key, j ?| ARRAY['x'], j ?& ARRAY['x'], j @? '$.x'").unwrap();
        assert_eq!(p.parameters.len(), 1);
        assert!(p.sql.contains("j ? $1"));
    }
    #[test]
    fn invalid_forms_and_exact_parameter_limits() {
        for sql in [
            "SELECT :x,:x",
            "SELECT ?, :x",
            "SELECT $1",
            "SELECT 'unclosed",
            "SELECT $$unclosed",
            "SELECT 1 /* unclosed",
            "SELECT 1; DELETE FROM t",
            "SELECT 1;;",
            "SELECT E'first'\n'second'",
            "SET x=1",
            "BEGIN",
            "COPY t TO STDOUT",
            "SELECT 1 INTO t",
            "SELECT ?,?,?,?,?",
        ] {
            assert!(scan(sql).is_err(), "{sql}");
        }
        assert_eq!(scan("SELECT ?,?,?,?;--tail").unwrap().parameters.len(), 4);
        assert!(parse_exec("INSERT INTO t VALUES (1) RETURNING id", Mode, || Ok(())).is_err());
        assert!(parse_exec("CREATE TABLE t (id int)", Mode, || Ok(())).is_ok());
    }
    #[test]
    fn cancellation_is_checked_inside_large_comments_and_quotes() {
        for sql in [
            format!("SELECT '{}'", "x".repeat(20000)),
            format!("SELECT 1 /*{}*/", "x".repeat(20000)),
        ] {
            let mut calls = 0;
            let error = parse(&sql, Mode, 4, || {
                calls += 1;
                if calls > 5 {
                    Err(NativeError::new("Interrupted", "cancelled"))
                } else {
                    Ok(())
                }
            })
            .err()
            .unwrap();
            assert_eq!(error.kind, "Interrupted");
        }
    }
}
