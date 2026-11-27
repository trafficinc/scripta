use std::{fmt, rc::Rc};

use scr_ast::FunctionDeclaration;

use crate::NativeFunction;

/// Host-defined objects expose properties without introducing script classes.
/// Implementations may use safe interior mutability for writable properties.
pub trait NativeObject: fmt::Debug {
    /// Optional safe downcast for host resources such as process handles.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
    /// Retained payload counted toward the logical per-value budget.
    fn payload_bytes(&self) -> usize {
        0
    }
    fn type_name(&self) -> &str;
    fn get_property(&self, name: &str) -> Result<Value, String>;
    fn set_property(&self, name: &str, _value: Value) -> Result<(), String> {
        Err(format!("property {name} is read-only"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MapKey {
    Int(i64),
    String(crate::MapKeyText),
}

const SMALL_MAP_KEYS: usize = 16;

#[cfg(test)]
thread_local! {
    static HASHED_MAP_VALIDATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[inline]
pub(crate) fn small_map_validation(length: usize) -> bool {
    #[cfg(test)]
    if HASHED_MAP_VALIDATION.get() {
        return false;
    }
    length <= SMALL_MAP_KEYS
}

#[cfg(test)]
pub(crate) fn with_hashed_map_validation<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            HASHED_MAP_VALIDATION.set(self.0);
        }
    }
    let _restore = Restore(HASHED_MAP_VALIDATION.replace(true));
    run()
}

impl MapKey {
    /// Logical slot charge, preserving the original owned-string key layout.
    /// Shared text changes physical storage, not per-value quota boundaries.
    pub const LOGICAL_BYTES: usize = std::mem::size_of::<String>();

    pub(crate) fn from_value(value: Value) -> Result<Self, String> {
        match value {
            Value::Int(value) => Ok(Self::Int(value)),
            Value::String(value) => Ok(Self::String(value.into())),
            _ => Err("map keys must be int or string".into()),
        }
    }

    pub(crate) fn value(&self) -> Value {
        match self {
            Self::Int(value) => Value::Int(*value),
            Self::String(value) => Value::String(value.as_str().to_owned()),
        }
    }
}

#[derive(Clone)]
pub struct Callable(pub(crate) Rc<Function>);

// Receivers and captured free functions are mutually exclusive. Sharing this
// cold cache keeps additional plan slots out of every function declaration.
pub(crate) enum SpecialCall {
    Method(Box<crate::method_plan::Plan>),
    Captured(Box<crate::captured_call::Plan>),
}

// Runtime-owned immutable declaration metadata; public ASTs remain unchanged.
pub(crate) struct UserDeclaration {
    ast: FunctionDeclaration,
    pub required: usize,
    pub variadic: bool,
    pub numeric_call: std::cell::OnceCell<Option<Box<crate::numeric_call::Plan>>>,
    pub numeric_seen: std::cell::Cell<bool>,
    pub prepared_call: std::cell::OnceCell<Option<Box<crate::prepared_call::Plan>>>,
    pub compact_call: std::cell::OnceCell<Option<Box<crate::compact_call::Plan>>>,
    pub special_call: std::cell::OnceCell<Option<SpecialCall>>,
    parameter_names: std::cell::OnceCell<Box<[Rc<String>]>>,
    receiver_name: std::cell::OnceCell<Rc<String>>,
}
impl UserDeclaration {
    pub fn new(ast: FunctionDeclaration) -> Self {
        Self {
            required: ast
                .parameters
                .iter()
                .filter(|p| p.default.is_none() && !p.variadic)
                .count(),
            variadic: ast.parameters.last().is_some_and(|p| p.variadic),
            numeric_call: std::cell::OnceCell::new(),
            numeric_seen: std::cell::Cell::new(false),
            prepared_call: std::cell::OnceCell::new(),
            compact_call: std::cell::OnceCell::new(),
            special_call: std::cell::OnceCell::new(),
            parameter_names: std::cell::OnceCell::new(),
            receiver_name: std::cell::OnceCell::new(),
            ast,
        }
    }

    pub fn parameter_name(&self, index: usize) -> Option<&Rc<String>> {
        if self.parameters.len() > crate::bindings::SMALL_BINDINGS {
            return None;
        }
        self.parameter_names
            .get_or_init(|| {
                self.parameters
                    .iter()
                    .map(|p| Rc::new(p.binding.name.clone()))
                    .collect()
            })
            .get(index)
    }

    pub fn receiver_name(&self) -> &Rc<String> {
        self.receiver_name.get_or_init(|| Rc::new("this".into()))
    }
}
impl std::ops::Deref for UserDeclaration {
    type Target = FunctionDeclaration;
    fn deref(&self) -> &Self::Target {
        &self.ast
    }
}

pub(crate) enum Function {
    Native {
        name: Rc<str>,
        function: NativeFunction,
    },
    User {
        declaration: Rc<UserDeclaration>,
        receiver: Option<Value>,
        environment: usize,
        owner: Rc<()>,
        source: Option<std::sync::Arc<crate::SourceFile>>,
    },
}

impl fmt::Debug for Callable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Callable").field(&self.name()).finish()
    }
}

impl Callable {
    pub fn name(&self) -> &str {
        match self.0.as_ref() {
            Function::Native { name, .. } => name,
            Function::User { declaration, .. } => &declaration.name,
        }
    }
}

#[derive(Debug, Clone)]
// Keep the existing tagged slot size used by logical byte quotas. Without an
// explicit tag, collection indirection lets enum niche packing shrink the slot.
#[repr(u8)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// Immutable arbitrary bytes, with content equality and cheap clones.
    Bytes(Rc<[u8]>),
    List(crate::List),
    /// Ordered entries with unique, strictly typed keys.
    Map(crate::Map),
    NativeObject(Rc<dyn NativeObject>),
    Callable(Callable),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::List(a), Self::List(b)) => a == b,
            (Self::Map(a), Self::Map(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(key, value)| {
                        b.iter().any(|(other_key, other_value)| {
                            key == other_key && value == other_value
                        })
                    })
            }
            (Self::NativeObject(a), Self::NativeObject(b)) => Rc::ptr_eq(a, b),
            (Self::Callable(a), Self::Callable(b)) => Rc::ptr_eq(&a.0, &b.0),
            _ => false,
        }
    }
}

impl Value {
    /// Construct a list from an owned vector. Mutable access to its public
    /// List payload preserves independent value semantics.
    pub fn list(values: Vec<Value>) -> Self {
        Self::List(values.into())
    }

    /// Construct an insertion-ordered map from owned entries. Runtime boundaries
    /// still validate key uniqueness, nested values and logical resource limits.
    pub fn map(entries: Vec<(MapKey, Value)>) -> Self {
        Self::Map(entries.into())
    }

    /// Construct a native callable for object methods without a global registration.
    pub fn native_function(name: impl Into<String>, function: NativeFunction) -> Self {
        Self::Callable(Callable(Rc::new(Function::Native {
            name: Rc::from(name.into()),
            function,
        })))
    }

    pub fn type_name(&self) -> &str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::String(_) => "string",
            Self::Bytes(_) => "bytes",
            Self::List(_) => "list",
            Self::Map(_) => "map",
            Self::NativeObject(object) => object.type_name(),
            Self::Callable(_) => "callable",
        }
    }

    #[inline]
    pub(crate) fn validate(&self) -> Result<(), String> {
        match self {
            Self::Float(value) if !value.is_finite() => {
                Err("floating-point value must be finite".into())
            }
            Self::List(_) | Self::Map(_) => self.measure(0, None).map(|_| ()),
            _ => Ok(()),
        }
    }
    pub(crate) fn measure(
        &self,
        initial_depth: usize,
        limit: Option<usize>,
    ) -> Result<usize, String> {
        if initial_depth > crate::MAX_VALUE_DEPTH {
            return Err("value nesting exceeds 128 levels".into());
        }
        if !matches!(self, Self::List(_) | Self::Map(_)) {
            if matches!(self, Self::Float(value) if !value.is_finite()) {
                return Err("floating-point value must be finite".into());
            }
            let bytes = std::mem::size_of::<Value>()
                .checked_add(match self {
                    Self::String(value) => value.len(),
                    Self::Bytes(value) => value.len(),
                    Self::NativeObject(object) => object.payload_bytes(),
                    _ => 0,
                })
                .ok_or("value size overflow")?;
            if limit.is_some_and(|limit| bytes > limit) {
                return Err("execution value byte limit exceeded".into());
            }
            return Ok(bytes);
        }
        let mut pending = vec![(self, initial_depth)];
        let mut bytes = 0usize;
        while let Some((value, depth)) = pending.pop() {
            if depth > crate::MAX_VALUE_DEPTH {
                return Err("value nesting exceeds 128 levels".into());
            }
            let mut add = std::mem::size_of::<Value>();
            match value {
                Self::Float(n) if !n.is_finite() => {
                    return Err("floating-point value must be finite".into());
                }
                Self::String(s) => add = add.checked_add(s.len()).ok_or("value size overflow")?,
                Self::Bytes(b) => add = add.checked_add(b.len()).ok_or("value size overflow")?,
                Self::NativeObject(object) => {
                    add = add
                        .checked_add(object.payload_bytes())
                        .ok_or("value size overflow")?
                }
                Self::List(values) => pending.extend(values.iter().map(|v| (v, depth + 1))),
                Self::Map(entries) => {
                    if small_map_validation(entries.len()) {
                        add = measure_small_map(entries)?;
                        pending.extend(entries.iter().map(|(_, value)| (value, depth + 1)));
                    } else {
                        let mut keys = std::collections::HashSet::with_capacity(entries.len());
                        for (key, value) in entries {
                            if !keys.insert(key) {
                                return Err("duplicate host map key".into());
                            }
                            add = add
                                .checked_add(MapKey::LOGICAL_BYTES)
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
            .checked_add(MapKey::LOGICAL_BYTES)
            .ok_or("value size overflow")?;
        if let MapKey::String(key) = key {
            add = add.checked_add(key.len()).ok_or("value size overflow")?;
        }
    }
    Ok(add)
}

#[cfg(test)]
#[path = "map_validation_tests.rs"]
mod map_validation_tests;
