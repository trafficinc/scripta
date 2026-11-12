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

pub(crate) enum Function {
    Native {
        name: String,
        function: NativeFunction,
    },
    User {
        declaration: FunctionDeclaration,
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
            name: name.into(),
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
                .checked_add(if let Self::String(value) = self {
                    value.len()
                } else {
                    0
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
