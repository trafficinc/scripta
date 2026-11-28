//! Bounded hints for verified positions in ordered map entry storage.
use crate::{MapKey, Value, ops::IndexKey};
use std::collections::{HashMap, VecDeque};

const MIN_ENTRIES: usize = 64;
const MAX_ENTRIES: usize = 4096;
const MAX_INDEXES: usize = 8;
const MAX_KEY_BYTES: usize = 256 * 1024;

#[inline]
pub(crate) fn eligible_size(length: usize) -> bool {
    (MIN_ENTRIES..=MAX_ENTRIES).contains(&length)
}

struct Index {
    scope: usize,
    name: String,
    length: usize,
    bytes: usize,
    integers: HashMap<i64, usize>,
    strings: HashMap<String, usize>,
}
impl Index {
    fn position(&self, key: IndexKey<'_>) -> Option<usize> {
        match key {
            IndexKey::Int(key) => self.integers.get(&key).copied(),
            IndexKey::String(key) => self.strings.get(key).copied(),
        }
    }
    fn build(scope: usize, name: &str, entries: &[(MapKey, Value)]) -> Option<Self> {
        let mut bytes = name.len();
        let mut strings = 0;
        for (key, _) in entries {
            if let MapKey::String(key) = key {
                bytes = bytes.checked_add(key.len())?;
                strings += 1;
            }
        }
        if bytes > MAX_KEY_BYTES {
            return None;
        }
        let mut result = Self {
            scope,
            name: copy_text(name)?,
            length: entries.len(),
            bytes,
            integers: HashMap::new(),
            strings: HashMap::new(),
        };
        result.integers.try_reserve(entries.len() - strings).ok()?;
        result.strings.try_reserve(strings).ok()?;
        for (position, (key, _)) in entries.iter().enumerate() {
            let previous = match key {
                MapKey::Int(key) => result.integers.insert(*key, position),
                MapKey::String(key) => result.strings.insert(copy_text(key)?, position),
            };
            // Runtime maps have already passed uniqueness validation. Also keep
            // malformed input from supplying a last-duplicate position hint.
            if previous.is_some() {
                return None;
            }
        }
        Some(result)
    }
}
fn copy_text(text: &str) -> Option<String> {
    let mut result = String::new();
    result.try_reserve_exact(text.len()).ok()?;
    result.push_str(text);
    Some(result)
}

#[derive(Default)]
pub(crate) struct MapIndexes {
    indexes: VecDeque<Index>,
    entries: usize,
    bytes: usize,
}
impl MapIndexes {
    fn remove(&mut self, position: usize) {
        let index = self.indexes.remove(position).expect("cached map index");
        self.entries -= index.length;
        self.bytes -= index.bytes;
    }
    #[inline]
    pub fn invalidate(&mut self, scope: usize, name: &str) {
        if self.indexes.is_empty() {
            return;
        }
        if let Some(position) = self
            .indexes
            .iter()
            .position(|i| i.scope == scope && i.name == name)
        {
            self.remove(position);
        }
    }
    pub fn position(
        &mut self,
        scope: usize,
        name: &str,
        entries: &[(MapKey, Value)],
        key: &Value,
        build: bool,
    ) -> Option<usize> {
        self.position_key(scope, name, entries, IndexKey::from_value(key)?, build)
    }

    pub fn position_key(
        &mut self,
        scope: usize,
        name: &str,
        entries: &[(MapKey, Value)],
        key: IndexKey<'_>,
        build: bool,
    ) -> Option<usize> {
        if !eligible_size(entries.len()) {
            return None;
        }
        if let Some(at) = self
            .indexes
            .iter()
            .position(|i| i.scope == scope && i.name == name)
        {
            let index = &self.indexes[at];
            if let Some(position) = index.position(key).filter(|&p| {
                entries
                    .get(p)
                    .is_some_and(|(stored, _)| key.matches(stored))
            }) {
                // A hot map is already most recent; moving its index again
                // only copies bookkeeping and leaves the LRU order unchanged.
                if at + 1 != self.indexes.len() {
                    let index = self.indexes.remove(at).expect("cached map index");
                    self.indexes.push_back(index);
                }
                return Some(position);
            }
            // Hints never establish absence. A replaced map or reused scope ID
            // can make an old hint stale; the caller always has a linear fallback.
            if !build || (index.position(key).is_none() && index.length == entries.len()) {
                return None;
            }
            self.remove(at);
        } else if !build {
            // Growing writes alone must not rebuild an index after every insert.
            return None;
        }
        let index = Index::build(scope, name, entries)?;
        self.indexes.try_reserve(1).ok()?;
        while self.indexes.len() >= MAX_INDEXES
            || self.entries + index.length > MAX_ENTRIES
            || self.bytes + index.bytes > MAX_KEY_BYTES
        {
            self.remove(0);
        }
        let position = index.position(key);
        self.entries += index.length;
        self.bytes += index.bytes;
        self.indexes.push_back(index);
        position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entries(size: usize) -> Vec<(MapKey, Value)> {
        (0..size)
            .map(|i| (MapKey::Int(i as i64), Value::Int(i as i64)))
            .collect()
    }
    #[test]
    fn typed_hints_verify_positions_after_reordering_and_scope_reuse() {
        let mut cache = MapIndexes::default();
        let mut values = entries(MIN_ENTRIES);
        values[1].0 = MapKey::String("0".into());
        assert_eq!(
            cache.position(7, "map", &values, &Value::Int(0), true),
            Some(0)
        );
        assert_eq!(
            cache.position(7, "map", &values, &Value::String("0".into()), true),
            Some(1)
        );
        values.rotate_right(3);
        assert_eq!(
            cache.position(7, "map", &values, &Value::Int(0), true),
            Some(3)
        );
        assert_eq!(
            cache.position(7, "map", &values, &Value::String("0".into()), true),
            Some(4)
        );
        // Same scope ID/name and width, but an entirely different valid map.
        let replaced: Vec<_> = values
            .into_iter()
            .enumerate()
            .map(|(i, (_, v))| (MapKey::Int(1000 + i as i64), v))
            .collect();
        assert_eq!(
            cache.position(7, "map", &replaced, &Value::Int(1000), true),
            None
        );
        assert_eq!(
            crate::ops::index(&Value::map(replaced), &Value::Int(1000)).unwrap(),
            Value::Int(61)
        );
    }
    #[test]
    fn indexes_obey_entry_and_lru_bounds_and_skip_small_or_oversized_maps() {
        let mut cache = MapIndexes::default();
        assert_eq!(
            cache.position(0, "small", &entries(MIN_ENTRIES - 1), &Value::Int(0), true),
            None
        );
        assert!(cache.indexes.is_empty());
        for i in 0..MAX_INDEXES {
            cache.position(
                0,
                &format!("m{i}"),
                &entries(MIN_ENTRIES),
                &Value::Int(0),
                true,
            );
        }
        assert_eq!(cache.indexes.len(), MAX_INDEXES);
        cache.position(0, "m0", &entries(MIN_ENTRIES), &Value::Int(0), true);
        cache.position(0, "extra", &entries(MIN_ENTRIES), &Value::Int(0), true);
        assert!(!cache.indexes.iter().any(|i| i.name == "m1"));
        assert!(cache.indexes.iter().any(|i| i.name == "m0"));
        cache.position(0, "full", &entries(MAX_ENTRIES), &Value::Int(0), true);
        assert_eq!(cache.entries, MAX_ENTRIES);
        assert_eq!(cache.indexes.len(), 1);
        assert_eq!(
            cache.position(0, "over", &entries(MAX_ENTRIES + 1), &Value::Int(0), true),
            None
        );
        assert_eq!(cache.entries, MAX_ENTRIES);
        cache.position(0, "new", &entries(MIN_ENTRIES), &Value::Int(0), true);
        assert_eq!(cache.entries, MIN_ENTRIES);
    }
    #[test]
    fn key_byte_ceiling_is_exact_and_plus_one_falls_back_without_eviction() {
        let mut cache = MapIndexes::default();
        let mut values = entries(MIN_ENTRIES);
        values[0].0 = MapKey::String(("é".repeat((MAX_KEY_BYTES - 2) / 2) + "x").into());
        assert_eq!(
            cache.position(0, "m", &values, &Value::Int(1), true),
            Some(1)
        );
        assert_eq!(cache.bytes, MAX_KEY_BYTES);
        values[0].0 = MapKey::String("é".repeat(MAX_KEY_BYTES / 2).into());
        assert_eq!(cache.position(0, "n", &values, &Value::Int(1), true), None);
        assert_eq!(cache.bytes, MAX_KEY_BYTES);
        cache.position(0, "z", &entries(MIN_ENTRIES), &Value::Int(1), true);
        assert_eq!(cache.bytes, 1);
        assert_eq!(cache.indexes.len(), 1);
    }
    #[test]
    fn duplicate_maps_are_not_indexed_and_indexes_retain_no_value_owners() {
        let mut cache = MapIndexes::default();
        let mut values = entries(MIN_ENTRIES);
        values[1].0 = MapKey::Int(0);
        assert_eq!(
            cache.position(0, "bad", &values, &Value::Int(0), true),
            None
        );
        assert!(cache.indexes.is_empty());
        let callable = Value::native_function(
            "held",
            crate::NativeFunction::new(
                crate::NativeSignature::new(vec![], crate::NativeType::Null),
                |_, _| Ok(Value::Null),
            ),
        );
        let Value::Callable(ref function) = callable else {
            unreachable!()
        };
        let weak = std::rc::Rc::downgrade(&function.0);
        values = entries(MIN_ENTRIES);
        values[0].1 = callable;
        cache.position(0, "valid", &values, &Value::Int(0), true);
        drop(values);
        assert!(weak.upgrade().is_none());
        assert_eq!(cache.indexes.len(), 1);
    }
}
