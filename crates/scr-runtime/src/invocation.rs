//! Backend-independent host invocation with lossless exception propagation.
use super::*;

pub type InvocationResult = std::result::Result<Value, InvocationError>;

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
    pub fn invoke(&mut self, callee: Value, arguments: &[Value]) -> InvocationResult {
        self.checkpoint()?;
        let Value::Callable(callable) = callee else {
            return Err(self.error(NativeError::type_error("invocation requires a callable")));
        };
        for value in arguments {
            self.runtime
                .validate_value(value, 0, self.span)
                .map_err(|signal| InvocationError { signal })?;
        }
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
