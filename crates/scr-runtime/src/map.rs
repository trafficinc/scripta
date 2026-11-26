//! Ordered map storage with value semantics and plain-data sharing.
use crate::{MapKey, Value, list::plain_value};
use std::{
    cell::Cell,
    fmt,
    ops::{Deref, DerefMut},
    rc::Rc,
};

#[derive(Clone)]
struct Storage {
    entries: Vec<(MapKey, Value)>,
    plain: Cell<Option<bool>>,
}

/// Insertion-ordered map entries. Plain-data clones share storage until mutable
/// access; opaque handles retain element cloning and their ownership counts.
/// Key uniqueness is checked by the runtime, not by this storage wrapper.
pub struct Map(Rc<Storage>);

#[cfg(test)]
thread_local! {
    static EAGER_CLONES: Cell<bool> = const { Cell::new(false) };
}

impl Map {
    fn with_cache(entries: Vec<(MapKey, Value)>, plain: Option<bool>) -> Self {
        Self(Rc::new(Storage {
            entries,
            plain: Cell::new(plain),
        }))
    }

    pub(crate) fn plain(&self, depth: usize) -> bool {
        if let Some(plain) = self.0.plain.get() {
            return plain;
        }
        let plain = depth <= crate::MAX_VALUE_DEPTH
            && self
                .0
                .entries
                .iter()
                .all(|(_, value)| plain_value(value, depth + 1));
        self.0.plain.set(Some(plain));
        plain
    }

    /// Recover the entry vector, copying shared plain data when necessary.
    pub fn into_vec(self) -> Vec<(MapKey, Value)> {
        match Rc::try_unwrap(self.0) {
            Ok(storage) => storage.entries,
            Err(storage) => storage.entries.clone(),
        }
    }
}

impl Clone for Map {
    fn clone(&self) -> Self {
        #[cfg(test)]
        if EAGER_CLONES.get() {
            return Self::with_cache(self.0.entries.clone(), self.0.plain.get());
        }
        if self.plain(0) {
            Self(self.0.clone())
        } else {
            Self::with_cache(self.0.entries.clone(), Some(false))
        }
    }
}

impl Deref for Map {
    type Target = Vec<(MapKey, Value)>;
    fn deref(&self) -> &Self::Target {
        &self.0.entries
    }
}

impl DerefMut for Map {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let storage = Rc::make_mut(&mut self.0);
        // Key edits and nested writes invalidate the proof before a mutable
        // reference escapes. Detachment precedes insertion of opaque handles.
        storage.plain.set(None);
        &mut storage.entries
    }
}

impl From<Vec<(MapKey, Value)>> for Map {
    fn from(entries: Vec<(MapKey, Value)>) -> Self {
        Self::with_cache(entries, None)
    }
}

impl<const N: usize> From<[(MapKey, Value); N]> for Map {
    fn from(entries: [(MapKey, Value); N]) -> Self {
        Vec::from(entries).into()
    }
}

impl FromIterator<(MapKey, Value)> for Map {
    fn from_iter<T: IntoIterator<Item = (MapKey, Value)>>(entries: T) -> Self {
        Vec::from_iter(entries).into()
    }
}

impl Default for Map {
    fn default() -> Self {
        Vec::new().into()
    }
}

impl fmt::Debug for Map {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.entries.fmt(formatter)
    }
}

impl AsRef<[(MapKey, Value)]> for Map {
    fn as_ref(&self) -> &[(MapKey, Value)] {
        &self.0.entries
    }
}

// Storage comparison retains Vec's entry order. Value::Map equality separately
// implements the language's order-independent key/value contract.
impl<T: AsRef<[(MapKey, Value)]> + ?Sized> PartialEq<T> for Map {
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl IntoIterator for Map {
    type Item = (MapKey, Value);
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a Map {
    type Item = &'a (MapKey, Value);
    type IntoIter = std::slice::Iter<'a, (MapKey, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Map {
    type Item = &'a mut (MapKey, Value);
    type IntoIter = std::slice::IterMut<'a, (MapKey, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests;
