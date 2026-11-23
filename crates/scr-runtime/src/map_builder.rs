//! Transient positions for large literals; public map storage stays an ordered Vec.
use crate::*;
use std::collections::HashMap;

const MIN_ENTRIES: usize = 96;
const MAX_ENTRIES: usize = 4096;
const MAX_KEY_BYTES: usize = 256 * 1024;
const PREFIX: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key<'a> {
    Int(i64),
    Text(&'a str),
}
impl<'a> Key<'a> {
    fn literal(expr: &'a Expr) -> Option<Self> {
        match &expr.kind {
            ExprKind::Literal(Literal::Int(value)) => Some(Self::Int(*value)),
            ExprKind::Literal(Literal::String(value)) => Some(Self::Text(value)),
            _ => None,
        }
    }
    fn bytes(self) -> usize {
        match self {
            Self::Int(_) => 0,
            Self::Text(value) => value.len(),
        }
    }
}

#[inline]
pub(super) fn candidate_size(length: usize) -> bool {
    (MIN_ENTRIES..=MAX_ENTRIES).contains(&length)
}

#[inline(never)]
pub(super) fn eligible(entries: &[(Expr, Expr)]) -> bool {
    if !candidate_size(entries.len()) {
        return false;
    }
    let mut bytes = 0usize;
    for (key, _) in entries {
        let Some(key) = Key::literal(key) else {
            return false;
        };
        let Some(next) = bytes.checked_add(key.bytes()) else {
            return false;
        };
        if next > MAX_KEY_BYTES {
            return false;
        }
        bytes = next;
    }
    true
}

impl Runtime {
    #[inline(never)]
    pub(super) fn map_literal(
        &mut self,
        entries: &[(Expr, Expr)],
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let indexed = candidate_size(entries.len()) && eligible(entries);
        #[cfg(test)]
        let indexed = indexed && !self.disable_map_literal_index;
        if indexed {
            return self.indexed_map_literal(entries, span, output);
        }
        let mut map = Vec::new();
        for (key, value) in entries {
            let key_value = self.evaluate(key, output)?;
            let key = self.operation(
                MapKey::from_value(key_value)
                    .map_err(|_| ops::Error::Type("map keys must be int or string")),
                key.span,
            )?;
            let value = self.evaluate(value, output)?;
            if let Some((_, old)) = map.iter_mut().find(|(candidate, _)| candidate == &key) {
                *old = value;
            } else {
                map.push((key, value));
            }
        }
        let value = Value::Map(map);
        self.validate_value(&value, 0, span)?;
        Ok(value)
    }
    // Keep the transient table out of the general evaluator's hot stack frame.
    #[inline(never)]
    pub(super) fn indexed_map_literal(
        &mut self,
        entries: &[(Expr, Expr)],
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let mut map: Vec<(MapKey, Value)> = Vec::new();
        let mut positions: Option<HashMap<Key<'_>, usize>> = Some(HashMap::new());
        for (key_expr, value_expr) in entries {
            // Preserve key validation before the value, per-entry effects, and
            // final value validation. The table never owns a Value or key text.
            let key_value = self.evaluate(key_expr, output)?;
            let key = self.operation(
                MapKey::from_value(key_value)
                    .map_err(|_| ops::Error::Type("map keys must be int or string")),
                key_expr.span,
            )?;
            let value = self.evaluate(value_expr, output)?;
            if let Some((_, old)) = map
                .iter_mut()
                .take(PREFIX)
                .find(|(stored, _)| stored == &key)
            {
                *old = value;
                continue;
            }
            if map.len() < PREFIX {
                map.push((key, value));
                continue;
            }
            if let Some(index) = &mut positions {
                let literal = Key::literal(key_expr).expect("eligible literal key");
                if let Some(&at) = index.get(&literal) {
                    // Positions are private to this append/replace builder;
                    // reentrant value evaluation cannot access its structure.
                    map[at].1 = value;
                    continue;
                }
                // Reserve a bounded first batch instead of repeatedly growing
                // the transient index. Prefix-only duplicates allocate none.
                let reserve = if index.is_empty() {
                    entries.len().saturating_sub(PREFIX).min(128)
                } else {
                    1
                };
                #[cfg(test)]
                let reserve = if self
                    .fail_map_literal_index_after
                    .is_some_and(|limit| index.len() >= limit)
                {
                    usize::MAX
                } else {
                    reserve
                };
                if index.try_reserve(reserve).is_ok() {
                    index.insert(literal, map.len());
                } else {
                    // Metadata is best effort, with no new script failure.
                    positions = None;
                }
            } else if let Some((_, old)) = map
                .iter_mut()
                .skip(PREFIX)
                .find(|(stored, _)| stored == &key)
            {
                *old = value;
                continue;
            }
            map.push((key, value));
        }
        let value = Value::Map(map);
        self.validate_value(&value, 0, span)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entries(count: usize) -> Vec<(Expr, Expr)> {
        (0..count)
            .map(|i| {
                (
                    Expr {
                        kind: ExprKind::Literal(Literal::Int(i as i64)),
                        span: Span::default(),
                    },
                    Expr {
                        kind: ExprKind::Literal(Literal::Null),
                        span: Span::default(),
                    },
                )
            })
            .collect()
    }
    #[test]
    fn transient_index_limits_and_dynamic_keys_keep_the_legacy_path() {
        assert!(!eligible(&entries(MIN_ENTRIES - 1)));
        assert!(eligible(&entries(MIN_ENTRIES)));
        assert!(eligible(&entries(MAX_ENTRIES)));
        assert!(!eligible(&entries(MAX_ENTRIES + 1)));
        let mut rows = entries(MIN_ENTRIES);
        rows[0].0.kind = ExprKind::Literal(Literal::String("x".repeat(MAX_KEY_BYTES)));
        assert!(eligible(&rows));
        if let ExprKind::Literal(Literal::String(key)) = &mut rows[0].0.kind {
            key.push('x');
        }
        assert!(!eligible(&rows));
        rows[0].0.kind = ExprKind::Variable("dynamic".into());
        assert!(!eligible(&rows));
    }
}
