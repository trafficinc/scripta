//! Typed native registration and host services, independent of the standard library.
//!
//! ```
//! use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, Value};
//! let mut runtime = Runtime::new();
//! runtime.register("System::platform", NativeFunction::new(
//!     NativeSignature::new(vec![], NativeType::String),
//!     |_, _| Ok(Value::String(std::env::consts::OS.into())),
//! ))?;
//! # Ok::<(), scr_runtime::NativeError>(())
//! ```
use crate::Value;
use scr_diagnostics::Span;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt,
    io::{BufRead, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeError {
    pub kind: String,
    pub message: String,
}
impl NativeError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
    pub fn type_error(message: impl Into<String>) -> Self {
        Self::new("TypeError", message)
    }
}
impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}
impl std::error::Error for NativeError {}
impl From<std::io::Error> for NativeError {
    fn from(error: std::io::Error) -> Self {
        Self::new("IoError", error.to_string())
    }
}
impl From<String> for NativeError {
    fn from(message: String) -> Self {
        Self::new("NativeError", message)
    }
}
pub type NativeResult<T = Value> = Result<T, NativeError>;

/// Named capabilities are enforced by native modules before performing effects.
/// Defaults allow automation capabilities; explicit shell execution remains opt-in.
#[derive(Debug, Clone)]
pub struct PermissionConfig {
    pub allowed: BTreeSet<String>,
}
impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            allowed: BTreeSet::from([
                "sleep".into(),
                "filesystem.read".into(),
                "filesystem.write".into(),
                "environment.read".into(),
                "environment.write".into(),
                "process.execute".into(),
                "process.inspect".into(),
                "system.inspect".into(),
                "log.write".into(),
                "http.request".into(),
                "cli.prompt".into(),
            ]),
        }
    }
}
impl PermissionConfig {
    pub fn require(&self, capability: &str) -> NativeResult<()> {
        if self.allowed.contains(capability) {
            Ok(())
        } else {
            Err(NativeError::new(
                "PermissionError",
                format!("permission denied: {capability}"),
            ))
        }
    }
}

/// Owned host settings. Environment values are an isolated snapshot, never a
/// mutation of the process environment. Hosts can inject a filtered snapshot.
#[derive(Debug, Clone)]
pub struct ContextOptions {
    pub working_directory: PathBuf,
    pub script_path: Option<PathBuf>,
    pub cli_arguments: Vec<OsString>,
    pub environment: BTreeMap<OsString, OsString>,
    pub permissions: PermissionConfig,
    /// Extra literal values removed from rendered runtime diagnostics.
    pub redacted_values: Vec<String>,
}
impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            working_directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            script_path: None,
            cli_arguments: Vec::new(),
            environment: std::env::vars_os().collect(),
            permissions: PermissionConfig::default(),
            redacted_values: Vec::new(),
        }
    }
}

/// A short-lived view of host services for one native invocation. Streams are
/// borrowed from the embedding runtime; source locations refer to the script call.
pub struct RuntimeContext<'a> {
    pub working_directory: &'a mut PathBuf,
    pub script_path: Option<&'a Path>,
    pub cli_arguments: &'a [OsString],
    pub environment: &'a mut BTreeMap<OsString, OsString>,
    pub input: &'a mut dyn BufRead,
    pub output: &'a mut dyn Write,
    pub error: &'a mut dyn Write,
    pub cancellation: Option<Arc<AtomicBool>>,
    pub permissions: &'a PermissionConfig,
    pub call_span: Span,
    pub max_value_bytes: Option<usize>,
}
impl RuntimeContext<'_> {
    pub fn check_cancelled(&self) -> NativeResult<()> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            Err(NativeError::new("Interrupted", "execution interrupted"))
        } else {
            Ok(())
        }
    }
    pub fn env(&self, name: impl AsRef<OsStr>) -> NativeResult<Option<&OsStr>> {
        self.permissions.require("environment.read")?;
        let name = name.as_ref();
        #[cfg(windows)]
        if let Some(name) = name.to_str() {
            return Ok(self
                .environment
                .iter()
                .find(|(key, _)| {
                    key.to_str()
                        .is_some_and(|key| key.eq_ignore_ascii_case(name))
                })
                .map(|(_, value)| value.as_os_str()));
        }
        Ok(self.environment.get(name).map(OsString::as_os_str))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeType {
    Any,
    Null,
    Bool,
    Int,
    Float,
    String,
    List,
    Map,
    Callable,
    Object(&'static str),
    OneOf(Vec<NativeType>),
}
impl NativeType {
    fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Any, _) => true,
            (Self::Null, Value::Null)
            | (Self::Bool, Value::Bool(_))
            | (Self::Int, Value::Int(_))
            | (Self::Float, Value::Float(_))
            | (Self::String, Value::String(_))
            | (Self::List, Value::List(_))
            | (Self::Map, Value::Map(_))
            | (Self::Callable, Value::Callable(_)) => true,
            (Self::OneOf(types), _) => types.iter().any(|ty| ty.accepts(value)),
            (Self::Object(name), Value::NativeObject(object)) => object.type_name() == *name,
            _ => false,
        }
    }
}
impl fmt::Display for NativeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("any"),
            Self::Null => f.write_str("null"),
            Self::Bool => f.write_str("bool"),
            Self::Int => f.write_str("int"),
            Self::Float => f.write_str("float"),
            Self::String => f.write_str("string"),
            Self::List => f.write_str("list"),
            Self::Map => f.write_str("map"),
            Self::Callable => f.write_str("callable"),
            Self::Object(name) => f.write_str(name),
            Self::OneOf(types) => {
                for (index, ty) in types.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" or ")?;
                    }
                    write!(f, "{ty}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeSignature {
    pub parameters: Vec<NativeType>,
    pub returns: NativeType,
    pub variadic: Option<NativeType>,
    pub optional: Vec<NativeType>,
}
impl NativeSignature {
    pub fn new(parameters: Vec<NativeType>, returns: NativeType) -> Self {
        Self {
            parameters,
            returns,
            variadic: None,
            optional: Vec::new(),
        }
    }
    pub fn with_optional(mut self, types: Vec<NativeType>) -> Self {
        self.optional = types;
        self
    }
    pub fn with_variadic(mut self, ty: NativeType) -> Self {
        self.variadic = Some(ty);
        self
    }
    fn validate(&self, name: &str, values: &[Value]) -> NativeResult<()> {
        if values.len() < self.parameters.len()
            || (self.variadic.is_none()
                && values.len() > self.parameters.len() + self.optional.len())
        {
            let expected = if self.parameters.len() == 1 {
                "exactly one argument".into()
            } else {
                format!("{} arguments", self.parameters.len())
            };
            let expected = if !self.optional.is_empty() && self.variadic.is_none() {
                format!(
                    "{} to {} arguments",
                    self.parameters.len(),
                    self.parameters.len() + self.optional.len()
                )
            } else if self.variadic.is_some() {
                format!("at least {} arguments", self.parameters.len())
            } else {
                expected
            };
            return Err(NativeError::type_error(format!(
                "{name}() expects {expected}, got {}",
                values.len()
            )));
        }
        for (index, value) in values.iter().enumerate() {
            let ty = self
                .parameters
                .get(index)
                .or_else(|| {
                    index
                        .checked_sub(self.parameters.len())
                        .and_then(|index| self.optional.get(index))
                })
                .or(self.variadic.as_ref())
                .ok_or_else(|| NativeError::type_error("invalid native signature"))?;
            if !ty.accepts(value) {
                return Err(NativeError::type_error(format!(
                    "{name}() argument {} requires {ty}, got {}",
                    index + 1,
                    value.type_name()
                )));
            }
        }
        Ok(())
    }
}

type Callback = dyn Fn(&mut RuntimeContext<'_>, &[Value]) -> NativeResult;
#[derive(Clone)]
pub struct NativeFunction {
    signature: NativeSignature,
    callback: Rc<Callback>,
}
impl NativeFunction {
    pub fn new(
        signature: NativeSignature,
        callback: impl Fn(&mut RuntimeContext<'_>, &[Value]) -> NativeResult + 'static,
    ) -> Self {
        Self {
            signature,
            callback: Rc::new(callback),
        }
    }
    pub fn signature(&self) -> &NativeSignature {
        &self.signature
    }
    pub(crate) fn invoke(
        &self,
        name: &str,
        context: &mut RuntimeContext<'_>,
        values: &[Value],
    ) -> NativeResult {
        context.check_cancelled()?;
        self.signature.validate(name, values)?;
        let result = (self.callback)(context, values);
        context.check_cancelled()?;
        let value = result?;
        value.validate().map_err(NativeError::from)?;
        if !self.signature.returns.accepts(&value) {
            return Err(NativeError::new(
                "NativeError",
                format!(
                    "{name}() returned {}, expected {}",
                    value.type_name(),
                    self.signature.returns
                ),
            ));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_reject_wrong_arity_and_implicit_numeric_conversions() {
        let signature = NativeSignature::new(vec![NativeType::Int], NativeType::Null);
        for args in [
            vec![],
            vec![Value::Int(1), Value::Int(2)],
            vec![Value::Float(1.0)],
            vec![Value::Bool(true)],
            vec![Value::String("1".into())],
        ] {
            assert_eq!(
                signature.validate("test", &args).unwrap_err().kind,
                "TypeError"
            );
        }
        assert!(signature.validate("test", &[Value::Int(1)]).is_ok());
    }

    #[test]
    fn scalar_and_union_constraints_preserve_value_kinds() {
        for (ty, value) in [
            (NativeType::Null, Value::Null),
            (NativeType::Bool, Value::Bool(false)),
            (NativeType::Int, Value::Int(0)),
            (NativeType::Float, Value::Float(0.0)),
            (NativeType::String, Value::String(String::new())),
            (NativeType::List, Value::List(vec![])),
            (NativeType::Map, Value::Map(vec![])),
        ] {
            assert!(ty.accepts(&value));
            assert!(NativeType::Any.accepts(&value));
            assert_eq!(
                NativeType::OneOf(vec![NativeType::List, NativeType::Map]).accepts(&value),
                matches!(value, Value::List(_) | Value::Map(_))
            );
        }
    }
}

#[cfg(test)]
mod variadic_tests {
    use super::*;
    #[test]
    fn variadic_signatures_enforce_required_and_rest_types() {
        let signature = NativeSignature::new(vec![NativeType::Int], NativeType::Null)
            .with_variadic(NativeType::String);
        assert!(signature.validate("test", &[Value::Int(1)]).is_ok());
        assert!(
            signature
                .validate(
                    "test",
                    &[
                        Value::Int(1),
                        Value::String("x".into()),
                        Value::String("y".into())
                    ]
                )
                .is_ok()
        );
        assert!(
            signature
                .validate("test", &[])
                .unwrap_err()
                .message
                .contains("at least 1")
        );
        assert!(
            signature
                .validate("test", &[Value::Int(1), Value::Bool(true)])
                .unwrap_err()
                .message
                .contains("argument 2")
        );
    }
}

#[cfg(test)]
mod optional_tests {
    use super::*;
    #[test]
    fn optional_arguments_validate_arity_and_types() {
        let signature = NativeSignature::new(vec![NativeType::String], NativeType::Null)
            .with_optional(vec![NativeType::List, NativeType::Map]);
        assert!(
            signature
                .validate("process", &[Value::String("exe".into())])
                .is_ok()
        );
        assert!(
            signature
                .validate(
                    "process",
                    &[Value::String("exe".into()), Value::List(vec![])]
                )
                .is_ok()
        );
        assert!(
            signature
                .validate(
                    "process",
                    &[
                        Value::String("exe".into()),
                        Value::List(vec![]),
                        Value::Map(vec![])
                    ]
                )
                .is_ok()
        );
        for args in [
            vec![],
            vec![Value::String("exe".into()), Value::Int(1)],
            vec![
                Value::String("exe".into()),
                Value::List(vec![]),
                Value::Map(vec![]),
                Value::Null,
            ],
        ] {
            assert_eq!(
                signature.validate("process", &args).unwrap_err().kind,
                "TypeError"
            );
        }
    }
}
