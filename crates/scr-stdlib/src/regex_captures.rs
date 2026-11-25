//! Capture records and monotonic UTF-8-byte to Unicode-scalar positions.
use crate::bytes::{budget, grow, resource};
use regex::Regex;
use scr_runtime::{MapKey, NativeError, NativeResult, RuntimeContext, Value};
use std::mem::size_of;

const SLOT: usize = size_of::<Value>();
const KEY: usize = size_of::<MapKey>();

// Captures within one match can be nested and visited in nonmonotonic group
// order. Sort/deduplicate their endpoints, then scan the input forward once
// across all matches. Temporary position storage is O(groups), not O(input).
#[derive(Default)]
struct ScalarCursor {
    byte: usize,
    scalar: i64,
}
impl ScalarCursor {
    fn advance(
        &mut self,
        input: &str,
        target: usize,
        checkpoint: &mut impl FnMut() -> NativeResult<()>,
    ) -> NativeResult<i64> {
        debug_assert!(target >= self.byte);
        for (index, _) in input[self.byte..target].chars().enumerate() {
            if index % 1024 == 0 {
                checkpoint()?;
            }
            self.scalar = self.scalar.checked_add(1).ok_or_else(|| {
                NativeError::new("RangeError", "match position exceeds i64 range")
            })?;
        }
        self.byte = target;
        Ok(self.scalar)
    }
}
fn copy_text(
    text: &str,
    checkpoint: &mut impl FnMut() -> NativeResult<()>,
) -> NativeResult<String> {
    let mut result = String::new();
    result
        .try_reserve_exact(text.len())
        .map_err(|_| resource("capture text allocation failed"))?;
    let mut at = 0;
    while at < text.len() {
        checkpoint()?;
        let mut end = at.saturating_add(16 * 1024).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        result.push_str(&text[at..end]);
        at = end;
    }
    Ok(result)
}
fn group_size(
    context: &RuntimeContext<'_>,
    total: &mut usize,
    found: Option<regex::Match<'_>>,
) -> NativeResult<()> {
    // Participating group: a three-entry map, string value, and two ints.
    // Nonparticipating group: exactly one null value slot.
    if let Some(found) = found {
        grow(context, total, 4 * SLOT + 3 * KEY + 12)?; // text/start/end key bytes
        grow(context, total, found.len())
    } else {
        grow(context, total, SLOT)
    }
}
fn preflight(
    context: &RuntimeContext<'_>,
    regex: &Regex,
    captures: &regex::Captures<'_>,
    total: &mut usize,
    checkpoint: &mut impl FnMut() -> NativeResult<()>,
) -> NativeResult<usize> {
    // Outer map, groups list, named map, and the two map keys.
    grow(context, total, 3 * SLOT + 2 * KEY + 11)?;
    for found in captures.iter() {
        checkpoint()?;
        group_size(context, total, found)?;
    }
    let mut names = 0;
    for (index, name) in regex.capture_names().enumerate() {
        checkpoint()?;
        if let Some(name) = name {
            grow(context, total, KEY)?;
            grow(context, total, name.len())?;
            group_size(context, total, captures.get(index))?;
            names += 1;
        }
    }
    Ok(names)
}
fn group(
    found: Option<regex::Match<'_>>,
    positions: &[(usize, i64)],
    checkpoint: &mut impl FnMut() -> NativeResult<()>,
) -> NativeResult<Value> {
    let Some(found) = found else {
        return Ok(Value::Null);
    };
    let position = |byte| {
        positions[positions
            .binary_search_by_key(&byte, |p| p.0)
            .expect("capture endpoint")]
        .1
    };
    let mut fields = Vec::new();
    fields
        .try_reserve_exact(3)
        .map_err(|_| resource("capture record allocation failed"))?;
    fields.push((
        MapKey::String("text".into()),
        Value::String(copy_text(found.as_str(), checkpoint)?),
    ));
    fields.push((
        MapKey::String("start".into()),
        Value::Int(position(found.start())),
    ));
    fields.push((
        MapKey::String("end".into()),
        Value::Int(position(found.end())),
    ));
    Ok(Value::Map(fields))
}
fn record(
    input: &str,
    regex: &Regex,
    captures: &regex::Captures<'_>,
    names: usize,
    cursor: &mut ScalarCursor,
    checkpoint: &mut impl FnMut() -> NativeResult<()>,
) -> NativeResult<Value> {
    let mut positions = Vec::new();
    positions
        .try_reserve_exact(
            captures
                .len()
                .checked_mul(2)
                .ok_or_else(|| resource("capture position size overflow"))?,
        )
        .map_err(|_| resource("capture position allocation failed"))?;
    for found in captures.iter().flatten() {
        checkpoint()?;
        positions.push((found.start(), 0));
        positions.push((found.end(), 0));
    }
    checkpoint()?;
    positions.sort_unstable_by_key(|p| p.0);
    positions.dedup_by_key(|p| p.0);
    for (byte, scalar) in &mut positions {
        checkpoint()?;
        *scalar = cursor.advance(input, *byte, checkpoint)?;
    }
    let mut groups = Vec::new();
    groups
        .try_reserve_exact(captures.len())
        .map_err(|_| resource("capture groups allocation failed"))?;
    for found in captures.iter() {
        checkpoint()?;
        groups.push(group(found, &positions, checkpoint)?);
    }
    let mut named = Vec::new();
    named
        .try_reserve_exact(names)
        .map_err(|_| resource("named captures allocation failed"))?;
    for (index, name) in regex.capture_names().enumerate() {
        checkpoint()?;
        if let Some(name) = name {
            named.push((
                MapKey::String(copy_text(name, checkpoint)?),
                group(captures.get(index), &positions, checkpoint)?,
            ));
        }
    }
    let mut fields = Vec::new();
    fields
        .try_reserve_exact(2)
        .map_err(|_| resource("capture result allocation failed"))?;
    fields.push((MapKey::String("groups".into()), Value::List(groups)));
    fields.push((MapKey::String("named".into()), Value::Map(named)));
    Ok(Value::Map(fields))
}
pub(super) fn collect(
    context: &RuntimeContext<'_>,
    input: &str,
    regex: &Regex,
    all: bool,
) -> NativeResult<Value> {
    collect_with_checkpoint(context, input, regex, all, || context.check_cancelled())
}
fn collect_with_checkpoint(
    context: &RuntimeContext<'_>,
    input: &str,
    regex: &Regex,
    all: bool,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<Value> {
    checkpoint()?;
    budget(context, SLOT)?;
    let mut total = if all { SLOT } else { 0 };
    let mut cursor = ScalarCursor::default();
    let mut result = Vec::new();
    for captures in regex.captures_iter(input) {
        checkpoint()?;
        let names = preflight(context, regex, &captures, &mut total, &mut checkpoint)?;
        let value = record(input, regex, &captures, names, &mut cursor, &mut checkpoint)?;
        if !all {
            return Ok(value);
        }
        result
            .try_reserve(1)
            .map_err(|_| resource("capture list allocation failed"))?;
        result.push(value);
    }
    checkpoint()?;
    Ok(if all {
        Value::List(result)
    } else {
        Value::Null
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::sync::{Arc, atomic::AtomicBool};

    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
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
            runtime_identity: None,
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }

    #[test]
    fn exact_minimum_and_nested_budgets_cover_empty_and_unmatched_groups() {
        with_context(|context| {
            for (input, pattern, record_size) in [
                ("", "z", SLOT),
                ("", "", 7 * SLOT + 5 * KEY + 23),
                ("b", "(?P<n>a)?b", 9 * SLOT + 6 * KEY + 25),
                ("é", "(?P<名>é)", 15 * SLOT + 12 * KEY + 56),
            ] {
                let regex = Regex::new(pattern).unwrap();
                for all in [false, true] {
                    context.max_value_bytes = None;
                    let expected = collect(context, input, &regex, all).unwrap();
                    let required = record_size + if all && pattern != "z" { SLOT } else { 0 };
                    context.max_value_bytes = Some(required);
                    assert_eq!(
                        collect(context, input, &regex, all).unwrap(),
                        expected,
                        "{pattern}, all={all}"
                    );
                    context.max_value_bytes = Some(required - 1);
                    assert_eq!(
                        collect(context, input, &regex, all).unwrap_err().kind,
                        "ResourceLimitError",
                        "{pattern}, all={all}"
                    );
                    context.max_value_bytes = None;
                    context.cancellation = Some(Arc::new(AtomicBool::new(true)));
                    assert_eq!(
                        collect(context, input, &regex, all).unwrap_err().kind,
                        "Interrupted"
                    );
                    context.cancellation = None;
                }
            }
        });
    }

    #[test]
    fn cancellation_covers_preflight_positions_text_copy_and_accumulation() {
        with_context(|context| {
            let regex = Regex::new("(?P<outer>(é+))").unwrap();
            let input = format!("{}{} {}", "x".repeat(4096), "é".repeat(8193), "é");
            for all in [false, true] {
                let mut count = 0;
                collect_with_checkpoint(context, &input, &regex, all, || {
                    count += 1;
                    Ok(())
                })
                .unwrap();
                for stop in 0..count {
                    let mut calls = 0;
                    let result = collect_with_checkpoint(context, &input, &regex, all, || {
                        let current = calls;
                        calls += 1;
                        if current == stop {
                            Err(NativeError::new("Interrupted", "execution interrupted"))
                        } else {
                            Ok(())
                        }
                    });
                    assert_eq!(result.unwrap_err().kind, "Interrupted");
                    assert_eq!(calls, stop + 1);
                }
            }
        });
    }

    #[test]
    fn scalar_cursor_scans_incremental_prefixes_and_copy_preserves_utf8_boundaries() {
        let input = "é".repeat(4096);
        let mut cursor = ScalarCursor::default();
        let mut calls = 0;
        for position in 1..=4096 {
            assert_eq!(
                cursor
                    .advance(&input, position * 2, &mut || {
                        calls += 1;
                        Ok(())
                    })
                    .unwrap(),
                position as i64
            );
        }
        assert_eq!(calls, 4096);
        assert_eq!(cursor.byte, input.len());
        let text = format!("{}é{}", "a".repeat(16383), "界".repeat(8193));
        assert_eq!(copy_text(&text, &mut || Ok(())).unwrap(), text);
    }
}
