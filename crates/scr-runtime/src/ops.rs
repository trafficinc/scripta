use std::cmp::Ordering;

use crate::Value;
use scr_ast::{BinaryOp, UnaryOp};

pub(crate) fn boolean(value: Value) -> Result<bool, String> {
    if let Value::Bool(value) = value {
        Ok(value)
    } else {
        Err("boolean condition requires bool; use bool(value) explicitly".into())
    }
}

fn float(value: f64) -> Result<Value, String> {
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        Err("floating-point arithmetic overflow".into())
    }
}

pub(crate) fn unary(op: UnaryOp, value: Value) -> Result<Value, String> {
    match (op, value) {
        (UnaryOp::Not, value) => boolean(value).map(|value| Value::Bool(!value)),
        (UnaryOp::Positive, value @ (Value::Int(_) | Value::Float(_))) => Ok(value),
        (UnaryOp::Negative, Value::Int(value)) => value
            .checked_neg()
            .map(Value::Int)
            .ok_or_else(|| "integer arithmetic overflow".into()),
        (UnaryOp::Negative, Value::Float(value)) => float(-value),
        _ => Err("unary arithmetic requires a numeric value".into()),
    }
}

pub(crate) fn binary(op: BinaryOp, left: Value, right: Value) -> Result<Value, String> {
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
                    .map_err(|_| "string allocation failed")?;
                left.push_str(&right);
                Ok(Value::String(left))
            }
            _ => Err("concatenation requires two strings; use str(value)".into()),
        };
    }
    if matches!(op, B::Less | B::LessEqual | B::Greater | B::GreaterEqual) {
        let ordering =
            match (&left, &right) {
                (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
                (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
                (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
                _ => return Err(
                    "comparison requires matching numeric types or two strings; convert explicitly"
                        .into(),
                ),
            }
            .ok_or("cannot compare non-finite numbers")?;
        return Ok(Value::Bool(match op {
            B::Less => ordering == Ordering::Less,
            B::LessEqual => ordering != Ordering::Greater,
            B::Greater => ordering == Ordering::Greater,
            _ => ordering != Ordering::Less,
        }));
    }
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => {
            if matches!(op, B::Divide | B::Remainder) && b == 0 {
                return Err("division by zero".into());
            }
            let value = match op {
                B::Add => a.checked_add(b),
                B::Subtract => a.checked_sub(b),
                B::Multiply => a.checked_mul(b),
                B::Divide => a.checked_div(b),
                B::Remainder => a.checked_rem(b),
                _ => return Err("invalid arithmetic operator".into()),
            };
            value
                .map(Value::Int)
                .ok_or_else(|| "integer arithmetic overflow".into())
        }
        (Value::Float(a), Value::Float(b)) => {
            if matches!(op, B::Divide | B::Remainder) && b == 0.0 {
                return Err("division by zero".into());
            }
            float(match op {
                B::Add => a + b,
                B::Subtract => a - b,
                B::Multiply => a * b,
                B::Divide => a / b,
                B::Remainder => a % b,
                _ => return Err("invalid arithmetic operator".into()),
            })
        }
        _ => Err(
            "arithmetic requires matching numeric types; use int() or float() explicitly".into(),
        ),
    }
}

pub(crate) fn index(value: &Value, key: &Value) -> Result<Value, String> {
    index_ref(value, key).cloned()
}
pub(crate) fn index_ref<'a>(value: &'a Value, key: &Value) -> Result<&'a Value, String> {
    match value {
        Value::List(items) => items
            .get(list_index(key)?)
            .ok_or_else(|| "list index out of bounds".into()),
        Value::Map(entries) => {
            let key = crate::MapKey::from_value(key.clone())?;
            entries
                .iter()
                .find(|(candidate, _)| candidate == &key)
                .map(|(_, value)| value)
                .ok_or_else(|| "map key not found".into())
        }
        _ => Err("index access requires a list or map".into()),
    }
}

fn list_index(value: &Value) -> Result<usize, String> {
    let Value::Int(value) = value else {
        return Err("list index must be a nonnegative int".into());
    };
    usize::try_from(*value).map_err(|_| "list index must be a nonnegative int".into())
}

pub(crate) fn set_index(
    value: &mut Value,
    path: &[Value],
    replacement: Value,
    append: bool,
) -> Result<(), String> {
    let Some((key, rest)) = path.split_first() else {
        if append {
            let Value::List(items) = value else {
                return Err("list append requires a list".into());
            };
            items.try_reserve(1).map_err(|_| "list allocation failed")?;
            items.push(replacement);
            return Ok(());
        }
        *value = replacement;
        return Ok(());
    };
    match value {
        Value::List(items) => {
            let index = list_index(key)?;
            if index == items.len() && rest.is_empty() && !append {
                items.try_reserve(1).map_err(|_| "list allocation failed")?;
                items.push(replacement);
                return Ok(());
            }
            let item = items.get_mut(index).ok_or("list index out of bounds")?;
            set_index(item, rest, replacement, append)
        }
        Value::Map(entries) => {
            let key = crate::MapKey::from_value(key.clone())?;
            if let Some((_, item)) = entries.iter_mut().find(|(candidate, _)| candidate == &key) {
                set_index(item, rest, replacement, append)
            } else if rest.is_empty() && !append {
                entries.push((key, replacement));
                Ok(())
            } else {
                Err("map key not found".into())
            }
        }
        _ => Err("index assignment requires a list or map".into()),
    }
}
