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
    String(String),
}

impl MapKey {
    pub(crate) fn from_value(value: Value) -> Result<Self, String> {
        match value {
            Value::Int(value) => Ok(Self::Int(value)),
            Value::String(value) => Ok(Self::String(value)),
            _ => Err("map keys must be int or string".into()),
        }
    }

    pub(crate) fn value(&self) -> Value {
        match self {
            Self::Int(value) => Value::Int(*value),
            Self::String(value) => Value::String(value.clone()),
        }
    }
}

#[derive(Clone)]
pub struct Callable(pub(crate) Rc<Function>);

// Runtime-owned immutable declaration metadata; public ASTs remain unchanged.
pub(crate) struct UserDeclaration {
    ast: FunctionDeclaration,
    pub required: usize,
    pub variadic: bool,
    pub numeric_call: std::cell::OnceCell<Option<Box<crate::numeric_call::Plan>>>,
    pub numeric_seen: std::cell::Cell<bool>,
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
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// Immutable arbitrary bytes, with content equality and cheap clones.
    Bytes(Rc<[u8]>),
    List(Vec<Value>),
    /// Ordered entries with unique, strictly typed keys.
    Map(Vec<(MapKey, Value)>),
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
                    let mut keys = std::collections::HashSet::with_capacity(entries.len());
                    for (key, value) in entries {
                        if !keys.insert(key) {
                            return Err("duplicate host map key".into());
                        }
                        add = add
                            .checked_add(std::mem::size_of::<MapKey>())
                            .ok_or("value size overflow")?;
                        if let MapKey::String(key) = key {
                            add = add.checked_add(key.len()).ok_or("value size overflow")?;
                        }
                        pending.push((value, depth + 1));
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
