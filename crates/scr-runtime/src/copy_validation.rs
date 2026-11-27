//! Validation specialized for discarded, plain variable-to-variable list copies.
use crate::{Expr, MapKey, Place, Root, Runtime, ScriptOutput, Value};
use scr_diagnostics::Span;

pub(super) fn assignment(
    runtime: &mut Runtime,
    target: &Expr,
    rhs: &Expr,
    output: &mut dyn ScriptOutput,
) -> crate::Result<()> {
    let place = runtime.place(target, output)?;
    let value = runtime.evaluate(rhs, output)?;
    assign(runtime, &place, value, target.span)
}

fn assign(runtime: &mut Runtime, place: &Place<'_>, value: Value, span: Span) -> crate::Result<()> {
    let Value::List(values) = &value else {
        return runtime.write_place(place, value, span);
    };
    let Root::Variable { scope, name, slot } = &place.root else {
        return runtime.write_place(place, value, span);
    };
    if !place.path.is_empty() || place.append {
        return runtime.write_place(place, value, span);
    }
    let Some(result) = validate(values, runtime.options.max_value_bytes) else {
        return runtime.write_place(place, value, span);
    };
    #[cfg(test)]
    {
        runtime.copy_validations_executed += 1;
    }
    runtime.checked(result, span)?;
    // Match the plain-variable commit in write_place, including hint invalidation.
    runtime.map_indexes.get_mut().invalidate(*scope, name);
    let variables = &mut runtime.environments.scope_mut(*scope).variables;
    if let Some(destination) = variables.get_slot_mut(*slot, name) {
        *destination = value;
    } else {
        variables.insert((*name).into(), value);
    }
    Ok(())
}

fn primitive(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Int(_))
}

pub(super) fn validate(values: &[Value], limit: Option<usize>) -> Option<Result<(), String>> {
    if values.len() < 32 || !values.last().is_some_and(primitive) {
        return None;
    }
    Some(measure(values, limit).map(|_| ()))
}

// A distinct borrowed key keeps this table's generated code independent of
// the general validator's HashSet<&MapKey>, without cloning key text.
#[derive(PartialEq, Eq, Hash)]
enum CopyKey<'a> {
    Int(i64),
    Text(&'a str),
}
impl<'a> From<&'a MapKey> for CopyKey<'a> {
    fn from(key: &'a MapKey) -> Self {
        match key {
            MapKey::Int(value) => Self::Int(*value),
            MapKey::String(value) => Self::Text(value),
        }
    }
}

fn measure(values: &[Value], limit: Option<usize>) -> Result<usize, String> {
    let slot = size_of::<Value>();
    if limit.is_some_and(|limit| slot > limit) {
        return Err("execution value byte limit exceeded".into());
    }
    let prefix = values
        .iter()
        .rposition(|value| !primitive(value))
        .map_or(0, |i| i + 1);
    let mut bytes = (values.len() - prefix)
        .checked_add(1)
        .and_then(|slots| slots.checked_mul(slot))
        .ok_or("value size overflow")?;
    if limit.is_some_and(|limit| bytes > limit) {
        return Err("execution value byte limit exceeded".into());
    }
    if prefix == 0 {
        return Ok(bytes);
    }
    // Repeat the general validator's node rules with the suffix already counted.
    let mut pending: Vec<_> = values[..prefix].iter().map(|value| (value, 1)).collect();
    while let Some((value, depth)) = pending.pop() {
        if depth > crate::MAX_VALUE_DEPTH {
            return Err("value nesting exceeds 128 levels".into());
        }
        let mut add = slot;
        match value {
            Value::Float(n) if !n.is_finite() => {
                return Err("floating-point value must be finite".into());
            }
            Value::String(s) => add = add.checked_add(s.len()).ok_or("value size overflow")?,
            Value::Bytes(b) => add = add.checked_add(b.len()).ok_or("value size overflow")?,
            Value::NativeObject(object) => {
                add = add
                    .checked_add(object.payload_bytes())
                    .ok_or("value size overflow")?;
            }
            Value::List(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
            Value::Map(entries) => {
                if crate::value::small_map_validation(entries.len()) {
                    add = measure_small_map(entries)?;
                    pending.extend(entries.iter().map(|(_, value)| (value, depth + 1)));
                } else {
                    let mut keys = std::collections::HashSet::with_capacity(entries.len());
                    for (key, value) in entries {
                        if !keys.insert(CopyKey::from(key)) {
                            return Err("duplicate host map key".into());
                        }
                        add = add
                            .checked_add(size_of::<MapKey>())
                            .ok_or("value size overflow")?;
                        if let MapKey::String(key) = key {
                            add = add.checked_add(key.len()).ok_or("value size overflow")?;
                        }
                        pending.push((value, depth + 1));
                    }
                }
            }
            _ => {}
        }
        bytes = bytes.checked_add(add).ok_or("value size overflow")?;
        if limit.is_some_and(|limit| bytes > limit) {
            return Err("execution value byte limit exceeded".into());
        }
    }
    Ok(bytes)
}

// Keep the original hashed loop in the visitor; isolate only the bounded
// pairwise path so scalar/list traversal does not inherit its working state.
#[inline(never)]
fn measure_small_map(entries: &[(MapKey, Value)]) -> Result<usize, String> {
    let mut add = size_of::<Value>();
    for (index, (key, _)) in entries.iter().enumerate() {
        if entries[..index].iter().any(|(seen, _)| seen == key) {
            return Err("duplicate host map key".into());
        }
        add = add
            .checked_add(size_of::<MapKey>())
            .ok_or("value size overflow")?;
        if let MapKey::String(key) = key {
            add = add.checked_add(key.len()).ok_or("value size overflow")?;
        }
    }
    Ok(add)
}
