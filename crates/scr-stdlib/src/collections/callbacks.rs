//! Eager transforms and stable key/comparison sorting over collection snapshots.
use scr_runtime::{
    InvocationArgument, InvocationContext, InvocationError, InvocationResult, MapKey, NativeError,
    NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use std::{cmp::Ordering, mem::size_of};

type Result<T> = std::result::Result<T, InvocationError>;
fn resource(context: &InvocationContext<'_>, message: &str) -> InvocationError {
    context.error(NativeError::new("ResourceLimitError", message))
}
fn type_error(context: &InvocationContext<'_>, message: &str) -> InvocationError {
    context.error(NativeError::type_error(message))
}
fn add(context: &InvocationContext<'_>, left: usize, right: usize) -> Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| resource(context, "callback collection size overflow"))
}
fn budget(context: &InvocationContext<'_>, size: usize) -> Result<usize> {
    if context.max_value_bytes().is_some_and(|limit| size > limit) {
        return Err(resource(
            context,
            "callback collection exceeds execution value byte limit",
        ));
    }
    Ok(size)
}
fn key_size(context: &InvocationContext<'_>, key: &MapKey) -> Result<usize> {
    add(
        context,
        size_of::<MapKey>(),
        match key {
            MapKey::String(text) => text.len(),
            _ => 0,
        },
    )
}
fn measure(context: &mut InvocationContext<'_>, value: &Value, depth: usize) -> Result<usize> {
    context.checkpoint()?;
    if depth > 128 {
        return Err(resource(
            context,
            "callback collection nesting exceeds 128 levels",
        ));
    }
    let mut size = size_of::<Value>();
    match value {
        Value::String(text) => size = add(context, size, text.len())?,
        Value::Bytes(bytes) => size = add(context, size, bytes.len())?,
        Value::NativeObject(object) => size = add(context, size, object.payload_bytes())?,
        Value::List(items) => {
            for item in items {
                let next = measure(context, item, depth + 1)?;
                size = budget(context, add(context, size, next)?)?;
            }
        }
        Value::Map(items) => {
            for (key, item) in items {
                let next = measure(context, item, depth + 1)?;
                size = budget(
                    context,
                    add(context, add(context, size, key_size(context, key)?)?, next)?,
                )?;
            }
        }
        _ => {}
    }
    budget(context, size)
}
fn reserve<T>(context: &InvocationContext<'_>, values: &mut Vec<T>, count: usize) -> Result<()> {
    values
        .try_reserve_exact(count)
        .map_err(|_| resource(context, "callback collection allocation failed"))
}
// Filters do not know their final length. Use geometric capacity growth after
// checking each logical output size, rather than reallocating for every item.
fn reserve_next<T>(context: &InvocationContext<'_>, values: &mut Vec<T>) -> Result<()> {
    values
        .try_reserve(1)
        .map_err(|_| resource(context, "callback collection allocation failed"))
}
fn list_transform(
    context: &mut InvocationContext<'_>,
    args: &[Value],
    filter: bool,
) -> InvocationResult {
    let (Value::List(items), callback) = (&args[0], &args[1]) else {
        unreachable!()
    };
    let mut size = budget(context, size_of::<Value>())?;
    let mut result = Vec::new();
    if !filter {
        let slots = items
            .len()
            .checked_mul(size_of::<Value>())
            .ok_or_else(|| resource(context, "callback collection size overflow"))?;
        budget(context, add(context, size, slots)?)?;
        reserve(context, &mut result, items.len())?;
    }
    for item in items {
        context.checkpoint()?;
        let value = context.invoke(callback.clone(), std::slice::from_ref(item))?;
        let output = if filter {
            match value {
                Value::Bool(false) => continue,
                Value::Bool(true) => item,
                _ => return Err(type_error(context, "filter callback must return bool")),
            }
        } else {
            &value
        };
        let cost = measure(context, output, 1)?;
        size = budget(context, add(context, size, cost)?)?;
        if filter {
            reserve_next(context, &mut result)?;
            result.push(output.clone());
        } else {
            result.push(value);
        }
    }
    let result = Value::list(result);
    measure(context, &result, 0)?;
    Ok(result)
}
fn map_transform(
    context: &mut InvocationContext<'_>,
    args: &[Value],
    filter: bool,
) -> InvocationResult {
    let (Value::Map(items), callback) = (&args[0], &args[1]) else {
        unreachable!()
    };
    let mut size = budget(context, size_of::<Value>())?;
    let mut result = Vec::new();
    if !filter {
        let mut minimum = size;
        for (key, _) in items {
            context.checkpoint()?;
            minimum = budget(
                context,
                add(
                    context,
                    add(context, minimum, key_size(context, key)?)?,
                    size_of::<Value>(),
                )?,
            )?;
        }
        reserve(context, &mut result, items.len())?;
    }
    for (key, item) in items {
        context.checkpoint()?;
        let key_value = match key {
            MapKey::Int(value) => Value::Int(*value),
            MapKey::String(value) => Value::String(value.clone()),
        };
        let value = context.invoke_owned(callback.clone(), [item.clone(), key_value])?;
        let output = if filter {
            match value {
                Value::Bool(false) => continue,
                Value::Bool(true) => item,
                _ => return Err(type_error(context, "filter callback must return bool")),
            }
        } else {
            &value
        };
        let cost = measure(context, output, 1)?;
        size = budget(
            context,
            add(context, add(context, size, key_size(context, key)?)?, cost)?,
        )?;
        if filter {
            reserve_next(context, &mut result)?;
            result.push((key.clone(), output.clone()));
        } else {
            result.push((key.clone(), value));
        }
    }
    let result = Value::Map(result);
    measure(context, &result, 0)?;
    Ok(result)
}
fn reduce(context: &mut InvocationContext<'_>, args: &[Value]) -> InvocationResult {
    let callback = &args[1];
    measure(context, &args[2], 0)?;
    let mut accumulator = args[2].clone();
    match &args[0] {
        Value::List(items) => {
            for item in items {
                context.checkpoint()?;
                accumulator =
                    context.invoke_owned(callback.clone(), [accumulator, item.clone()])?;
                measure(context, &accumulator, 0)?;
            }
        }
        Value::Map(items) => {
            for (key, item) in items {
                context.checkpoint()?;
                let key = match key {
                    MapKey::Int(key) => Value::Int(*key),
                    MapKey::String(key) => Value::String(key.clone()),
                };
                accumulator =
                    context.invoke_owned(callback.clone(), [accumulator, item.clone(), key])?;
                measure(context, &accumulator, 0)?;
            }
        }
        _ => unreachable!(),
    }
    Ok(accumulator)
}
fn scalar_kind(value: &Value) -> Option<u8> {
    match value {
        Value::Int(_) => Some(0),
        Value::Float(v) if v.is_finite() => Some(1),
        Value::String(_) => Some(2),
        _ => None,
    }
}
fn scalar_compare(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).expect("finite keys"),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => unreachable!("validated homogeneous keys"),
    }
}

// Match measure's list-root checkpoint and prefix-budget order. Prepare holders
// during that same cooperative walk, without adding logical instruction ticks.
fn prepare_sort_arguments<'a>(
    context: &mut InvocationContext<'_>,
    items: &'a [Value],
    comparison: bool,
) -> Result<Vec<InvocationArgument<'a>>> {
    context.checkpoint()?;
    let mut arguments = Vec::new();
    let mut cache = comparison
        && context.max_value_bytes().is_none()
        && items
            .first()
            .is_some_and(|item| matches!(item, Value::List(_) | Value::Map(_)));
    let mut size = size_of::<Value>();
    for item in items {
        let cost = measure(context, item, 1)?;
        size = budget(context, add(context, size, cost)?)?;
        if cache {
            // Grow only after the corresponding input was traversed, so an
            // early limit/cancellation failure cannot reserve the entire cache.
            if arguments.try_reserve(1).is_ok() {
                arguments.push(InvocationArgument::new(item));
            } else {
                arguments = Vec::new();
                cache = false;
            }
        }
    }
    budget(context, size)?;
    Ok(arguments)
}
// Merge indexes, keeping fallible script calls outside Rust's infallible sort
// interface. Every pass is bounded even if a comparator is inconsistent.
fn sorted_indexes(
    context: &mut InvocationContext<'_>,
    count: usize,
    mut compare: impl FnMut(&mut InvocationContext<'_>, usize, usize) -> Result<Ordering>,
) -> Result<Vec<usize>> {
    let scratch = count
        .checked_mul(2 * size_of::<usize>())
        .ok_or_else(|| resource(context, "sort scratch size overflow"))?;
    budget(context, scratch)?;
    let mut order = Vec::new();
    reserve(context, &mut order, count)?;
    let mut next = Vec::new();
    reserve(context, &mut next, count)?;
    for index in 0..count {
        context.checkpoint()?;
        order.push(index);
        next.push(0);
    }
    let mut width = 1usize;
    while width < count {
        let mut start = 0;
        while start < count {
            let middle = start.saturating_add(width).min(count);
            let end = middle.saturating_add(width).min(count);
            let (mut left, mut right) = (start, middle);
            for slot in &mut next[start..end] {
                context.checkpoint()?;
                if left < middle
                    && (right == end
                        || compare(context, order[left], order[right])? != Ordering::Greater)
                {
                    *slot = order[left];
                    left += 1;
                } else {
                    *slot = order[right];
                    right += 1;
                }
            }
            start = end;
        }
        std::mem::swap(&mut order, &mut next);
        width = width.saturating_mul(2);
    }
    Ok(order)
}
fn sort(context: &mut InvocationContext<'_>, args: &[Value], by_key: bool) -> InvocationResult {
    let (Value::List(items), callback) = (&args[0], &args[1]) else {
        unreachable!()
    };
    let descending = matches!(args.get(2), Some(Value::Bool(true)));
    let arguments = if !by_key
        && context.max_value_bytes().is_none()
        && items
            .first()
            .is_some_and(|item| matches!(item, Value::List(_) | Value::Map(_)))
    {
        prepare_sort_arguments(context, items, true)?
    } else {
        measure(context, &args[0], 0)?;
        Vec::new()
    };
    // Preflight scratch before invoking any key/comparison callback.
    let scratch = items
        .len()
        .checked_mul(2 * size_of::<usize>())
        .ok_or_else(|| resource(context, "sort scratch size overflow"))?;
    budget(context, scratch)?;
    let order = if by_key {
        let slots = items
            .len()
            .checked_mul(size_of::<Value>())
            .ok_or_else(|| resource(context, "sort key size overflow"))?;
        budget(context, add(context, size_of::<Value>(), slots)?)?;
        let mut keys = Vec::new();
        reserve(context, &mut keys, items.len())?;
        let mut size = size_of::<Value>();
        let mut kind = None;
        for item in items {
            let key = context.invoke(callback.clone(), std::slice::from_ref(item))?;
            let current = scalar_kind(&key).ok_or_else(|| {
                type_error(
                    context,
                    "sortBy keys must be homogeneous ints, finite floats, or strings",
                )
            })?;
            if kind.is_some_and(|kind| kind != current) {
                return Err(type_error(
                    context,
                    "sortBy keys must be homogeneous ints, finite floats, or strings",
                ));
            }
            kind = Some(current);
            let cost = measure(context, &key, 1)?;
            size = budget(context, add(context, size, cost)?)?;
            keys.push(key);
        }
        sorted_indexes(context, items.len(), |_, left, right| {
            let order = scalar_compare(&keys[left], &keys[right]);
            Ok(if descending { order.reverse() } else { order })
        })?
    } else {
        sorted_indexes(context, items.len(), |context, left, right| {
            let value = if arguments.is_empty() {
                context.invoke_owned(
                    callback.clone(),
                    [items[left].clone(), items[right].clone()],
                )?
            } else {
                context.invoke_arguments(callback.clone(), [&arguments[left], &arguments[right]])?
            };
            let Value::Int(value) = value else {
                return Err(type_error(context, "sortWith comparator must return int"));
            };
            let order = value.cmp(&0);
            Ok(if descending { order.reverse() } else { order })
        })?
    };
    drop(arguments);
    let mut size = size_of::<Value>();
    for &index in &order {
        let cost = measure(context, &items[index], 1)?;
        size = budget(context, add(context, size, cost)?)?;
    }
    let mut output = Vec::new();
    reserve(context, &mut output, items.len())?;
    for index in order {
        context.checkpoint()?;
        output.push(items[index].clone());
    }
    Ok(Value::list(output))
}
pub(super) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Any, Bool, Callable, List, Map};
    for (namespace, kind) in [("List", List), ("Map", Map)] {
        for name in ["map", "filter", "reduce"] {
            let mut parameters = vec![kind.clone(), Callable];
            if name == "reduce" {
                parameters.push(Any);
            }
            let returns = if name == "reduce" { Any } else { kind.clone() };
            runtime.register(
                format!("{namespace}::{name}"),
                NativeFunction::reentrant(
                    NativeSignature::new(parameters, returns),
                    move |context, args| match (namespace, name) {
                        (_, "reduce") => reduce(context, args),
                        ("List", _) => list_transform(context, args, name == "filter"),
                        _ => map_transform(context, args, name == "filter"),
                    },
                ),
            )?;
        }
    }
    for name in ["sortBy", "sortWith"] {
        runtime.register(
            format!("List::{name}"),
            NativeFunction::reentrant(
                NativeSignature::new(vec![List, Callable], List).with_optional(vec![Bool]),
                move |context, args| sort(context, args, name == "sortBy"),
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
