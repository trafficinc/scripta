//! Collection helpers return new values and preserve strictly typed map keys.
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{collections::HashMap, mem::size_of};

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
        Value::Bytes(value) => bytes = add(bytes, value.len())?,
        Value::NativeObject(object) => bytes = add(bytes, object.payload_bytes())?,
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

// Borrowed output plans let generated keys and nested results be measured in
// full, at their final depth, before cloning any input payloads.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key<'a> {
    Int(i64),
    Text(&'a str),
}

impl<'a> Key<'a> {
    fn stored(key: &'a MapKey) -> Self {
        match key {
            MapKey::Int(i) => Self::Int(*i),
            MapKey::String(s) => Self::Text(s),
        }
    }

    fn bytes(self) -> NativeResult<usize> {
        let text = match self {
            Self::Text(s) => s.len(),
            Self::Int(_) => 0,
        };
        add(size_of::<MapKey>(), text)
    }

    fn owned(self) -> MapKey {
        match self {
            Self::Int(i) => MapKey::Int(i),
            Self::Text(s) => MapKey::String(s.into()),
        }
    }
}

enum Output<'a> {
    Value(&'a Value),
    Key(Key<'a>),
    List(Vec<Output<'a>>),
    Map(Vec<(Key<'a>, Output<'a>)>),
}

fn reserve<T>(values: &mut Vec<T>) -> NativeResult<()> {
    values
        .try_reserve(1)
        .map_err(|_| resource("collection result allocation failed"))
}

impl Output<'_> {
    fn bytes(&self, context: &RuntimeContext<'_>, depth: usize) -> NativeResult<usize> {
        context.check_cancelled()?;
        if depth > 128 {
            return Err(resource("collection result nesting exceeds 128 levels"));
        }
        let mut bytes = size_of::<Value>();
        match self {
            Self::Value(v) => return measure(context, v, depth),
            Self::Key(k) => bytes = add(bytes, k.bytes()? - size_of::<MapKey>())?,
            Self::List(values) => {
                for value in values {
                    bytes = budget(context, add(bytes, value.bytes(context, depth + 1)?)?)?;
                }
            }
            Self::Map(entries) => {
                for (key, value) in entries {
                    bytes = budget(
                        context,
                        add(add(bytes, key.bytes()?)?, value.bytes(context, depth + 1)?)?,
                    )?;
                }
            }
        }
        budget(context, bytes)
    }

    fn copy(&self, context: &RuntimeContext<'_>) -> NativeResult {
        context.check_cancelled()?;
        Ok(match self {
            Self::Value(v) => (*v).clone(),
            Self::Key(k) => match k.owned() {
                MapKey::Int(i) => Value::Int(i),
                MapKey::String(s) => Value::String(s),
            },
            Self::List(values) => {
                let mut result = Vec::new();
                result
                    .try_reserve_exact(values.len())
                    .map_err(|_| resource("list result allocation failed"))?;
                for value in values {
                    result.push(value.copy(context)?);
                }
                Value::List(result)
            }
            Self::Map(entries) => {
                let mut result = Vec::new();
                result
                    .try_reserve_exact(entries.len())
                    .map_err(|_| resource("map result allocation failed"))?;
                for (key, value) in entries {
                    result.push((key.owned(), value.copy(context)?));
                }
                Value::Map(result)
            }
        })
    }

    fn finish(self, context: &RuntimeContext<'_>) -> NativeResult {
        self.bytes(context, 0)?;
        self.copy(context)
    }
}

fn selected_list<'a>(
    context: &RuntimeContext<'_>,
    values: impl IntoIterator<Item = &'a Value>,
) -> NativeResult<Output<'a>> {
    let mut result = Vec::new();
    let mut bytes = budget(context, size_of::<Value>())?;
    for value in values {
        context.check_cancelled()?;
        bytes = budget(context, add(bytes, measure(context, value, 1)?)?)?;
        reserve(&mut result)?;
        result.push(Output::Value(value));
    }
    Ok(Output::List(result))
}

fn generated_key(value: &Value) -> NativeResult<Key<'_>> {
    match value {
        Value::Int(i) => Ok(Key::Int(*i)),
        Value::String(s) => Ok(Key::Text(s)),
        _ => Err(NativeError::type_error("map keys must be ints or strings")),
    }
}

fn contains(context: &RuntimeContext<'_>, values: &[Value], needle: &Value) -> NativeResult<bool> {
    for value in values {
        context.check_cancelled()?;
        if value == needle {
            return Ok(true);
        }
    }
    Ok(false)
}

fn aggregate(context: &RuntimeContext<'_>, values: &[Value], product: bool) -> NativeResult {
    let mut result = match values.first() {
        Some(Value::Float(_)) => Value::Float(if product { 1.0 } else { 0.0 }),
        _ => Value::Int(i64::from(product)),
    };
    for value in values {
        context.check_cancelled()?;
        result = match (&result, value) {
            (Value::Int(a), Value::Int(b)) => Value::Int(
                (if product {
                    a.checked_mul(*b)
                } else {
                    a.checked_add(*b)
                })
                .ok_or_else(|| {
                    NativeError::new("ArithmeticError", "integer aggregation overflow")
                })?,
            ),
            (Value::Float(a), Value::Float(b)) if b.is_finite() => {
                let value = if product { a * b } else { a + b };
                if !value.is_finite() {
                    return Err(NativeError::new(
                        "ArithmeticError",
                        "nonfinite aggregation result",
                    ));
                }
                Value::Float(value)
            }
            _ => {
                return Err(NativeError::type_error(
                    "aggregation requires only ints or only finite floats",
                ));
            }
        };
    }
    measure(context, &result, 0)?;
    Ok(result)
}

fn column<'a>(
    context: &RuntimeContext<'_>,
    rows: &'a [Value],
    field: &Value,
    index: Option<&Value>,
) -> NativeResult {
    let mut values = Vec::new();
    let mut entries: Vec<(Key<'a>, Output<'a>)> = Vec::new();
    let mut positions = HashMap::new();
    let mut bytes = budget(context, size_of::<Value>())?;
    for row in rows {
        context.check_cancelled()?;
        let Value::Map(row) = row else {
            return Err(NativeError::type_error("column rows must be maps"));
        };
        let value = lookup(context, row, field)?
            .ok_or_else(|| NativeError::new("RangeError", "column field is missing"))?;
        if let Some(index) = index {
            let key =
                generated_key(lookup(context, row, index)?.ok_or_else(|| {
                    NativeError::new("RangeError", "column index field is missing")
                })?)?;
            put_entry(
                context,
                &mut entries,
                &mut positions,
                &mut bytes,
                key,
                Output::Value(value),
            )?;
        } else {
            bytes = budget(context, add(bytes, measure(context, value, 1)?)?)?;
            reserve(&mut values)?;
            values.push(Output::Value(value));
        }
    }
    if index.is_some() {
        Output::Map(entries)
    } else {
        Output::List(values)
    }
    .finish(context)
}

fn put_entry<'a>(
    context: &RuntimeContext<'_>,
    entries: &mut Vec<(Key<'a>, Output<'a>)>,
    positions: &mut HashMap<Key<'a>, usize>,
    minimum_bytes: &mut usize,
    key: Key<'a>,
    value: Output<'a>,
) -> NativeResult<()> {
    context.check_cancelled()?;
    if let Some(&position) = positions.get(&key) {
        entries[position].1 = value;
        return Ok(());
    }
    // Future duplicate keys can replace large values with smaller ones. Count
    // only keys and Value slots here; finish measures the final winning values.
    *minimum_bytes = budget(
        context,
        add(add(*minimum_bytes, key.bytes()?)?, size_of::<Value>())?,
    )?;
    reserve(entries)?;
    positions
        .try_reserve(1)
        .map_err(|_| resource("collection key index allocation failed"))?;
    positions.insert(key, entries.len());
    entries.push((key, value));
    Ok(())
}

fn append_entry<'a>(
    context: &RuntimeContext<'_>,
    entries: &mut Vec<(Key<'a>, Output<'a>)>,
    bytes: &mut usize,
    key: Key<'a>,
    value: Output<'a>,
) -> NativeResult<()> {
    // These entries already have their final values, so check their full size
    // before growing even the borrowed output plan.
    *bytes = budget(
        context,
        add(add(*bytes, key.bytes()?)?, value.bytes(context, 1)?)?,
    )?;
    reserve(entries)?;
    entries.push((key, value));
    Ok(())
}

fn list_extra(name: &str, context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult {
    context.check_cancelled()?;
    if name == "fill" {
        let count = nonnegative(&args[0])?;
        if count == 0 {
            return list_result(context, std::iter::empty());
        }
        // Reject impossible sizes before walking the generated iterator.
        let bytes = measure(context, &args[1], 1)?;
        budget(
            context,
            add(
                size_of::<Value>(),
                bytes
                    .checked_mul(count)
                    .ok_or_else(|| resource("collection result size overflow"))?,
            )?,
        )?;
        return list_result(context, std::iter::repeat_n(&args[1], count));
    }
    let items = list(&args[0]);
    match name {
        "push" => list_result(context, items.iter().chain(std::iter::once(&args[1]))),
        "unshift" => list_result(context, std::iter::once(&args[1]).chain(items.iter())),
        "reverse" => list_result(context, items.iter().rev()),
        "sum" | "product" => aggregate(context, items, name == "product"),
        "column" => column(context, items, &args[1], None),
        "search" => {
            for (i, value) in items.iter().enumerate() {
                context.check_cancelled()?;
                if value == &args[1] {
                    let index =
                        Value::Int(i64::try_from(i).map_err(|_| resource("list index overflow"))?);
                    return Output::Value(&index).finish(context);
                }
            }
            Output::Value(&Value::Null).finish(context)
        }
        "pop" | "shift" => {
            if items.is_empty() {
                return Err(NativeError::new(
                    "RangeError",
                    "cannot remove from an empty list",
                ));
            }
            let (retained, value) = if name == "pop" {
                (&items[..items.len() - 1], &items[items.len() - 1])
            } else {
                (&items[1..], &items[0])
            };
            Output::Map(vec![
                (Key::Text("items"), selected_list(context, retained)?),
                (Key::Text("value"), Output::Value(value)),
            ])
            .finish(context)
        }
        "splice" => {
            let start = nonnegative(&args[1])?;
            let length = nonnegative(&args[2])?;
            if start > items.len() || length > items.len() - start {
                return Err(NativeError::new(
                    "RangeError",
                    "splice range is out of bounds",
                ));
            }
            let replacement = args.get(3).map(list).unwrap_or(&[]);
            Output::Map(vec![
                (
                    Key::Text("items"),
                    selected_list(
                        context,
                        items[..start]
                            .iter()
                            .chain(replacement)
                            .chain(&items[start + length..]),
                    )?,
                ),
                (
                    Key::Text("removed"),
                    selected_list(context, &items[start..start + length])?,
                ),
            ])
            .finish(context)
        }
        "chunk" => {
            let size = nonnegative(&args[1])?;
            if size == 0 {
                return Err(NativeError::new(
                    "RangeError",
                    "chunk size must be positive",
                ));
            }
            let mut chunks = Vec::new();
            let mut bytes = size_of::<Value>();
            for chunk in items.chunks(size) {
                let output = selected_list(context, chunk)?;
                bytes = budget(context, add(bytes, output.bytes(context, 1)?)?)?;
                reserve(&mut chunks)?;
                chunks.push(output);
            }
            Output::List(chunks).finish(context)
        }
        "unique" | "diff" | "intersect" => {
            let mut selected = Vec::new();
            let mut bytes = budget(context, size_of::<Value>())?;
            for (i, value) in items.iter().enumerate() {
                context.check_cancelled()?;
                let keep = if name == "unique" {
                    !contains(context, &items[..i], value)?
                } else {
                    contains(context, list(&args[1]), value)? == (name == "intersect")
                };
                if keep {
                    bytes = budget(context, add(bytes, measure(context, value, 1)?)?)?;
                    reserve(&mut selected)?;
                    selected.push(Output::Value(value));
                }
            }
            Output::List(selected).finish(context)
        }
        _ => unreachable!("registered extra list helper"),
    }
}

fn map_extra(name: &str, context: &RuntimeContext<'_>, args: &[Value]) -> NativeResult {
    context.check_cancelled()?;
    if name == "column" {
        return column(context, list(&args[0]), &args[1], Some(&args[2]));
    }
    let mut result = Vec::new();
    let mut positions = HashMap::new();
    let mut bytes = budget(context, size_of::<Value>())?;
    if name == "combine" {
        let keys = list(&args[0]);
        let values = list(&args[1]);
        if keys.len() != values.len() {
            return Err(NativeError::new(
                "RangeError",
                "combine lists must have equal lengths",
            ));
        }
        for (key, value) in keys.iter().zip(values) {
            context.check_cancelled()?;
            put_entry(
                context,
                &mut result,
                &mut positions,
                &mut bytes,
                generated_key(key)?,
                Output::Value(value),
            )?;
        }
        return Output::Map(result).finish(context);
    }
    let entries = map(&args[0]);
    if name == "search" {
        for (key, value) in entries {
            context.check_cancelled()?;
            if value == &args[1] {
                return Output::Key(Key::stored(key)).finish(context);
            }
        }
        return Output::Value(&Value::Null).finish(context);
    }
    // Borrow keys and values, and retain the ordered input as the source of
    // output order. Hash table iteration never determines collection order.
    let mut right_index = if matches!(name, "merge" | "replace" | "diffKeys" | "intersectKeys") {
        key_index(context, map(&args[1]))?
    } else {
        HashMap::new()
    };
    for (key, value) in entries {
        context.check_cancelled()?;
        if name == "flip" {
            put_entry(
                context,
                &mut result,
                &mut positions,
                &mut bytes,
                generated_key(value)?,
                Output::Key(Key::stored(key)),
            )?;
            continue;
        }
        let mut output = value;
        let keep = match name {
            "merge" | "replace" => {
                output = right_index.remove(&Key::stored(key)).unwrap_or(value);
                true
            }
            "diff" | "intersect" => {
                let mut found = false;
                for (_, candidate) in map(&args[1]) {
                    context.check_cancelled()?;
                    if value == candidate {
                        found = true;
                        break;
                    }
                }
                found == (name == "intersect")
            }
            "diffKeys" | "intersectKeys" => {
                right_index.contains_key(&Key::stored(key)) == (name == "intersectKeys")
            }
            "reverse" => true,
            _ => unreachable!("registered extra map helper"),
        };
        if keep {
            append_entry(
                context,
                &mut result,
                &mut bytes,
                Key::stored(key),
                Output::Value(output),
            )?;
        }
    }
    if name == "merge" {
        for (key, value) in map(&args[1]) {
            context.check_cancelled()?;
            if right_index.contains_key(&Key::stored(key)) {
                append_entry(
                    context,
                    &mut result,
                    &mut bytes,
                    Key::stored(key),
                    Output::Value(value),
                )?;
            }
        }
    }
    if name == "reverse" {
        result.reverse();
    }
    Output::Map(result).finish(context)
}

fn key_index<'a>(
    context: &RuntimeContext<'_>,
    entries: &'a [(MapKey, Value)],
) -> NativeResult<HashMap<Key<'a>, &'a Value>> {
    context.check_cancelled()?;
    let mut index = HashMap::new();
    index
        .try_reserve(entries.len())
        .map_err(|_| resource("collection key index allocation failed"))?;
    for (key, value) in entries {
        context.check_cancelled()?;
        index.insert(Key::stored(key), value);
    }
    Ok(index)
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    use NativeType::{Any, Bool, Float, Int, List, Map, Null, OneOf, String as Text};
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
    let key = NativeType::OneOf(vec![Int, Text]);
    for (name, parameters, optional, returns) in [
        ("push", vec![List, Any], vec![], List),
        ("unshift", vec![List, Any], vec![], List),
        ("pop", vec![List], vec![], Map),
        ("shift", vec![List], vec![], Map),
        ("splice", vec![List, Int, Int], vec![List], Map),
        ("reverse", vec![List], vec![], List),
        ("chunk", vec![List, Int], vec![], List),
        ("fill", vec![Int, Any], vec![], List),
        ("search", vec![List, Any], vec![], OneOf(vec![Int, Null])),
        ("unique", vec![List], vec![], List),
        ("sum", vec![List], vec![], OneOf(vec![Int, Float])),
        ("product", vec![List], vec![], OneOf(vec![Int, Float])),
        ("diff", vec![List, List], vec![], List),
        ("intersect", vec![List, List], vec![], List),
        ("column", vec![List, key.clone()], vec![], List),
    ] {
        runtime.register(
            format!("List::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns).with_optional(optional),
                move |context, args| list_extra(name, context, args),
            ),
        )?;
    }
    for (name, parameters, returns) in [
        ("merge", vec![Map, Map], Map),
        ("replace", vec![Map, Map], Map),
        ("combine", vec![List, List], Map),
        ("flip", vec![Map], Map),
        ("search", vec![Map, Any], OneOf(vec![Int, Text, Null])),
        ("reverse", vec![Map], Map),
        ("diff", vec![Map, Map], Map),
        ("intersect", vec![Map, Map], Map),
        ("diffKeys", vec![Map, Map], Map),
        ("intersectKeys", vec![Map, Map], Map),
        ("column", vec![List, key.clone(), key], Map),
    ] {
        runtime.register(
            format!("Map::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| map_extra(name, context, args),
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
    fn new_helpers_enforce_exact_final_output_budgets_and_cancellation() {
        use std::sync::{Arc, atomic::AtomicBool};
        let sample = Value::Map(vec![
            (MapKey::Int(1), Value::String("é".into())),
            (MapKey::String("1".into()), Value::Null),
        ]);
        let rows = Value::List(vec![Value::Map(vec![
            (MapKey::String("id".into()), Value::String("é".into())),
            (MapKey::String("v".into()), ints(&[1, 2])),
        ])]);
        let text = |s: &str| Value::String(s.into());
        let cases = vec![
            (false, "push", vec![ints(&[1]), ints(&[2])]),
            (false, "unshift", vec![ints(&[1]), ints(&[2])]),
            (
                false,
                "pop",
                vec![Value::List(vec![ints(&[1]), Value::Null])],
            ),
            (
                false,
                "shift",
                vec![Value::List(vec![ints(&[1]), Value::Null])],
            ),
            (
                false,
                "splice",
                vec![ints(&[1, 2]), Value::Int(0), Value::Int(1), ints(&[3, 4])],
            ),
            (
                false,
                "splice",
                vec![ints(&[]), Value::Int(0), Value::Int(0)],
            ),
            (false, "reverse", vec![ints(&[1, 2])]),
            (false, "chunk", vec![ints(&[1, 2, 3]), Value::Int(2)]),
            (false, "chunk", vec![ints(&[]), Value::Int(1)]),
            (false, "fill", vec![Value::Int(2), sample.clone()]),
            (false, "fill", vec![Value::Int(0), sample.clone()]),
            (false, "search", vec![ints(&[1]), Value::Int(1)]),
            (false, "search", vec![ints(&[]), Value::Null]),
            (
                false,
                "unique",
                vec![Value::List(vec![sample.clone(), sample.clone()])],
            ),
            (false, "sum", vec![ints(&[1, 2])]),
            (false, "product", vec![ints(&[1, 2])]),
            (false, "diff", vec![ints(&[1, 2]), ints(&[2])]),
            (false, "intersect", vec![ints(&[1, 2]), ints(&[2])]),
            (false, "column", vec![rows.clone(), text("v")]),
            (true, "merge", vec![sample.clone(), Value::Map(vec![])]),
            (true, "replace", vec![sample.clone(), Value::Map(vec![])]),
            (
                true,
                "combine",
                vec![
                    ints(&[1, 1]),
                    Value::List(vec![text("overwritten"), Value::Null]),
                ],
            ),
            (
                true,
                "flip",
                vec![Value::Map(vec![(
                    MapKey::String("é".into()),
                    text("value"),
                )])],
            ),
            (true, "search", vec![sample.clone(), Value::Null]),
            (true, "search", vec![Value::Map(vec![]), Value::Null]),
            (true, "reverse", vec![sample.clone()]),
            (true, "diff", vec![sample.clone(), Value::Map(vec![])]),
            (true, "intersect", vec![sample.clone(), sample.clone()]),
            (true, "diffKeys", vec![sample.clone(), Value::Map(vec![])]),
            (true, "intersectKeys", vec![sample.clone(), sample]),
            (true, "column", vec![rows, text("v"), text("id")]),
        ];
        for (is_map, name, args) in cases {
            let call = |c: &mut RuntimeContext<'_>| {
                if is_map {
                    map_extra(name, c, &args)
                } else {
                    list_extra(name, c, &args)
                }
            };
            let expected = with_context(None, call).unwrap();
            let required = with_context(None, |c| measure(c, &expected, 0)).unwrap();
            assert_eq!(
                with_context(Some(required), call).unwrap(),
                expected,
                "{is_map}::{name}"
            );
            assert_eq!(
                with_context(Some(required - 1), call).unwrap_err().kind,
                "ResourceLimitError",
                "{is_map}::{name}"
            );
            with_context(None, |c| {
                c.cancellation = Some(Arc::new(AtomicBool::new(true)));
                assert_eq!(call(c).unwrap_err().kind, "Interrupted", "{is_map}::{name}");
            });
        }
    }

    #[test]
    fn overwritten_values_do_not_cause_premature_budget_errors() {
        let large = Value::Bytes(vec![7; 1024].into());
        let integer = Value::Int(1);
        let field = Value::String("v".into());
        let index = Value::String("id".into());
        let rows = Value::List(
            [large.clone(), Value::Null]
                .into_iter()
                .map(|value| {
                    Value::Map(vec![
                        (MapKey::String("id".into()), integer.clone()),
                        (MapKey::String("v".into()), value),
                    ])
                })
                .collect(),
        );
        let required = 2 * size_of::<Value>() + size_of::<MapKey>();
        for (name, args) in [
            (
                "combine",
                vec![ints(&[1, 1]), Value::List(vec![large.clone(), Value::Null])],
            ),
            ("column", vec![rows, field, index]),
            (
                "merge",
                vec![
                    Value::Map(vec![(MapKey::Int(1), large)]),
                    Value::Map(vec![(MapKey::Int(1), Value::Null)]),
                ],
            ),
        ] {
            assert_eq!(
                with_context(Some(required), |c| map_extra(name, c, &args)).unwrap(),
                Value::Map(vec![(MapKey::Int(1), Value::Null)]),
                "{name}"
            );
        }
        let args = [Value::Map(vec![
            (MapKey::String("x".repeat(1024)), integer.clone()),
            (MapKey::String("x".into()), integer),
        ])];
        assert_eq!(
            with_context(Some(required + 1), |c| map_extra("flip", c, &args)).unwrap(),
            Value::Map(vec![(MapKey::Int(1), Value::String("x".into()))])
        );
    }

    #[test]
    fn byte_and_native_object_payloads_count_toward_preflight_limits() {
        #[derive(Debug)]
        struct Payload;
        impl scr_runtime::NativeObject for Payload {
            fn type_name(&self) -> &str {
                "Payload"
            }
            fn payload_bytes(&self) -> usize {
                64
            }
            fn get_property(&self, _: &str) -> Result<Value, String> {
                Err("no properties".into())
            }
        }
        for value in [
            Value::Bytes(vec![7; 64].into()),
            Value::NativeObject(std::rc::Rc::new(Payload)),
        ] {
            let slot = size_of::<Value>();
            let required = 3 * slot + 128;
            let args = [Value::Int(2), value.clone()];
            assert!(with_context(Some(required), |c| list_extra("fill", c, &args)).is_ok());
            assert_eq!(
                with_context(Some(required - 1), |c| list_extra("fill", c, &args))
                    .unwrap_err()
                    .kind,
                "ResourceLimitError"
            );
            assert!(
                with_context(Some(2 * slot + 64), |c| list_result(
                    c,
                    [&value].into_iter()
                ))
                .is_ok()
            );
            assert_eq!(
                with_context(Some(2 * slot + 63), |c| list_result(
                    c,
                    [&value].into_iter()
                ))
                .unwrap_err()
                .kind,
                "ResourceLimitError"
            );
        }
    }

    #[test]
    fn generated_wrappers_count_toward_depth_and_fill_checks_size_before_iteration() {
        with_context(None, |context| {
            let mut nested = Value::Int(1);
            for _ in 0..127 {
                nested = Value::List(vec![nested]);
            }
            assert!(list_extra("fill", context, &[Value::Int(1), nested.clone()]).is_ok());
            assert_eq!(
                list_extra(
                    "chunk",
                    context,
                    &[Value::List(vec![nested.clone()]), Value::Int(1)]
                )
                .unwrap_err()
                .kind,
                "ResourceLimitError"
            );
            assert_eq!(
                list_extra(
                    "pop",
                    context,
                    &[Value::List(vec![nested.clone(), Value::Null])]
                )
                .unwrap_err()
                .kind,
                "ResourceLimitError"
            );
            assert_eq!(
                list_extra(
                    "splice",
                    context,
                    &[Value::List(vec![nested]), Value::Int(0), Value::Int(0)]
                )
                .unwrap_err()
                .kind,
                "ResourceLimitError"
            );
            assert_eq!(
                list_extra("fill", context, &[Value::Int(i64::MAX), Value::Int(0)])
                    .unwrap_err()
                    .kind,
                "ResourceLimitError"
            );
            for name in ["sum", "product"] {
                for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    assert_eq!(
                        list_extra(name, context, &[Value::List(vec![Value::Float(value)])])
                            .unwrap_err()
                            .kind,
                        "TypeError"
                    );
                }
            }
        });
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
