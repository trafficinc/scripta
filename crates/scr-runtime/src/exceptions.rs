//! Exception values share script-object ownership and tracing, including causes.
use super::*;
use crate::objects::{ScriptClass, ScriptObject};
use scr_ast::{Binding, ClassDeclaration};
use scr_diagnostics::SourcePosition;
use std::cell::{Cell, OnceCell, RefCell};

pub(super) const PROPERTIES: &[&str] = &[
    "name", "message", "code", "file", "line", "column", "trace", "cause",
];
// Native extensions can introduce additional names without changing their API.
const BUILTINS: &[&str] = &[
    "Error",
    "TypeError",
    "IndexError",
    "ArithmeticError",
    "NativeError",
    "IoError",
    "PermissionError",
    "PropertyError",
    "ClassError",
    "RangeError",
    "ResourceLimitError",
    "EncodingError",
    "ModuleError",
    "ModuleSyntaxError",
    "OutputError",
    "PathError",
    "FilesystemError",
    "ProcessError",
    "StreamError",
    "StreamLimitError",
    "TimeoutError",
    "WouldBlockError",
    "JsonError",
    "CsvError",
    "RegexError",
    "DateError",
    "PasswordError",
    "SystemError",
    "WebError",
    "CookieError",
    "SessionError",
    "UploadError",
    "PackageError",
    "AssertionError",
    "HttpError",
    "SqliteError",
];
pub(super) fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

pub(crate) struct ExceptionData {
    pub message: String,
    pub code: Option<&'static str>,
    pub diagnostic: Option<Box<RuntimeError>>,
}
impl ExceptionData {
    pub fn payload_bytes(&self) -> usize {
        std::mem::size_of::<Value>()
            .saturating_add(self.message.len())
            .saturating_add(
                self.diagnostic
                    .as_deref()
                    .map_or(0, diagnostic_payload_bytes),
            )
    }
}
fn diagnostic_payload_bytes(error: &RuntimeError) -> usize {
    error.message.len().saturating_add(
        error
            .trace
            .iter()
            .map(|frame| {
                std::mem::size_of::<StackFrame>()
                    .saturating_add(frame.function.len())
                    .saturating_add(
                        frame
                            .source
                            .as_ref()
                            .map_or(0, |source| source.path.as_os_str().len()),
                    )
            })
            .fold(0usize, usize::saturating_add),
    )
}

impl Runtime {
    pub(super) fn builtin_exception_class(&self, name: &str) -> Rc<ScriptClass> {
        if let Some(class) = self.exception_classes.borrow().get(name) {
            return class.clone();
        }
        let parent = (name != "Error").then(|| self.builtin_exception_class("Error"));
        let class = Rc::new(ScriptClass {
            declaration: (&ClassDeclaration {
                name: Binding {
                    name: name.into(),
                    span: Span::default(),
                },
                parent: None,
                interfaces: Vec::new(),
                fields: Vec::new(),
                methods: Vec::new(),
            })
                .into(),
            is_interface: false,
            builtin: true,
            parent: OnceCell::from(parent),
            resolving: Cell::new(false),
            interfaces: OnceCell::from(Vec::new()),
            method_cache: crate::method_cache::MethodCache::default(),
            environment: 0,
            owner: self.identity.clone(),
            source: None,
        });
        self.exception_classes
            .borrow_mut()
            .insert(name.into(), class.clone());
        class
    }

    pub(super) fn exception_instance<'a>(&self, value: &'a Value) -> Option<&'a ScriptObject> {
        let Value::NativeObject(object) = value else {
            return None;
        };
        object
            .as_any()?
            .downcast_ref::<ScriptObject>()
            .filter(|object| {
                object.exception.borrow().is_some()
                    && Rc::ptr_eq(&object.class.owner, &self.identity)
            })
    }

    pub(super) fn signal_matches(&self, signal: &Signal, annotation: &str) -> bool {
        let kind = match signal {
            Signal::Throw(value, _) => return self.exception_matches(value, annotation),
            Signal::TypeError(_) => "TypeError",
            Signal::OperationException(kind, _) => kind,
            Signal::NativeException(error, _) => &error.kind,
            _ => return false,
        };
        if annotation == "Error" {
            return true;
        }
        if kind != annotation {
            return false;
        }
        self.class(None, annotation)
            .is_none_or(|class| class.builtin)
    }

    pub(super) fn exception_matches(&self, value: &Value, annotation: &str) -> bool {
        if annotation == "Error" {
            return true;
        } // Legacy catch-all, including arbitrary values.
        let (namespace, name) = annotation
            .split_once("::")
            .map_or((None, annotation), |(ns, name)| (Some(ns), name));
        if let Some(class) = self.class(namespace, name) {
            if let Value::NativeObject(object) = value {
                if let Some(instance) = object
                    .as_any()
                    .and_then(|o| o.downcast_ref::<ScriptObject>())
                {
                    return Rc::ptr_eq(&instance.class.owner, &self.identity)
                        && instance.class.is_a(&class);
                }
            }
            return false;
        }
        // Retain named native extension catches without requiring registration.
        namespace.is_none() && value.type_name() == name
    }

    pub(super) fn exception_value(
        &mut self,
        kind: &str,
        error: &RuntimeError,
        span: Span,
    ) -> Result<Value> {
        let class = self.builtin_exception_class(kind);
        let prefix = format!("{kind}: ");
        let message = error
            .message
            .strip_prefix(&prefix)
            .unwrap_or(&error.message)
            .to_owned();
        self.check_exception_payload(
            std::mem::size_of::<Value>()
                .saturating_add(message.len())
                .saturating_add(diagnostic_payload_bytes(error)),
            span,
        )?;
        self.allocate_exception(
            class,
            message,
            Value::Null,
            Some(Box::new(error.clone())),
            span,
        )
    }

    fn allocate_exception(
        &mut self,
        class: Rc<ScriptClass>,
        message: String,
        cause: Value,
        diagnostic: Option<Box<RuntimeError>>,
        span: Span,
    ) -> Result<Value> {
        let code = diagnostic.as_ref().map(|error| error.code);
        let data = ExceptionData {
            message,
            code,
            diagnostic,
        };
        self.check_exception_size(&data, span)?;
        let previous = self.enter_scope(0, span)?;
        let scope = self.environments.current;
        let root = self.environments.scope_mut(scope);
        root.parent = None;
        root.captured = true;
        root.variables.insert("\0cause".into(), cause);
        let object: Rc<dyn NativeObject> = Rc::new(ScriptObject {
            class,
            scope,
            payload: Cell::new(0),
            requires_snapshot: Cell::new(true),
            exception: RefCell::new(Some(data)),
        });
        self.environments.track_object(&object);
        self.environments.leave(previous);
        Ok(Value::NativeObject(object))
    }

    pub(super) fn check_exception_size(&self, data: &ExceptionData, span: Span) -> Result<()> {
        self.check_exception_payload(data.payload_bytes(), span)
    }

    fn check_exception_payload(&self, bytes: usize, span: Span) -> Result<()> {
        if self
            .options
            .max_value_bytes
            .is_some_and(|limit| bytes.saturating_add(std::mem::size_of::<Value>()) > limit)
        {
            return self.fail("execution value byte limit exceeded", span);
        }
        Ok(())
    }

    pub(super) fn construct_exception(
        &mut self,
        class: Rc<ScriptClass>,
        arguments: Vec<Value>,
        span: Span,
    ) -> Result<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(self.native_error(
                NativeError::type_error("Error constructor requires a message and optional cause"),
                span,
            ));
        }
        let Value::String(message) = &arguments[0] else {
            return Err(self.native_error(
                NativeError::type_error("Error message must be a string"),
                span,
            ));
        };
        let cause = arguments.get(1).cloned().unwrap_or(Value::Null);
        if cause != Value::Null && self.exception_instance(&cause).is_none() {
            return Err(self.native_error(
                NativeError::type_error("Error cause must be an Error or null"),
                span,
            ));
        }
        // Causes are immutable and can only reference previously constructed errors.
        let mut depth = 0;
        let mut next = cause.clone();
        while let Some(instance) = self.exception_instance(&next) {
            let following = self
                .environments
                .scope(instance.scope)
                .variables
                .get("\0cause")
                .cloned()
                .unwrap_or(Value::Null);
            self.checkpoint(span)?;
            depth += 1;
            if depth > MAX_VALUE_DEPTH {
                return self.fail("exception cause nesting exceeds 128 levels", span);
            }
            next = following;
        }
        let message = self.error(message.clone(), span).message;
        self.allocate_exception(class, message, cause, None, span)
    }

    pub(super) fn check_exception_trace(&self, instance: &ScriptObject, span: Span) -> Result<()> {
        let data = instance.exception.borrow();
        let Some(error) = data.as_ref().and_then(|data| data.diagnostic.as_ref()) else {
            return Ok(());
        };
        let mut bytes = std::mem::size_of::<Value>();
        for frame in &error.trace {
            let strings = frame.function.len().saturating_add(
                frame
                    .source
                    .as_ref()
                    .map_or(0, |source| source.path.as_os_str().len()),
            );
            bytes = bytes
                .saturating_add(std::mem::size_of::<Value>())
                .saturating_add(4 * (MapKey::LOGICAL_BYTES + std::mem::size_of::<Value>()))
                .saturating_add("functionfilelinecolumn".len())
                .saturating_add(strings);
            if self
                .options
                .max_value_bytes
                .is_some_and(|limit| bytes > limit)
            {
                return self.fail("execution value byte limit exceeded", span);
            }
        }
        Ok(())
    }

    pub(super) fn exception_property(&self, instance: &ScriptObject, name: &str) -> Option<Value> {
        let data = instance.exception.borrow();
        let data = data.as_ref()?;
        let diagnostic = data.diagnostic.as_deref();
        let text = |s: &str| Value::String(s.to_owned());
        let location = || {
            diagnostic.and_then(|error| {
                error.source.as_ref().map(|source| {
                    let position = SourcePosition::at(&source.text, error.span.start);
                    (
                        scr_diagnostics::redact_text(
                            &source.path.to_string_lossy(),
                            &error.redacted_values,
                        ),
                        position,
                    )
                })
            })
        };
        Some(match name {
            "name" => Value::String(
                self.error(&instance.class.declaration.name.name, Span::default())
                    .message,
            ),
            "message" => text(&data.message),
            "code" => data.code.map_or(Value::Null, text),
            "file" => location()
                .as_ref()
                .map_or(Value::Null, |(file, _)| text(file)),
            "line" => location()
                .as_ref()
                .map_or(Value::Null, |(_, pos)| Value::Int(pos.line as i64)),
            "column" => location()
                .as_ref()
                .map_or(Value::Null, |(_, pos)| Value::Int(pos.column as i64)),
            "trace" => Value::List(
                diagnostic
                    .into_iter()
                    .flat_map(|error| {
                        error.trace.iter().map(|frame| {
                            let location = frame.source.as_ref().map(|source| {
                                (
                                    scr_diagnostics::redact_text(
                                        &source.path.to_string_lossy(),
                                        &error.redacted_values,
                                    ),
                                    SourcePosition::at(&source.text, frame.span.start),
                                )
                            });
                            Value::map(vec![
                                (
                                    MapKey::String("function".into()),
                                    Value::String(scr_diagnostics::redact_text(
                                        &frame.function,
                                        &error.redacted_values,
                                    )),
                                ),
                                (
                                    MapKey::String("file".into()),
                                    location
                                        .as_ref()
                                        .map_or(Value::Null, |(file, _)| text(file)),
                                ),
                                (
                                    MapKey::String("line".into()),
                                    location.as_ref().map_or(Value::Null, |(_, pos)| {
                                        Value::Int(pos.line as i64)
                                    }),
                                ),
                                (
                                    MapKey::String("column".into()),
                                    location.as_ref().map_or(Value::Null, |(_, pos)| {
                                        Value::Int(pos.column as i64)
                                    }),
                                ),
                            ])
                        })
                    })
                    .collect(),
            ),
            "cause" => self
                .environments
                .scope(instance.scope)
                .variables
                .get("\0cause")
                .cloned()
                .unwrap_or(Value::Null),
            _ => return None,
        })
    }

    pub(super) fn throw_value(&mut self, value: Value, span: Span) -> Result<()> {
        if let Some(instance) = self.exception_instance(&value) {
            let mut data = instance.exception.borrow_mut();
            let data = data.as_mut().expect("exception");
            if data.diagnostic.is_none() {
                let error = self.error(format!("{}: {}", value.type_name(), data.message), span);
                let replacement = ExceptionData {
                    message: data.message.clone(),
                    code: data.code,
                    diagnostic: Some(error),
                };
                self.check_exception_size(&replacement, span)?;
                if self.options.max_value_bytes.is_some_and(|limit| {
                    replacement
                        .payload_bytes()
                        .saturating_add(instance.payload.get())
                        .saturating_add(std::mem::size_of::<Value>())
                        > limit
                }) {
                    return self.fail("execution value byte limit exceeded", span);
                }
                *data = replacement;
            }
            let error = data.diagnostic.clone().expect("captured throw");
            return Err(Signal::Throw(value.clone(), error));
        }
        let message = match &value {
            Value::String(message) => format!("uncaught thrown value: {message}"),
            _ => format!("uncaught thrown {} value", value.type_name()),
        };
        Err(Signal::Throw(value, self.error(message, span)))
    }
}
