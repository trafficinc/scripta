//! List value storage. Only trees without reference-counted handles may share.
use crate::Value;
use std::{
    cell::Cell,
    fmt,
    ops::{Deref, DerefMut},
    rc::Rc,
};

#[derive(Clone)]
struct Storage {
    values: Vec<Value>,
    plain: Cell<Option<bool>>,
}

/// A list with value semantics. Plain-data clones may share storage until a
/// mutable borrow; lists containing bytes, objects or callables clone their
/// elements so native ownership observations and collector edges stay intact.
pub struct List(Rc<Storage>);

#[cfg(test)]
thread_local! {
    static EAGER_CLONES: Cell<bool> = const { Cell::new(false) };
}

impl List {
    fn with_cache(values: Vec<Value>, plain: Option<bool>) -> Self {
        Self(Rc::new(Storage {
            values,
            plain: Cell::new(plain),
        }))
    }

    fn plain(&self, depth: usize) -> bool {
        if let Some(plain) = self.0.plain.get() {
            return plain;
        }
        let plain = depth <= crate::MAX_VALUE_DEPTH
            && self
                .0
                .values
                .iter()
                .all(|value| plain_value(value, depth + 1));
        self.0.plain.set(Some(plain));
        plain
    }

    /// Recover the owned vector, copying shared plain data when necessary.
    pub fn into_vec(self) -> Vec<Value> {
        match Rc::try_unwrap(self.0) {
            Ok(storage) => storage.values,
            Err(storage) => storage.values.clone(),
        }
    }
}

// Shape inspection does not validate values or invoke native hooks. Its first
// traversal replaces the traversal a deep clone previously needed. Depth is
// bounded even for invalid host-built values; each list caches a conservative
// result locally, with no address cache or runtime-owned roots.
pub(crate) fn plain_value(value: &Value, depth: usize) -> bool {
    if depth > crate::MAX_VALUE_DEPTH {
        return false;
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::String(_) => true,
        Value::List(items) => items.plain(depth),
        Value::Map(items) => items.plain(depth),
        Value::Bytes(_) | Value::NativeObject(_) | Value::Callable(_) => false,
    }
}

impl Clone for List {
    fn clone(&self) -> Self {
        #[cfg(test)]
        if EAGER_CLONES.get() {
            return Self::with_cache(self.0.values.clone(), Some(false));
        }
        if self.plain(0) {
            Self(self.0.clone())
        } else {
            Self::with_cache(self.0.values.clone(), Some(false))
        }
    }
}

impl Deref for List {
    type Target = Vec<Value>;
    fn deref(&self) -> &Self::Target {
        &self.0.values
    }
}

impl DerefMut for List {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let storage = Rc::make_mut(&mut self.0);
        // Invalidate before exposing arbitrary Vec mutation, including insertion
        // of opaque handles. A shared allocation is detached first.
        storage.plain.set(None);
        &mut storage.values
    }
}

impl From<Vec<Value>> for List {
    fn from(values: Vec<Value>) -> Self {
        Self::with_cache(values, None)
    }
}

impl<const N: usize> From<[Value; N]> for List {
    fn from(values: [Value; N]) -> Self {
        Vec::from(values).into()
    }
}

impl FromIterator<Value> for List {
    fn from_iter<T: IntoIterator<Item = Value>>(values: T) -> Self {
        Vec::from_iter(values).into()
    }
}

impl Default for List {
    fn default() -> Self {
        Vec::new().into()
    }
}

impl fmt::Debug for List {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.values.fmt(formatter)
    }
}

impl AsRef<[Value]> for List {
    fn as_ref(&self) -> &[Value] {
        &self.0.values
    }
}

impl<T: AsRef<[Value]> + ?Sized> PartialEq<T> for List {
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl IntoIterator for List {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a List {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut List {
    type Item = &'a mut Value;
    type IntoIter = std::slice::IterMut<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests;
