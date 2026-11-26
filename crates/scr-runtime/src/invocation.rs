//! Backend-independent host invocation with lossless exception propagation.
use super::*;

pub type InvocationResult = std::result::Result<Value, InvocationError>;
const MAX_CACHEABLE_ARGUMENT_NODES: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgumentState {
    Unknown,
    Immutable,
    Opaque,
}

/// A borrowed callback argument whose successful structural validation can be
/// reused for an immutable data tree of at most 64 Value nodes. Objects and
/// callables are never cached; byte-budgeted invocations always perform their
/// ordinary value checks.
///
/// Keeping the argument alive prevents mutation of its owned collection data:
/// ```compile_fail
/// use scr_runtime::{InvocationArgument, Value};
/// let mut value = Value::list(vec![Value::Int(1)]);
/// let argument = InvocationArgument::new(&value);
/// value = Value::Null;
/// drop(argument);
/// ```
pub struct InvocationArgument<'a> {
    value: &'a Value,
    state: std::cell::Cell<ArgumentState>,
}

impl<'a> InvocationArgument<'a> {
    /// Borrow without validating or charging a checkpoint. The first actual
    /// invocation validates normally, before any result can be reused.
    #[inline]
    pub fn new(value: &'a Value) -> Self {
        Self {
            value,
            state: std::cell::Cell::new(ArgumentState::Unknown),
        }
    }
}

// Limit additional classification work. This only inspects enum variants, never
// native hooks, and only runs after ordinary validation has succeeded.
fn immutable_argument(value: &Value, remaining: &mut usize) -> bool {
    if *remaining == 0 {
        return false;
    }
    *remaining -= 1;
    match value {
        Value::List(items) => items.iter().all(|item| immutable_argument(item, remaining)),
        Value::Map(items) => items
            .iter()
            .all(|(_, item)| immutable_argument(item, remaining)),
        Value::NativeObject(_) | Value::Callable(_) => false,
        Value::Null
        | Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::String(_)
        | Value::Bytes(_) => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Completed,
    Exited(u8),
}

/// Retains the original interpreter signal. Only catchable exceptions have a
/// kind; exits, interruption, and fatal failures must be propagated by callbacks.
pub struct InvocationError {
    pub(crate) signal: Signal,
}

impl InvocationError {
    pub fn exception_kind(&self) -> Option<&str> {
        match &self.signal {
            Signal::Throw(value, _) => Some(value.type_name()),
            Signal::TypeError(_) => Some("TypeError"),
            Signal::NativeException(error, _) => Some(&error.kind),
            Signal::OperationException(kind, _) => Some(kind),
            _ => None,
        }
    }
    pub fn diagnostic(&self) -> Option<&RuntimeError> {
        match &self.signal {
            Signal::Error(error)
            | Signal::TypeError(error)
            | Signal::NativeException(_, error)
            | Signal::OperationException(_, error)
            | Signal::Interrupted(error)
            | Signal::Throw(_, error) => Some(error),
            _ => None,
        }
    }
    pub fn exit_code(&self) -> Option<u8> {
        if let Signal::Exit(code) = self.signal {
            Some(code)
        } else {
            None
        }
    }
    pub fn is_interrupted(&self) -> bool {
        matches!(self.signal, Signal::Interrupted(_))
    }
}
impl std::fmt::Debug for InvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationError")
            .field("kind", &self.exception_kind())
            .field("diagnostic", &self.diagnostic())
            .field("exit_code", &self.exit_code())
            .finish()
    }
}
impl std::fmt::Display for InvocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(error) = self.diagnostic() {
            error.fmt(f)
        } else if let Some(code) = self.exit_code() {
            write!(f, "script exited with code {code}")
        } else {
            f.write_str("invalid invocation control flow")
        }
    }
}
impl std::error::Error for InvocationError {}

/// Scoped access supplied to a reentrant native function. Invocation uses the
/// current output writer, stack, limits, cancellation flag, and lexical owners.
pub struct InvocationContext<'a> {
    pub(crate) runtime: &'a mut Runtime,
    pub(crate) output: &'a mut dyn ScriptOutput,
    pub(crate) span: Span,
}
impl InvocationContext<'_> {
    /// Logical per-value byte ceiling for checked native output/work allocation.
    /// This is not an aggregate host heap limit and does not reset execution budgets.
    pub fn max_value_bytes(&self) -> Option<usize> {
        self.runtime.options.max_value_bytes
    }

    pub fn invoke(&mut self, callee: Value, arguments: &[Value]) -> InvocationResult {
        let callable = self.validate_invocation(callee, arguments)?;
        let mut values = if arguments.is_empty() {
            Vec::new()
        } else {
            self.runtime.take_argument_buffer()
        };
        values.extend_from_slice(arguments);
        self.runtime
            .call_callable(callable, values, self.span, self.output)
            .map_err(|signal| InvocationError { signal })
    }

    /// Invoke with an owned, fixed-size argument array, moving its values into
    /// the reusable call buffer instead of cloning their collection payloads.
    /// Validation and control-flow propagation match `invoke`; callers must
    /// still clone any input whose snapshot they need to retain.
    pub fn invoke_owned<const N: usize>(
        &mut self,
        callee: Value,
        arguments: [Value; N],
    ) -> InvocationResult {
        let callable = self.validate_invocation(callee, &arguments)?;
        let mut values = if N == 0 {
            Vec::new()
        } else {
            self.runtime.take_argument_buffer()
        };
        values.extend(arguments);
        self.runtime
            .call_callable(callable, values, self.span, self.output)
            .map_err(|signal| InvocationError { signal })
    }

    /// Invoke with borrowed argument holders. Values are cloned into independent
    /// callback snapshots as in invoke_owned. Only successful checks of small
    /// immutable data are reused, and only while the byte quota is unset.
    pub fn invoke_arguments<const N: usize>(
        &mut self,
        callee: Value,
        arguments: [&InvocationArgument<'_>; N],
    ) -> InvocationResult {
        // Preserve owned invocation's clone/checkpoint/validation ordering,
        // including native-handle alias counts during payload observations.
        let values = arguments.map(|argument| argument.value.clone());
        #[cfg(not(test))]
        let reuse = true;
        #[cfg(test)]
        let reuse = !self.runtime.disable_argument_validation_reuse;
        if !reuse || self.max_value_bytes().is_some() {
            return self.invoke_owned(callee, values);
        }
        self.checkpoint()?;
        let Value::Callable(callable) = callee else {
            return Err(self.error(NativeError::type_error("invocation requires a callable")));
        };
        for (argument, value) in arguments.iter().zip(&values) {
            if argument.state.get() == ArgumentState::Immutable {
                #[cfg(test)]
                {
                    self.runtime.argument_validations_reused += 1;
                }
                continue;
            }
            self.runtime
                .validate_value(value, 0, self.span)
                .map_err(|signal| InvocationError { signal })?;
            if argument.state.get() == ArgumentState::Unknown {
                let mut remaining = MAX_CACHEABLE_ARGUMENT_NODES;
                argument
                    .state
                    .set(if immutable_argument(value, &mut remaining) {
                        ArgumentState::Immutable
                    } else {
                        ArgumentState::Opaque
                    });
            }
        }
        let mut buffer = if N == 0 {
            Vec::new()
        } else {
            self.runtime.take_argument_buffer()
        };
        buffer.extend(values);
        self.runtime
            .call_callable(callable, buffer, self.span, self.output)
            .map_err(|signal| InvocationError { signal })
    }

    #[inline]
    fn validate_invocation(
        &mut self,
        callee: Value,
        arguments: &[Value],
    ) -> std::result::Result<Callable, InvocationError> {
        self.checkpoint()?;
        let Value::Callable(callable) = callee else {
            return Err(self.error(NativeError::type_error("invocation requires a callable")));
        };
        for value in arguments {
            self.runtime
                .validate_value(value, 0, self.span)
                .map_err(|signal| InvocationError { signal })?;
        }
        Ok(callable)
    }
    pub fn error(&self, error: NativeError) -> InvocationError {
        InvocationError {
            signal: self.runtime.native_error(error, self.span),
        }
    }
    pub fn require_permission(&self, capability: &str) -> std::result::Result<(), InvocationError> {
        self.runtime
            .options
            .context
            .permissions
            .require(capability)
            .map_err(|error| self.error(error))
    }
    pub fn checkpoint(&mut self) -> std::result::Result<(), InvocationError> {
        self.runtime
            .checkpoint(self.span)
            .map_err(|signal| InvocationError { signal })
    }
}

#[cfg(test)]
mod argument_tests;

impl Runtime {
    /// Look up a global script/native function without evaluating source.
    pub fn get_callable(&self, name: &str) -> Option<Value> {
        self.environments.scope(0).functions.get(name).cloned()
    }
    /// Invoke a retained callable from the host. The instruction budget is not
    /// reset, and user callables must belong to this runtime. Unlike execute,
    /// this method preserves script exit as a distinct non-success outcome.
    pub fn invoke(
        &mut self,
        callee: Value,
        arguments: &[Value],
        output: &mut impl Write,
    ) -> InvocationResult {
        let span = match &callee {
            Value::Callable(callable) => match callable.0.as_ref() {
                Function::User { declaration, .. } => declaration.name_span,
                _ => Span::default(),
            },
            _ => Span::default(),
        };
        let mut output = output::ExecutionOutput::new(output, &self.options);
        let host_frame = self.stack.is_empty();
        if host_frame {
            self.stack.push(
                StackFrame {
                    function: "<host>".into(),
                    span,
                    source: self.source.clone(),
                }
                .into(),
            );
        }
        let result = InvocationContext {
            runtime: self,
            output: &mut output,
            span,
        }
        .invoke(callee, arguments);
        if host_frame {
            self.stack.pop();
        }
        if result.is_ok() {
            output.finish().map_err(|error| InvocationError {
                signal: self.native_error(error, span),
            })?;
        }
        self.collect_garbage();
        result
    }
}
