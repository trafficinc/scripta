//! Strict result conversion, with a sizing pass before output allocation.
use mysql::{Column, Row, consts::ColumnType};
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
    NativeError::new("DbLimitError", "decoded MySQL result exceeds byte limit")
}
fn add(total: &mut usize, bytes: usize, budget: usize) -> NativeResult<()> {
    *total = total.checked_add(bytes).ok_or_else(limit)?;
    if *total > budget {
        return Err(limit());
    }
    Ok(())
}
enum Cell<'a> {
    Null,
    Int(i64),
    Float(f64),
    Text(&'a str),
    Bytes(&'a [u8]),
    Unsigned(u64),
    Temporal {
        value: &'a mysql::Value,
        date_only: bool,
        precision: u8,
        bytes: usize,
    },
}
impl Cell<'_> {
    fn bytes(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Bytes(b) => b.len(),
            Self::Unsigned(n) => n.ilog10() as usize + 1,
            Self::Temporal { bytes, .. } => *bytes,
            _ => 0,
        }
    }
    fn value(self) -> NativeResult {
        Ok(match self {
            Self::Null => Value::Null,
            Self::Int(n) => Value::Int(n),
            Self::Float(f) => Value::Float(f),
            Self::Text(s) => Value::String(copy_text(s)?),
            Self::Bytes(b) => Value::Bytes(copy_bytes(b)?.into()),
            Self::Unsigned(n) => Value::String(n.to_string()),
            Self::Temporal {
                value,
                date_only,
                precision,
                ..
            } => Value::String(temporal(value, date_only, precision)),
        })
    }
}
fn cell<'a>(value: &'a mysql::Value, column: &Column) -> NativeResult<Cell<'a>> {
    use ColumnType::*;
    use mysql::Value as M;
    if matches!(value, M::NULL) {
        return Ok(Cell::Null);
    }
    Ok(match (column.column_type(), value) {
        (
            MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_INT24
            | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_YEAR,
            M::Int(n),
        ) => Cell::Int(*n),
        (
            MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_INT24
            | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_YEAR,
            M::UInt(n),
        ) => match i64::try_from(*n) {
            Ok(n) => Cell::Int(n),
            Err(_) => Cell::Unsigned(*n),
        },
        (MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE, M::Float(f)) if f.is_finite() => {
            Cell::Float(f64::from(*f))
        }
        (MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE, M::Double(f)) if f.is_finite() => Cell::Float(*f),
        (MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL | MYSQL_TYPE_JSON, M::Bytes(b)) => {
            Cell::Text(std::str::from_utf8(b).map_err(|_| kind("MySQL text is not valid UTF-8"))?)
        }
        (MYSQL_TYPE_BIT, M::Bytes(b)) => Cell::Bytes(b),
        (
            MYSQL_TYPE_VARCHAR
            | MYSQL_TYPE_VAR_STRING
            | MYSQL_TYPE_STRING
            | MYSQL_TYPE_TINY_BLOB
            | MYSQL_TYPE_MEDIUM_BLOB
            | MYSQL_TYPE_LONG_BLOB
            | MYSQL_TYPE_BLOB
            | MYSQL_TYPE_ENUM
            | MYSQL_TYPE_SET,
            M::Bytes(b),
        ) => {
            if column.character_set() == 63 {
                Cell::Bytes(b)
            } else {
                Cell::Text(
                    std::str::from_utf8(b).map_err(|_| kind("MySQL text is not valid UTF-8"))?,
                )
            }
        }
        (
            MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE,
            M::Date(year, month, day, hour, minute, second, micro),
        ) => {
            if *year > 9999
                || *month > 12
                || *day > 31
                || (*hour, *minute, *second, *micro) != (0, 0, 0, 0)
            {
                return Err(kind("invalid MySQL date representation"));
            }
            Cell::Temporal {
                value,
                date_only: true,
                precision: 0,
                bytes: 10,
            }
        }
        (
            MYSQL_TYPE_DATETIME
            | MYSQL_TYPE_TIMESTAMP
            | MYSQL_TYPE_DATETIME2
            | MYSQL_TYPE_TIMESTAMP2,
            M::Date(year, month, day, hour, minute, second, micro),
        ) => {
            if *year > 9999
                || *month > 12
                || *day > 31
                || *hour > 23
                || *minute > 59
                || *second > 59
            {
                return Err(kind("invalid MySQL timestamp representation"));
            }
            let precision = precision(column, *micro)?;
            Cell::Temporal {
                value,
                date_only: false,
                precision,
                bytes: 19 + fraction_size(precision),
            }
        }
        (
            MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2,
            M::Time(negative, days, hour, minute, second, micro),
        ) => {
            let hours = u64::from(*days) * 24 + u64::from(*hour);
            if hours > 838 || *hour > 23 || *minute > 59 || *second > 59 {
                return Err(kind("invalid MySQL time representation"));
            }
            let precision = precision(column, *micro)?;
            Cell::Temporal {
                value,
                date_only: false,
                precision,
                bytes: usize::from(*negative)
                    + if hours >= 100 { 3 } else { 2 }
                    + 6
                    + fraction_size(precision),
            }
        }
        _ => return Err(kind("unsupported or invalid MySQL result type")),
    })
}
fn fraction_size(precision: u8) -> usize {
    if precision == 0 {
        0
    } else {
        usize::from(precision) + 1
    }
}
fn precision(column: &Column, micro: u32) -> NativeResult<u8> {
    let precision = column.decimals();
    if precision > 6 || micro >= 1_000_000 || micro % 10_u32.pow(u32::from(6 - precision)) != 0 {
        return Err(kind("invalid MySQL temporal precision"));
    }
    Ok(precision)
}
fn temporal(value: &mysql::Value, date_only: bool, precision: u8) -> String {
    let (mut output, micro) = match value {
        mysql::Value::Date(y, m, d, h, min, s, micro) => {
            let mut value = format!("{y:04}-{m:02}-{d:02}");
            if !date_only {
                value.push_str(&format!(" {h:02}:{min:02}:{s:02}"));
            }
            (value, *micro)
        }
        mysql::Value::Time(negative, days, h, min, s, micro) => {
            let hours = u64::from(*days) * 24 + u64::from(*h);
            (
                format!(
                    "{}{hours:02}:{min:02}:{s:02}",
                    if *negative { "-" } else { "" }
                ),
                *micro,
            )
        }
        _ => unreachable!("temporal values validated before construction"),
    };
    if precision != 0 {
        let digits = micro / 10_u32.pow(u32::from(6 - precision));
        output.push_str(&format!(
            ".{digits:0width$}",
            width = usize::from(precision)
        ));
    }
    output
}
fn copy_bytes(bytes: &[u8]) -> NativeResult<Vec<u8>> {
    let mut result = Vec::new();
    result.try_reserve_exact(bytes.len()).map_err(|_| limit())?;
    result.extend_from_slice(bytes);
    Ok(result)
}
fn copy_text(text: &str) -> NativeResult<String> {
    let mut result = String::new();
    result.try_reserve_exact(text.len()).map_err(|_| limit())?;
    result.push_str(text);
    Ok(result)
}
fn labels<'a>(
    columns: &'a [Column],
    checkpoint: &mut impl FnMut() -> NativeResult<()>,
) -> NativeResult<Vec<&'a str>> {
    let mut names = Vec::new();
    names
        .try_reserve_exact(columns.len())
        .map_err(|_| limit())?;
    let mut seen = BTreeSet::new();
    for column in columns {
        checkpoint()?;
        let name = std::str::from_utf8(column.name_ref())
            .map_err(|_| kind("MySQL column label is not valid UTF-8"))?;
        if !seen.insert(name) {
            return Err(kind("duplicate MySQL column labels; use SQL aliases"));
        }
        names.push(name);
    }
    Ok(names)
}

pub(crate) fn convert(
    rows: &[Row],
    columns: &[Column],
    selection: Selection,
    budget: Budget,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult {
    checkpoint()?;
    if matches!(selection, Selection::Column(index) if index >= columns.len()) {
        return Err(NativeError::new(
            "IndexError",
            "MySQL column index out of range",
        ));
    }
    if !matches!(selection, Selection::All) && rows.len() > 1 {
        return Err(kind("unexpected MySQL row count"));
    }
    let names = if matches!(selection, Selection::Column(_)) {
        Vec::new()
    } else {
        labels(columns, &mut checkpoint)?
    };
    let mut total = match selection {
        Selection::All => mem::size_of::<Value>(),
        Selection::Row if rows.is_empty() => mem::size_of::<Value>(),
        _ => 0,
    };
    for row in rows {
        checkpoint()?;
        if row.len() != columns.len() {
            return Err(kind("MySQL result metadata does not match row"));
        }
        if !matches!(selection, Selection::Column(_)) {
            add(&mut total, mem::size_of::<Value>(), budget.bytes)?;
        }
        for (index, column) in columns.iter().enumerate() {
            if matches!(selection,Selection::Column(selected) if selected!=index) {
                continue;
            }
            checkpoint()?;
            let value = row
                .as_ref(index)
                .ok_or_else(|| kind("incomplete MySQL row"))?;
            let value = cell(value, column)?;
            if value.bytes() > budget.cell {
                return Err(limit());
            }
            add(&mut total, mem::size_of::<Value>(), budget.bytes)?;
            add(&mut total, value.bytes(), budget.bytes)?;
            if !matches!(selection, Selection::Column(_)) {
                add(&mut total, mem::size_of::<MapKey>(), budget.bytes)?;
                add(&mut total, names[index].len(), budget.bytes)?;
            }
        }
    }
    if total > budget.bytes {
        return Err(limit());
    }
    if matches!(selection, Selection::Column(_)) && rows.is_empty() {
        return Err(NativeError::new(
            "DbEndOfResultsError",
            "no more result rows",
        ));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(rows.len()).map_err(|_| limit())?;
    for row in rows {
        checkpoint()?;
        if let Selection::Column(index) = selection {
            return cell(
                row.as_ref(index)
                    .ok_or_else(|| kind("incomplete MySQL row"))?,
                &columns[index],
            )?
            .value();
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(columns.len())
            .map_err(|_| limit())?;
        for (index, column) in columns.iter().enumerate() {
            checkpoint()?;
            let value = cell(
                row.as_ref(index)
                    .ok_or_else(|| kind("incomplete MySQL row"))?,
                column,
            )?
            .value()?;
            entries.push((MapKey::String(copy_text(names[index])?), value));
        }
        output.push(Value::Map(entries));
    }
    Ok(match selection {
        Selection::All => Value::list(output),
        Selection::Row => output.into_iter().next().unwrap_or(Value::Null),
        Selection::Column(_) => unreachable!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mysql::{Value as M, consts::ColumnType as T};
    fn column(name: &[u8], kind: T, charset: u16, precision: u8) -> Column {
        Column::new(kind)
            .with_name(name)
            .with_character_set(charset)
            .with_decimals(precision)
    }
    fn row(columns: &[Column], values: Vec<M>) -> Row {
        mysql_common::row::new_row(values, columns.to_vec().into())
    }
    fn scalar(kind: T, value: M, charset: u16, precision: u8) -> NativeResult {
        let columns = vec![column(b"v", kind, charset, precision)];
        let rows = vec![row(&columns, vec![value])];
        convert(
            &rows,
            &columns,
            Selection::Column(0),
            Budget {
                bytes: usize::MAX,
                cell: usize::MAX,
            },
            || Ok(()),
        )
    }
    #[test]
    fn text_binary_json_and_exact_numeric_types_are_preserved() {
        assert_eq!(
            scalar(
                T::MYSQL_TYPE_VAR_STRING,
                M::Bytes("🦀".as_bytes().to_vec()),
                45,
                0
            )
            .unwrap(),
            Value::String("🦀".into())
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_BLOB, M::Bytes(vec![0, 255]), 63, 0).unwrap(),
            Value::Bytes(vec![0, 255].into())
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_VAR_STRING, M::Bytes(vec![255]), 45, 0)
                .unwrap_err()
                .kind,
            "DbTypeError"
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_JSON, M::Bytes(b"false".to_vec()), 63, 0).unwrap(),
            Value::String("false".into())
        );
        assert_eq!(
            scalar(
                T::MYSQL_TYPE_NEWDECIMAL,
                M::Bytes(b"12345678901234567890.00100".to_vec()),
                63,
                5
            )
            .unwrap(),
            Value::String("12345678901234567890.00100".into())
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_LONGLONG, M::UInt(u64::MAX), 63, 0).unwrap(),
            Value::String(u64::MAX.to_string())
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_LONGLONG, M::UInt(i64::MAX as u64), 63, 0).unwrap(),
            Value::Int(i64::MAX)
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_TINY, M::Int(0), 63, 0).unwrap(),
            Value::Int(0)
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_GEOMETRY, M::NULL, 63, 0).unwrap(),
            Value::Null
        );
    }
    #[test]
    fn temporal_text_preserves_precision_zero_dates_and_extended_negative_times() {
        assert_eq!(
            scalar(T::MYSQL_TYPE_DATE, M::Date(0, 0, 0, 0, 0, 0, 0), 63, 0).unwrap(),
            Value::String("0000-00-00".into())
        );
        assert_eq!(
            scalar(
                T::MYSQL_TYPE_DATETIME,
                M::Date(2026, 11, 24, 12, 34, 56, 1),
                63,
                6
            )
            .unwrap(),
            Value::String("2026-11-24 12:34:56.000001".into())
        );
        assert_eq!(
            scalar(
                T::MYSQL_TYPE_TIMESTAMP,
                M::Date(2026, 11, 24, 12, 34, 56, 0),
                63,
                3
            )
            .unwrap(),
            Value::String("2026-11-24 12:34:56.000".into())
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_TIME, M::Time(true, 2, 1, 2, 3, 123000), 63, 3).unwrap(),
            Value::String("-49:02:03.123".into())
        );
        assert_eq!(
            scalar(
                T::MYSQL_TYPE_TIME,
                M::Time(true, 34, 22, 59, 59, 999999),
                63,
                6
            )
            .unwrap(),
            Value::String("-838:59:59.999999".into())
        );
        assert_eq!(
            scalar(
                T::MYSQL_TYPE_DATETIME,
                M::Date(2026, 11, 24, 12, 34, 56, 1),
                63,
                3
            )
            .unwrap_err()
            .kind,
            "DbTypeError"
        );
    }
    #[test]
    fn unsupported_types_nonfinite_values_and_bad_labels_fail_without_lossy_conversion() {
        assert_eq!(
            scalar(T::MYSQL_TYPE_GEOMETRY, M::Bytes(vec![1, 2]), 63, 0)
                .unwrap_err()
                .kind,
            "DbTypeError"
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_DOUBLE, M::Double(f64::INFINITY), 63, 0)
                .unwrap_err()
                .kind,
            "DbTypeError"
        );
        assert_eq!(
            scalar(T::MYSQL_TYPE_FLOAT, M::Float(f32::NAN), 63, 0)
                .unwrap_err()
                .kind,
            "DbTypeError"
        );
        let columns = vec![
            column(b"same", T::MYSQL_TYPE_LONG, 63, 0),
            column(b"same", T::MYSQL_TYPE_LONG, 63, 0),
        ];
        let rows = vec![row(&columns, vec![M::Int(1), M::Int(2)])];
        assert_eq!(
            convert(
                &rows,
                &columns,
                Selection::All,
                Budget {
                    bytes: usize::MAX,
                    cell: usize::MAX
                },
                || Ok(())
            )
            .unwrap_err()
            .kind,
            "DbTypeError"
        );
        assert_eq!(
            convert(
                &rows,
                &columns,
                Selection::Column(1),
                Budget {
                    bytes: usize::MAX,
                    cell: usize::MAX
                },
                || Ok(())
            )
            .unwrap(),
            Value::Int(2)
        );
        let bad_columns = vec![column(&[255], T::MYSQL_TYPE_LONG, 63, 0)];
        assert_eq!(
            convert(
                &[],
                &bad_columns,
                Selection::All,
                Budget {
                    bytes: 1000,
                    cell: 1000
                },
                || Ok(())
            )
            .unwrap_err()
            .kind,
            "DbTypeError"
        );
    }
    #[test]
    fn output_and_cell_budgets_cover_exact_and_limit_plus_one_with_utf8_bytes() {
        let columns = vec![column(b"x", T::MYSQL_TYPE_VAR_STRING, 45, 0)];
        let rows = vec![row(&columns, vec![M::Bytes("é".as_bytes().to_vec())])];
        let exact = 2 * mem::size_of::<Value>() + mem::size_of::<MapKey>() + 1 + 2;
        let all_exact = exact + mem::size_of::<Value>();
        for (bytes, succeeds) in [(all_exact, true), (all_exact - 1, false)] {
            let result = convert(
                &rows,
                &columns,
                Selection::All,
                Budget { bytes, cell: 2 },
                || Ok(()),
            );
            if succeeds {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().kind, "DbLimitError");
            }
        }
        assert!(
            convert(
                &rows,
                &columns,
                Selection::Row,
                Budget {
                    bytes: exact,
                    cell: 2
                },
                || Ok(())
            )
            .is_ok()
        );
        assert_eq!(
            convert(
                &rows,
                &columns,
                Selection::Row,
                Budget {
                    bytes: exact - 1,
                    cell: 2
                },
                || Ok(())
            )
            .unwrap_err()
            .kind,
            "DbLimitError"
        );
        assert_eq!(
            convert(
                &rows,
                &columns,
                Selection::Row,
                Budget {
                    bytes: exact,
                    cell: 1
                },
                || Ok(())
            )
            .unwrap_err()
            .kind,
            "DbLimitError"
        );
        let columns = vec![column(b"n", T::MYSQL_TYPE_LONGLONG, 63, 0)];
        let rows = vec![row(&columns, vec![M::UInt(u64::MAX)])];
        assert!(
            convert(
                &rows,
                &columns,
                Selection::Column(0),
                Budget {
                    bytes: mem::size_of::<Value>() + 20,
                    cell: 20
                },
                || Ok(())
            )
            .is_ok()
        );
        assert_eq!(
            convert(
                &rows,
                &columns,
                Selection::Column(0),
                Budget {
                    bytes: mem::size_of::<Value>() + 20,
                    cell: 19
                },
                || Ok(())
            )
            .unwrap_err()
            .kind,
            "DbLimitError"
        );
    }
    #[test]
    fn conversion_checks_cancellation_across_multiple_rows_without_returning_partial_output() {
        let columns = vec![column(b"n", T::MYSQL_TYPE_LONG, 63, 0)];
        let rows = (0..50)
            .map(|n| row(&columns, vec![M::Int(n)]))
            .collect::<Vec<_>>();
        let mut checks = 0;
        let error = convert(
            &rows,
            &columns,
            Selection::All,
            Budget {
                bytes: usize::MAX,
                cell: usize::MAX,
            },
            || {
                checks += 1;
                if checks == 110 {
                    Err(NativeError::new("Interrupted", "execution interrupted"))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, "Interrupted");
    }
}
