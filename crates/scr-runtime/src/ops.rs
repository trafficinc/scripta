use std::cmp::Ordering;

use crate::Value;
use scr_ast::{BinaryOp, UnaryOp};

/// Ordinary data errors are typed independently of their diagnostic wording.
/// Resource failures remain fatal and must not become script exceptions.
#[derive(Debug)]
pub(crate) enum Error {
    Type(&'static str),
    Index(&'static str),
    Arithmetic(&'static str),
    Fatal(&'static str),
}
pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) fn boolean(value: Value) -> Result<bool> {
    if let Value::Bool(value) = value {
        Ok(value)
    } else {
        Err(Error::Type(
            "boolean condition requires bool; use bool(value) explicitly",
        ))
    }
}

fn float(value: f64) -> Result<Value> {
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        Err(Error::Arithmetic("floating-point arithmetic overflow"))
    }
}

/// Checked integer arithmetic shared by ordinary and prepared evaluation.
#[inline]
pub(crate) fn integer(op: BinaryOp, a: i64, b: i64) -> Result<i64> {
    use BinaryOp as B;
    if matches!(op, B::Divide | B::Remainder) && b == 0 {
        return Err(Error::Arithmetic("division by zero"));
    }
    match op {
        B::Add => a.checked_add(b),
        B::Subtract => a.checked_sub(b),
        B::Multiply => a.checked_mul(b),
        B::Divide => a.checked_div(b),
        B::Remainder => a.checked_rem(b),
        _ => return Err(Error::Fatal("invalid arithmetic operator")),
    }
    .ok_or(Error::Arithmetic("integer arithmetic overflow"))
}

pub(crate) fn unary(op: UnaryOp, value: Value) -> Result<Value> {
    match (op, value) {
        (UnaryOp::Not, value) => boolean(value).map(|value| Value::Bool(!value)),
        (UnaryOp::Positive, value @ (Value::Int(_) | Value::Float(_))) => Ok(value),
        (UnaryOp::Negative, Value::Int(value)) => value
            .checked_neg()
            .map(Value::Int)
            .ok_or(Error::Arithmetic("integer arithmetic overflow")),
        (UnaryOp::Negative, Value::Float(value)) => float(-value),
        _ => Err(Error::Type("unary arithmetic requires a numeric value")),
    }
}

pub(crate) fn binary(op: BinaryOp, left: Value, right: Value) -> Result<Value> {
    use BinaryOp as B;
    if matches!(op, B::Equal | B::NotEqual) {
        return Ok(Value::Bool(if op == B::Equal {
            left == right
        } else {
            left != right
        }));
    }
    if op == B::Concat {
        return match (left, right) {
            (Value::String(mut left), Value::String(right)) => {
                left.try_reserve(right.len())
                    .map_err(|_| Error::Fatal("string allocation failed"))?;
                left.push_str(&right);
                Ok(Value::String(left))
            }
            _ => Err(Error::Type(
                "concatenation requires two strings; use str(value)",
            )),
        };
    }
    if matches!(op, B::Less | B::LessEqual | B::Greater | B::GreaterEqual) {
        let ordering = match (&left, &right) {
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
            _ => {
                return Err(Error::Type(
                    "comparison requires matching numeric types or two strings; convert explicitly",
                ));
            }
        }
        .ok_or(Error::Arithmetic("cannot compare non-finite numbers"))?;
        return Ok(Value::Bool(match op {
            B::Less => ordering == Ordering::Less,
            B::LessEqual => ordering != Ordering::Greater,
            B::Greater => ordering == Ordering::Greater,
            _ => ordering != Ordering::Less,
        }));
    }
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => integer(op, a, b).map(Value::Int),
        (Value::Float(a), Value::Float(b)) => {
            if matches!(op, B::Divide | B::Remainder) && b == 0.0 {
                return Err(Error::Arithmetic("division by zero"));
            }
            float(match op {
                B::Add => a + b,
                B::Subtract => a - b,
                B::Multiply => a * b,
                B::Divide => a / b,
                B::Remainder => a % b,
                _ => return Err(Error::Fatal("invalid arithmetic operator")),
            })
        }
        _ => Err(Error::Type(
            "arithmetic requires matching numeric types; use int() or float() explicitly",
        )),
    }
}

pub(crate) fn key_matches(key: &crate::MapKey, value: &Value) -> bool {
    IndexKey::from_value(value).is_some_and(|value| value.matches(key))
}

#[derive(Clone, Copy)]
pub(crate) enum IndexKey<'a> {
    Int(i64),
    String(&'a str),
}

impl<'a> IndexKey<'a> {
    fn owned(self) -> crate::MapKey {
        match self {
            Self::Int(value) => crate::MapKey::Int(value),
            Self::String(text) => crate::MapKey::String(text.into()),
        }
    }

    pub fn from_value(value: &'a Value) -> Option<Self> {
        match value {
            Value::Int(value) => Some(Self::Int(*value)),
            Value::String(value) => Some(Self::String(value)),
            _ => None,
        }
    }

    pub fn matches(self, key: &crate::MapKey) -> bool {
        match (self, key) {
            (Self::Int(a), crate::MapKey::Int(b)) => a == *b,
            (Self::String(a), crate::MapKey::String(b)) => a == b,
            _ => false,
        }
    }
}
fn valid_key(key: &Value) -> Result<()> {
    if matches!(key, Value::Int(_) | Value::String(_)) {
        Ok(())
    } else {
        Err(Error::Type("map keys must be int or string"))
    }
}

pub(crate) fn index(value: &Value, key: &Value) -> Result<Value> {
    index_ref(value, key).cloned()
}
pub(crate) fn index_ref<'a>(value: &'a Value, key: &Value) -> Result<&'a Value> {
    if let Some(key) = IndexKey::from_value(key) {
        return index_ref_key(value, key);
    }
    Err(Error::Type(match value {
        Value::List(_) => "list index must be an int",
        Value::Map(_) => "map keys must be int or string",
        _ => "index access requires a list or map",
    }))
}

pub(crate) fn index_ref_key<'a>(value: &'a Value, key: IndexKey<'_>) -> Result<&'a Value> {
    match value {
        Value::List(items) => {
            let IndexKey::Int(key) = key else {
                return Err(Error::Type("list index must be an int"));
            };
            items
                .get(integer_index(key, items.len())?)
                .ok_or(Error::Index("list index out of bounds"))
        }
        Value::Map(entries) => entries
            .iter()
            .find(|(candidate, _)| key.matches(candidate))
            .map(|(_, value)| value)
            .ok_or(Error::Index("map key not found")),
        _ => Err(Error::Type("index access requires a list or map")),
    }
}

fn list_index(value: &Value, len: usize) -> Result<usize> {
    let Value::Int(value) = value else {
        return Err(Error::Type("list index must be an int"));
    };
    integer_index(*value, len)
}

fn integer_index(value: i64, len: usize) -> Result<usize> {
    let index = if value < 0 {
        len as i128 + value as i128
    } else {
        value as i128
    };
    usize::try_from(index).map_err(|_| Error::Index("list index out of bounds"))
}

pub(crate) fn set_index(
    value: &mut Value,
    path: &[Value],
    replacement: Value,
    append: bool,
) -> Result<()> {
    let Some((key, rest)) = path.split_first() else {
        if append {
            let Value::List(items) = value else {
                return Err(Error::Type("list append requires a list"));
            };
            items
                .try_reserve(1)
                .map_err(|_| Error::Fatal("list allocation failed"))?;
            items.push(replacement);
            return Ok(());
        }
        *value = replacement;
        return Ok(());
    };
    match value {
        Value::List(items) => {
            let index = list_index(key, items.len())?;
            if index == items.len() && rest.is_empty() && !append {
                items
                    .try_reserve(1)
                    .map_err(|_| Error::Fatal("list allocation failed"))?;
                items.push(replacement);
                return Ok(());
            }
            let item = items
                .get_mut(index)
                .ok_or(Error::Index("list index out of bounds"))?;
            set_index(item, rest, replacement, append)
        }
        Value::Map(entries) => {
            valid_key(key)?;
            if let Some((_, item)) = entries
                .iter_mut()
                .find(|(candidate, _)| key_matches(candidate, key))
            {
                set_index(item, rest, replacement, append)
            } else if rest.is_empty() && !append {
                entries.push((
                    IndexKey::from_value(key)
                        .expect("validated map key")
                        .owned(),
                    replacement,
                ));
                Ok(())
            } else {
                Err(Error::Index("map key not found"))
            }
        }
        _ => Err(Error::Type("index assignment requires a list or map")),
    }
}

pub(crate) fn set_index_with_hint(
    value: &mut Value,
    path: &[Value],
    replacement: Value,
    append: bool,
    hint: Option<usize>,
) -> Result<()> {
    if let (Some(position), Some((key, rest)), Value::Map(entries)) =
        (hint, path.split_first(), &mut *value)
    {
        if entries
            .get(position)
            .is_some_and(|(stored, _)| key_matches(stored, key))
        {
            return set_index(&mut entries[position].1, rest, replacement, append);
        }
    }
    set_index(value, path, replacement, append)
}

/// Single final index with a borrowed, already typed literal key.
pub(crate) fn set_index_key(
    value: &mut Value,
    key: IndexKey<'_>,
    replacement: Value,
    hint: Option<usize>,
) -> Result<()> {
    match value {
        Value::List(items) => {
            let IndexKey::Int(key) = key else {
                return Err(Error::Type("list index must be an int"));
            };
            let index = integer_index(key, items.len())?;
            if index == items.len() {
                items
                    .try_reserve(1)
                    .map_err(|_| Error::Fatal("list allocation failed"))?;
                items.push(replacement);
            } else {
                *items
                    .get_mut(index)
                    .ok_or(Error::Index("list index out of bounds"))? = replacement;
            }
            Ok(())
        }
        Value::Map(entries) => {
            if let Some(at) = hint.filter(|&at| {
                entries
                    .get(at)
                    .is_some_and(|(stored, _)| key.matches(stored))
            }) {
                entries[at].1 = replacement;
            } else if let Some((_, value)) =
                entries.iter_mut().find(|(stored, _)| key.matches(stored))
            {
                *value = replacement;
            } else {
                entries.push((key.owned(), replacement));
            }
            Ok(())
        }
        _ => Err(Error::Type("index assignment requires a list or map")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_integer_boundaries_have_independent_expected_results() {
        use BinaryOp as B;
        // Differential execution now shares this helper, so retain an explicit
        // arithmetic oracle for signed boundaries, zero division and overflow.
        for (op, a, b, expected) in [
            (B::Add, i64::MAX, 0, i64::MAX),
            (B::Subtract, i64::MIN, 0, i64::MIN),
            (B::Multiply, i64::MIN, 1, i64::MIN),
            (B::Divide, -9, 2, -4),
            (B::Divide, 9, -2, -4),
            (B::Remainder, -9, 2, -1),
            (B::Remainder, 9, -2, 1),
        ] {
            assert_eq!(integer(op, a, b).unwrap(), expected, "{op:?}: {a}, {b}");
        }
        for (op, a, b, message) in [
            (B::Add, i64::MAX, 1, "integer arithmetic overflow"),
            (B::Subtract, i64::MIN, 1, "integer arithmetic overflow"),
            (B::Multiply, i64::MIN, -1, "integer arithmetic overflow"),
            (B::Divide, i64::MIN, -1, "integer arithmetic overflow"),
            (B::Remainder, i64::MIN, -1, "integer arithmetic overflow"),
            (B::Divide, i64::MIN, 0, "division by zero"),
            (B::Remainder, i64::MIN, 0, "division by zero"),
        ] {
            assert!(
                matches!(integer(op, a, b), Err(Error::Arithmetic(actual)) if actual == message),
                "{op:?}: {a}, {b}"
            );
        }
        assert!(matches!(
            integer(B::Concat, 1, 2),
            Err(Error::Fatal("invalid arithmetic operator"))
        ));
    }
}
