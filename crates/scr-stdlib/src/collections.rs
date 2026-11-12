//! Collection helpers return new values and preserve strictly typed map keys.
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::mem::size_of;

fn resource(message: &str) -> NativeError {
    NativeError::new("ResourceLimitError", message)
}

fn add(left: usize, right: usize) -> NativeResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| resource("collection result size overflow"))
}

fn budget(context: &RuntimeContext<'_>, bytes: usize) -> NativeResult<usize> {
    if context.max_value_bytes.is_some_and(|limit| bytes > limit) {
        return Err(resource(
            "collection result exceeds execution value byte limit",
        ));
    }
    Ok(bytes)
}

fn key_bytes(key: &MapKey) -> NativeResult<usize> {
    add(
        size_of::<MapKey>(),
        match key {
            MapKey::String(s) => s.len(),
            _ => 0,
        },
    )
}

// Match the runtime's logical Value accounting at each output position. Inputs
// have already been validated by the runtime; no duplicate-key table is needed.
fn measure(context: &RuntimeContext<'_>, value: &Value, depth: usize) -> NativeResult<usize> {
    context.check_cancelled()?;
    if depth > 128 {
        return Err(resource("collection result nesting exceeds 128 levels"));
    }
    let mut bytes = size_of::<Value>();
    match value {
        Value::String(s) => bytes = add(bytes, s.len())?,
        Value::List(values) => {
            for value in values {
                bytes = budget(context, add(bytes, measure(context, value, depth + 1)?)?)?;
            }
        }
        Value::Map(entries) => {
            for (key, value) in entries {
                bytes = budget(
                    context,
                    add(
                        add(bytes, key_bytes(key)?)?,
                        measure(context, value, depth + 1)?,
                    )?,
                )?;
            }
        }
        _ => {}
    }
    budget(context, bytes)
}

fn list_result<'a>(
    context: &RuntimeContext<'_>,
    values: impl Iterator<Item = &'a Value> + Clone,
) -> NativeResult {
    let mut bytes = budget(context, size_of::<Value>())?;
    let mut count = 0;
    for value in values.clone() {
        bytes = budget(context, add(bytes, measure(context, value, 1)?)?)?;
        count = add(count, 1)?;
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| resource("list result allocation failed"))?;
    for value in values {
        context.check_cancelled()?;
        result.push(value.clone());
    }
    Ok(Value::List(result))
}

fn map_result<'a>(
    context: &RuntimeContext<'_>,
    entries: impl Iterator<Item = &'a (MapKey, Value)> + Clone,
) -> NativeResult {
    let mut bytes = budget(context, size_of::<Value>())?;
    let mut count = 0;
    for (key, value) in entries.clone() {
        bytes = budget(
            context,
            add(add(bytes, key_bytes(key)?)?, measure(context, value, 1)?)?,
        )?;
        count = add(count, 1)?;
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| resource("map result allocation failed"))?;
    for entry in entries {
        context.check_cancelled()?;
        result.push(entry.clone());
    }
    Ok(Value::Map(result))
}

fn key_matches(key: &MapKey, value: &Value) -> bool {
    match (key, value) {
        (MapKey::Int(a), Value::Int(b)) => a == b,
        (MapKey::String(a), Value::String(b)) => a == b,
        _ => false,
    }
}

fn lookup<'a>(
    context: &RuntimeContext<'_>,
    entries: &'a [(MapKey, Value)],
    key: &Value,
) -> NativeResult<Option<&'a Value>> {
    for (candidate, value) in entries {
        context.check_cancelled()?;
        if key_matches(candidate, key) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn nonnegative(value: &Value) -> NativeResult<usize> {
    let Value::Int(value) = value else {
        unreachable!("validated index")
    };
    usize::try_from(*value)
        .map_err(|_| NativeError::new("RangeError", "list offsets must be nonnegative ints"))
}

fn list(value: &Value) -> &[Value] {
    let Value::List(values) = value else {
        unreachable!("validated list")
    };
    values
}

fn map(value: &Value) -> &[(MapKey, Value)] {
    let Value::Map(entries) = value else {
        unreachable!("validated map")
    };
    entries
}

fn sort(context: &RuntimeContext<'_>, values: &[Value], descending: bool) -> NativeResult {
    let valid = match values.first() {
        None => true,
        Some(Value::Int(_)) => values.iter().all(|v| matches!(v, Value::Int(_))),
        Some(Value::Float(_)) => values
            .iter()
            .all(|v| matches!(v, Value::Float(f) if f.is_finite())),
        Some(Value::String(_)) => values.iter().all(|v| matches!(v, Value::String(_))),
        _ => false,
    };
    if !valid {
        return Err(NativeError::type_error(
            "List::sort requires only ints, only finite floats, or only strings",
        ));
    }
    let Value::List(mut result) = list_result(context, values.iter())? else {
        unreachable!()
    };
    result.sort_by(|a, b| {
        let order = match (a, b) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap(),
            (Value::String(a), Value::String(b)) => a.cmp(b),
            _ => unreachable!("validated homogeneous sort"),
        };
        if descending { order.reverse() } else { order }
    });
    context.check_cancelled()?;
    Ok(Value::List(result))
}

fn list_call(name: &str, context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult {
    let items = list(&args[0]);
    match name {
        "remove" => {
            let index = nonnegative(&args[1])?;
            if index >= items.len() {
                return Err(NativeError::new(
                    "RangeError",
                    "list removal index is out of bounds",
                ));
            }
            list_result(
                context,
                items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| (i != index).then_some(v)),
            )
        }
        "slice" => {
            let start = nonnegative(&args[1])?;
            if start > items.len() {
                return Err(NativeError::new(
                    "RangeError",
                    "list slice start is out of bounds",
                ));
            }
            let length = args
                .get(2)
                .map(nonnegative)
                .transpose()?
                .unwrap_or(items.len() - start);
            if length > items.len() - start {
                return Err(NativeError::new(
                    "RangeError",
                    "list slice length is out of bounds",
                ));
            }
            list_result(context, items[start..start + length].iter())
        }
        "concat" => list_result(context, items.iter().chain(list(&args[1]).iter())),
        "contains" => {
            for value in items {
                context.check_cancelled()?;
                if value == &args[1] {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        "sort" => sort(
            context,
            items,
            matches!(args.get(1), Some(Value::Bool(true))),
        ),
        _ => unreachable!("registered list helper"),
    }
}

fn map_call(name: &str, context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult {
    if name == "create" {
        budget(context, size_of::<Value>())?;
        return Ok(Value::Map(Vec::new()));
    }
    let entries = map(&args[0]);
    match name {
        "has" => Ok(Value::Bool(lookup(context, entries, &args[1])?.is_some())),
        "get" => {
            let value = lookup(context, entries, &args[1])?
                .unwrap_or_else(|| args.get(2).unwrap_or(&Value::Null));
            measure(context, value, 0)?;
            Ok(value.clone())
        }
        "values" => list_result(context, entries.iter().map(|(_, v)| v)),
        "keys" => {
            let mut bytes = budget(context, size_of::<Value>())?;
            for (key, _) in entries {
                context.check_cancelled()?;
                let text_bytes = match key {
                    MapKey::String(s) => s.len(),
                    _ => 0,
                };
                bytes = budget(context, add(add(bytes, size_of::<Value>())?, text_bytes)?)?;
            }
            let mut result = Vec::new();
            result
                .try_reserve_exact(entries.len())
                .map_err(|_| resource("map keys allocation failed"))?;
            for (key, _) in entries {
                context.check_cancelled()?;
                result.push(match key {
                    MapKey::Int(v) => Value::Int(*v),
                    MapKey::String(v) => Value::String(v.clone()),
                });
            }
            Ok(Value::List(result))
        }
        "remove" => map_result(
            context,
            entries
                .iter()
                .filter(|(key, _)| !key_matches(key, &args[1])),
        ),
        _ => unreachable!("registered map helper"),
    }
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Any, Bool, Int, List, Map, String as Text};
    for (name, parameters, optional, returns) in [
        ("remove", vec![List, Int], vec![], List),
        ("slice", vec![List, Int], vec![Int], List),
        ("concat", vec![List, List], vec![], List),
        ("contains", vec![List, Any], vec![], Bool),
        ("sort", vec![List], vec![Bool], List),
    ] {
        runtime.register(
            format!("List::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns).with_optional(optional),
                move |context, args| list_call(name, context, args),
            ),
        )?;
    }
    for (name, parameters, optional, returns) in [
        ("create", vec![], vec![], Map),
        (
            "has",
            vec![Map, NativeType::OneOf(vec![Int, Text])],
            vec![],
            Bool,
        ),
        (
            "get",
            vec![Map, NativeType::OneOf(vec![Int, Text])],
            vec![Any],
            Any,
        ),
        ("keys", vec![Map], vec![], List),
        ("values", vec![Map], vec![], List),
        (
            "remove",
            vec![Map, NativeType::OneOf(vec![Int, Text])],
            vec![],
            Map,
        ),
    ] {
        runtime.register(
            format!("Map::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns).with_optional(optional),
                move |context, args| map_call(name, context, args),
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;

    fn with_context<T>(limit: Option<usize>, test: impl FnOnce(&mut RuntimeContext<'_>) -> T) -> T {
        let mut settings = ContextOptions::default();
        let mut input = std::io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: limit,
        })
    }

    fn ints(values: &[i64]) -> Value {
        Value::List(values.iter().copied().map(Value::Int).collect())
    }

    #[test]
    fn collection_outputs_account_for_nested_values_keys_and_empty_results() {
        let slot = size_of::<Value>();
        let key = size_of::<MapKey>();
        let cases = [
            (
                "List",
                "remove",
                vec![ints(&[1, 2]), Value::Int(0)],
                2 * slot,
            ),
            (
                "List",
                "slice",
                vec![
                    Value::List(vec![Value::Int(1), Value::String("é".into())]),
                    Value::Int(1),
                ],
                2 * slot + 2,
            ),
            ("List", "concat", vec![ints(&[1]), ints(&[2])], 3 * slot),
            ("List", "concat", vec![ints(&[]), ints(&[])], slot),
            ("List", "sort", vec![ints(&[3, 1, 2])], 4 * slot),
            ("Map", "create", vec![], slot),
            ("Map", "get", vec![Value::Map(vec![]), Value::Int(1)], slot),
            (
                "Map",
                "get",
                vec![
                    Value::Map(vec![]),
                    Value::Int(1),
                    Value::String("abc".into()),
                ],
                slot + 3,
            ),
            (
                "Map",
                "keys",
                vec![Value::Map(vec![
                    (MapKey::Int(1), Value::Null),
                    (MapKey::String("é".into()), Value::Null),
                ])],
                3 * slot + 2,
            ),
            (
                "Map",
                "values",
                vec![Value::Map(vec![
                    (MapKey::Int(1), ints(&[1])),
                    (MapKey::Int(2), Value::Int(2)),
                ])],
                4 * slot,
            ),
            (
                "Map",
                "remove",
                vec![
                    Value::Map(vec![
                        (MapKey::Int(1), Value::Null),
                        (MapKey::Int(2), Value::Int(2)),
                    ]),
                    Value::Int(1),
                ],
                2 * slot + key,
            ),
            (
                "Map",
                "remove",
                vec![
                    Value::Map(vec![(MapKey::String("é".into()), ints(&[1]))]),
                    Value::Int(1),
                ],
                3 * slot + key + 2,
            ),
        ];
        for (namespace, name, args, required) in cases {
            let call = |c: &mut RuntimeContext<'_>| match namespace {
                "List" => list_call(name, c, &args),
                _ => map_call(name, c, &args),
            };
            let expected = with_context(None, call).unwrap();
            assert_eq!(
                with_context(Some(required), call).unwrap(),
                expected,
                "{namespace}::{name}"
            );
            let error = with_context(Some(required - 1), call).unwrap_err();
            assert_eq!(error.kind, "ResourceLimitError", "{namespace}::{name}");
            assert!(error.message.contains("byte limit"));
        }
    }

    #[test]
    fn depth_overflow_and_cancellation_fail_before_copying_outputs() {
        use std::sync::{Arc, atomic::AtomicBool};
        with_context(None, |context| {
            assert_eq!(add(usize::MAX, 1).unwrap_err().kind, "ResourceLimitError");
            let mut nested = Value::Int(1);
            for _ in 0..128 {
                nested = Value::List(vec![nested]);
            }
            assert!(measure(context, &nested, 0).is_ok());
            assert!(
                list_result(context, [&nested].into_iter())
                    .unwrap_err()
                    .message
                    .contains("nesting")
            );
            assert_eq!(
                sort(context, &[Value::Float(f64::NAN)], false)
                    .unwrap_err()
                    .kind,
                "TypeError"
            );
            context.cancellation = Some(Arc::new(AtomicBool::new(true)));
            for result in [
                list_call("concat", context, &[ints(&[1]), ints(&[])]),
                list_call("contains", context, &[ints(&[1]), Value::Int(1)]),
                map_call(
                    "keys",
                    context,
                    &[Value::Map(vec![(MapKey::Int(1), Value::Null)])],
                ),
                map_call("get", context, &[Value::Map(vec![]), Value::Int(1)]),
                map_call(
                    "remove",
                    context,
                    &[
                        Value::Map(vec![(MapKey::Int(1), Value::Null)]),
                        Value::Int(2),
                    ],
                ),
            ] {
                assert_eq!(result.unwrap_err().kind, "Interrupted");
            }
        });
    }
}
