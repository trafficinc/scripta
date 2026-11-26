//! PostgreSQL text-format decoding preserves exact decimal and temporal strings.
use crate::protocol::{Column, Row};
use scr_runtime::{MapKey, NativeError, NativeResult, Value};
use std::{collections::BTreeSet, mem};
#[derive(Clone, Copy)]
pub(crate) enum Selection {
    Row,
    All,
    Column(usize),
}
pub(crate) struct Budget {
    pub bytes: usize,
    pub cell: usize,
}
fn kind(message: &str) -> NativeError {
    NativeError::new("DbTypeError", message)
}
fn limit() -> NativeError {
    NativeError::new(
        "DbLimitError",
        "decoded PostgreSQL result exceeds byte limit",
    )
}
fn cell(raw: &Option<Vec<u8>>, column: &Column, max: usize) -> NativeResult {
    let Some(raw) = raw else {
        return Ok(Value::Null);
    };
    if column.oid == 17 {
        let hex = raw
            .strip_prefix(b"\\x")
            .ok_or_else(|| kind("invalid PostgreSQL bytea text"))?;
        if hex.len() % 2 != 0 {
            return Err(kind("invalid PostgreSQL bytea text"));
        }
        if hex.len() / 2 > max {
            return Err(limit());
        }
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(kind("invalid PostgreSQL bytea text")),
        };
        let mut out = Vec::with_capacity(hex.len() / 2);
        for pair in hex.chunks_exact(2) {
            out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
        }
        return Ok(Value::Bytes(out.into()));
    }
    if raw.len() > max {
        return Err(limit());
    }
    let text = std::str::from_utf8(raw).map_err(|_| kind("PostgreSQL text is not valid UTF-8"))?;
    Ok(match column.oid {
        16 => match text {
            "t" => Value::Bool(true),
            "f" => Value::Bool(false),
            _ => return Err(kind("invalid PostgreSQL bool")),
        },
        20 | 21 | 23 | 26 => Value::Int(
            text.parse()
                .map_err(|_| kind("invalid PostgreSQL integer"))?,
        ),
        700 | 701 => {
            let n: f64 = text.parse().map_err(|_| kind("invalid PostgreSQL float"))?;
            if !n.is_finite() {
                return Err(kind("non-finite PostgreSQL float is unsupported"));
            }
            Value::Float(n)
        }
        // Text, char, name, varchar, numeric, JSON, temporal, interval and UUID.
        18 | 19 | 25 | 1042 | 1043 | 1700 | 114 | 3802 | 1082 | 1083 | 1114 | 1184 | 1186
        | 1266 | 2950 => Value::String(text.to_owned()),
        _ => {
            return Err(kind(
                "PostgreSQL column type is unsupported; cast it explicitly to text",
            ));
        }
    })
}
pub(crate) fn convert(
    rows: &[Row],
    columns: &[Column],
    selection: Selection,
    budget: Budget,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult {
    if rows.is_empty() {
        return match selection {
            Selection::All => Ok(Value::list(Vec::new())),
            Selection::Row => Ok(Value::Null),
            Selection::Column(_) => Err(NativeError::new(
                "DbEndOfResultsError",
                "no more result rows",
            )),
        };
    }
    let scalar = matches!(selection, Selection::Column(_));
    if !scalar {
        let mut labels = BTreeSet::new();
        for column in columns {
            checkpoint()?;
            if !labels.insert(column.name.as_str()) {
                return Err(kind("duplicate PostgreSQL column label; use SQL aliases"));
            }
        }
    }
    // Size raw payload plus output slots/keys before allocating script containers.
    let mut size = mem::size_of::<Value>();
    for row in rows {
        checkpoint()?;
        if !scalar {
            size = size.saturating_add(mem::size_of::<Value>()).saturating_add(
                columns
                    .len()
                    .saturating_mul(mem::size_of::<(MapKey, Value)>()),
            );
        }
        for (index, (raw, column)) in row.iter().zip(columns).enumerate() {
            if let Selection::Column(wanted) = selection {
                if index != wanted {
                    continue;
                }
            }
            size = size.saturating_add(raw.as_ref().map_or(0, Vec::len));
            if !scalar {
                size = size.saturating_add(column.name.len());
            }
            if size > budget.bytes {
                return Err(limit());
            }
        }
    }
    let mut output = Vec::new();
    for row in rows {
        checkpoint()?;
        if let Selection::Column(index) = selection {
            return cell(&row[index], &columns[index], budget.cell);
        }
        let mut map = Vec::with_capacity(columns.len());
        for (raw, column) in row.iter().zip(columns) {
            checkpoint()?;
            map.push((
                MapKey::String(column.name.clone()),
                cell(raw, column, budget.cell)?,
            ));
        }
        let row = Value::Map(map);
        if matches!(selection, Selection::Row) {
            return Ok(row);
        }
        output.push(row);
    }
    Ok(Value::list(output))
}
