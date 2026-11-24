//! Safe, synchronous AST interpretation with lexical scopes and injectable output.
#![forbid(unsafe_code)]

#[cfg(test)]
mod adaptive_read_tests;
mod arrays;
mod bindings;
#[cfg(test)]
mod block_scope_tests;
#[cfg(test)]
mod call_reuse_tests;
#[cfg(test)]
mod call_setup_tests;
mod compact_call;
#[cfg(test)]
mod compact_call_tests;
#[cfg(test)]
mod compact_frame_tests;
mod copy_validation;
#[cfg(test)]
mod copy_validation_tests;
mod environment;
mod error;
mod exceptions;
mod field_read;
#[cfg(test)]
mod field_update_tests;
mod float_loop;
mod integer_loop;
mod invocation;
mod map_builder;
#[cfg(test)]
mod map_builder_tests;
mod map_index;
mod method_cache;
#[cfg(test)]
mod method_cache_tests;
mod modules;
mod objects;
pub use invocation::{ExecutionOutcome, InvocationContext, InvocationError, InvocationResult};
mod native;
mod numeric_call;
mod numeric_call_loop;
mod ops;
mod output;
mod prepared_call;
#[cfg(test)]
mod test_support;
mod type_check;
pub use output::{OutputAction, ScriptOutput};
mod value;

use environment::Environments;
pub use error::{RuntimeError, SourceFile, StackFrame};
pub use native::{
    ContextOptions, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType,
    PermissionConfig, RuntimeContext,
};
use scr_ast::{
    AssignmentOp, BinaryOp, Block, Expr, ExprKind, Literal, Program, Statement, StatementKind,
    TypeAnnotation,
};
use scr_diagnostics::Span;
use std::{
    io::Write,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use value::Function;
pub use value::{Callable, MapKey, NativeObject, Value};

/// Hard ceiling for the recursive AST evaluator, even if a host requests more.
pub const MAX_RECURSION_LIMIT: usize = 64;
pub const MAX_VALUE_DEPTH: usize = 128;
pub const DEFAULT_MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    /// Maximum active script/native calls, excluding main (default 64).
    /// Values above MAX_RECURSION_LIMIT are capped for host stack safety.
    pub recursion_limit: usize,
    /// Host cancellation flag, checked at statements and loop back edges.
    pub interrupt_flag: Option<Arc<AtomicBool>>,
    /// Optional execution budget, reset for each execute call.
    pub instruction_limit: Option<u64>,
    pub context: ContextOptions,
    /// Logical bytes per value, including collection slots, string/byte contents, and reported native payloads.
    /// None preserves unrestricted payload sizes; this is not a process heap cap.
    pub max_value_bytes: Option<usize>,
    /// Live lexical scopes, including retained closure environments and globals.
    pub max_scopes: Option<usize>,
    pub max_source_bytes: usize,
    /// Cumulative generated output, including bytes later cleaned from buffers.
    pub max_output_bytes: Option<usize>,
    /// Total pending application-buffer payload (default 8 MiB, hard ceiling 64 MiB).
    pub max_output_buffer_bytes: usize,
    /// Maximum nested application buffers (default 32, hard ceiling 128).
    pub max_output_buffers: usize,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            recursion_limit: 64,
            interrupt_flag: None,
            instruction_limit: None,
            context: ContextOptions::default(),
            max_value_bytes: None,
            max_scopes: None,
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            max_output_bytes: None,
            max_output_buffer_bytes: 8 * 1024 * 1024,
            max_output_buffers: 32,
        }
    }
}

enum Signal {
    Error(Box<RuntimeError>),
    TypeError(Box<RuntimeError>),
    NativeException(NativeError, Box<RuntimeError>),
    OperationException(&'static str, Box<RuntimeError>),
    Interrupted(Box<RuntimeError>),
    Throw(Value, Box<RuntimeError>),
    Return(Value),
    Break,
    Continue,
    Exit(u8),
}
type Result<T> = std::result::Result<T, Signal>;
const MAX_ARGUMENT_BUFFERS: usize = MAX_RECURSION_LIMIT;
const MAX_ARGUMENT_CAPACITY: usize = 16;

#[derive(Default)]
pub struct Runtime {
    #[cfg(test)]
    disable_compact_calls: bool,
    #[cfg(test)]
    compact_calls_executed: usize,
    #[cfg(test)]
    compact_frame_calls_executed: usize,
    #[cfg(test)]
    force_compact_frames: bool,
    environments: Environments,
    map_indexes: std::cell::RefCell<map_index::MapIndexes>,
    exception_classes:
        std::cell::RefCell<std::collections::HashMap<String, Rc<objects::ScriptClass>>>,
    stack: Vec<error::ActiveFrame>,
    argument_buffers: Vec<Vec<Value>>,
    #[cfg(test)]
    disable_call_reuse: bool,
    #[cfg(test)]
    disable_shared_parameter_names: bool,
    #[cfg(test)]
    force_field_snapshot: bool,
    #[cfg(test)]
    force_hashed_fields: bool,
    #[cfg(test)]
    disable_method_cache: bool,
    #[cfg(test)]
    disable_field_reads: bool,
    #[cfg(test)]
    disable_map_literal_index: bool,
    #[cfg(test)]
    fail_map_literal_index_after: Option<usize>,
    identity: Rc<()>,
    options: RuntimeOptions,
    steps: u64,
    #[cfg(test)]
    disable_integer_loops: bool,
    #[cfg(test)]
    integer_loops_executed: usize,
    #[cfg(test)]
    float_loops_executed: usize,
    #[cfg(test)]
    disable_numeric_calls: bool,
    #[cfg(test)]
    force_numeric_calls: bool,
    #[cfg(test)]
    disable_integer_numeric_calls: bool,
    #[cfg(test)]
    integer_numeric_calls_executed: usize,
    #[cfg(test)]
    disable_numeric_annotations: bool,
    #[cfg(test)]
    disable_primitive_annotations: bool,
    #[cfg(test)]
    disable_numeric_parameter_scopes: bool,
    #[cfg(test)]
    numeric_parameter_scopes_reused: usize,
    #[cfg(test)]
    numeric_calls_executed: usize,
    #[cfg(test)]
    disable_prepared_calls: bool,
    #[cfg(test)]
    prepared_calls_executed: usize,
    #[cfg(test)]
    disable_numeric_call_loops: bool,
    #[cfg(test)]
    numeric_call_loops_executed: usize,
    class_resolution_depth: usize,
    #[cfg(test)]
    disable_copy_validation: bool,
    #[cfg(test)]
    copy_validations_executed: usize,
    error_stream: Option<Box<dyn Write>>,
    input_stream: Option<Box<dyn std::io::BufRead>>,
    source: Option<Arc<SourceFile>>,
    active_modules: Vec<std::path::PathBuf>,
    module_policy: Option<Rc<modules::ModulePolicy>>,
    module_resolver: Option<Rc<modules::ModuleResolver>>,
}

/// A resolved assignment target. Index expressions and object receivers are
/// evaluated once, before the RHS, even for compound assignment.
enum Root<'a> {
    Variable {
        scope: usize,
        name: &'a str,
        slot: usize,
    },
    Property {
        object: Rc<dyn NativeObject>,
        name: &'a str,
    },
}
struct Place<'a> {
    root: Root<'a>,
    path: Vec<Value>,
    append: bool,
}

impl Runtime {
    /// Opaque identity shared with native call contexts; contains no backend state.
    pub fn native_identity(&self) -> Rc<()> {
        self.identity.clone()
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(options: RuntimeOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    fn checkpoint(&mut self, span: Span) -> Result<()> {
        if self
            .options
            .interrupt_flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err(Signal::Interrupted(
                self.error("execution interrupted", span),
            ));
        }
        if self
            .options
            .instruction_limit
            .is_some_and(|limit| self.steps >= limit)
        {
            return Err(Signal::Interrupted(
                self.error("execution instruction limit exceeded", span),
            ));
        }
        self.environments.collect_if_needed();
        self.steps = self.steps.saturating_add(1);
        Ok(())
    }

    /// Reclaim unreachable captured lexical scopes and return the number reclaimed.
    /// Globals, active calls, and callable Values retained by hosts stay alive.
    /// Native-held callbacks are conservative roots; opaque native ownership cycles
    /// are not collected. Freed storage may remain resident in the allocator.
    pub fn collect_garbage(&mut self) -> usize {
        self.environments.collect()
    }

    /// Register a native implementation with a required positional signature.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        function: NativeFunction,
    ) -> NativeResult<()> {
        self.register_native(name, function)
    }

    pub fn register_native(
        &mut self,
        name: impl Into<String>,
        function: NativeFunction,
    ) -> NativeResult<()> {
        let name = name.into();
        const RESERVED: &[&str] = &[
            "echo",
            "exit",
            "if",
            "elseif",
            "else",
            "while",
            "for",
            "foreach",
            "as",
            "function",
            "class",
            "interface",
            "implements",
            "extends",
            "new",
            "in",
            "return",
            "break",
            "continue",
            "try",
            "catch",
            "finally",
            "throw",
            "true",
            "false",
            "null",
            "require",
            "require_once",
            "and",
            "or",
            "xor",
        ];
        if name.split("::").count() > 2
            || !name.split("::").all(|part| {
                let mut chars = part.chars();
                !RESERVED.contains(&part)
                    && chars
                        .next()
                        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
                    && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
            })
        {
            return Err(NativeError::new(
                "RegistryError",
                format!("invalid native function name {name}"),
            ));
        }
        if let Some((namespace, _)) = name.split_once("::") {
            if self.environments.has_alias(namespace) {
                return Err(NativeError::new(
                    "RegistryError",
                    format!("namespace {namespace} is already a module alias"),
                ));
            }
        }
        let functions = &mut self.environments.scope_mut(0).functions;
        if functions.contains_key(&name) {
            return Err(NativeError::new(
                "RegistryError",
                format!("function {name} is already registered"),
            ));
        }
        let callable = Value::Callable(Callable(Rc::new(Function::Native {
            name: Rc::from(name.as_str()),
            function,
        })));
        functions.insert(name, callable);
        Ok(())
    }

    /// Set the native diagnostic stream. Script echo and native stdout share
    /// the writer passed to execute; native stderr defaults to process stderr.
    pub fn set_error_stream(&mut self, stream: impl Write + 'static) {
        self.error_stream = Some(Box::new(stream));
    }

    pub fn set_input_stream(&mut self, stream: impl std::io::BufRead + 'static) {
        self.input_stream = Some(Box::new(stream));
    }

    fn native_error(&self, error: NativeError, span: Span) -> Signal {
        let runtime_error = self.error(error.to_string(), span);
        if error.kind != "Interrupted" {
            self.builtin_exception_class(&error.kind);
        }
        match error.kind.as_str() {
            "TypeError" => Signal::TypeError(runtime_error),
            "Interrupted" => Signal::Interrupted(runtime_error),
            _ => Signal::NativeException(error, runtime_error),
        }
    }

    fn property_result<T>(&self, result: std::result::Result<T, String>, span: Span) -> Result<T> {
        result
            .map_err(|message| self.native_error(NativeError::new("PropertyError", message), span))
    }

    pub fn get_global(&self, name: &str) -> Option<&Value> {
        self.environments.scope(0).variables.get(name)
    }

    pub fn set_global(
        &mut self,
        name: impl Into<String>,
        value: Value,
    ) -> std::result::Result<(), RuntimeError> {
        value
            .measure(0, self.options.max_value_bytes)
            .map_err(|message| *self.error(message, Span::default()))?;
        let name = name.into();
        self.map_indexes.get_mut().invalidate(0, &name);
        self.environments.scope_mut(0).variables.insert(name, value);
        Ok(())
    }

    /// Execute in this runtime's global environment. Exit is returned to the host;
    /// neither script exit nor script errors terminate the embedding process.
    pub fn execute(
        &mut self,
        program: &Program,
        output: &mut impl Write,
    ) -> std::result::Result<u8, RuntimeError> {
        self.execute_outcome(program, output)
            .map(|outcome| match outcome {
                ExecutionOutcome::Completed => 0,
                ExecutionOutcome::Exited(code) => code,
            })
    }

    /// Execute while distinguishing normal completion from explicit exit 0.
    pub fn execute_outcome(
        &mut self,
        program: &Program,
        output: &mut impl Write,
    ) -> std::result::Result<ExecutionOutcome, RuntimeError> {
        let mut output = output::BufferedOutput::new(output, &self.options)
            .map_err(|e| *self.error(e.to_string(), Span::default()))?;
        let result = self.execute_inner(program, &mut output);
        if result.is_ok() {
            output
                .finish()
                .map_err(|e| *self.error(e.to_string(), Span::default()))?;
        }
        result
    }
    fn execute_inner(
        &mut self,
        program: &Program,
        output: &mut dyn ScriptOutput,
    ) -> std::result::Result<ExecutionOutcome, RuntimeError> {
        self.collect_garbage();
        self.steps = 0;
        if self
            .options
            .max_scopes
            .is_some_and(|limit| self.environments.live > limit)
        {
            return Err(*self.error("execution scope limit exceeded", Span::default()));
        }
        let entry = self
            .source
            .as_ref()
            .map(|s| s.path.as_path())
            .or(self.options.context.script_path.as_deref())
            .and_then(|path| std::fs::canonicalize(path).ok());
        if let Some(path) = &entry {
            self.active_modules.push(path.clone());
        }
        self.stack.push(
            StackFrame {
                function: "<main>".into(),
                source: self.source.clone(),
                span: program
                    .statements
                    .first()
                    .map_or(Span::default(), |statement| statement.span),
            }
            .into(),
        );
        let result = (|| {
            // Only direct top-level declarations are hoisted. Conditional and
            // nested declarations retain their execution-dependent behavior.
            for statement in &program.statements {
                if let StatementKind::Interface(interface) = &statement.kind {
                    self.declare_interface(interface, statement.span)?;
                }
            }
            for statement in &program.statements {
                match &statement.kind {
                    StatementKind::Class(class) => self.declare_class(class, statement.span)?,
                    StatementKind::Function(_) => self.statement(statement, output)?,
                    _ => {}
                }
            }
            for statement in &program.statements {
                if let StatementKind::Class(class) = &statement.kind {
                    self.check_class(&class.name.name, statement.span)?;
                }
                if !matches!(
                    statement.kind,
                    StatementKind::Function(_)
                        | StatementKind::Class(_)
                        | StatementKind::Interface(_)
                ) {
                    self.statement(statement, output)?;
                }
            }
            Ok(())
        })();
        let result = match result {
            Ok(()) => Ok(ExecutionOutcome::Completed),
            Err(Signal::Exit(code)) => Ok(ExecutionOutcome::Exited(code)),
            Err(
                Signal::Error(error)
                | Signal::TypeError(error)
                | Signal::Interrupted(error)
                | Signal::Throw(_, error)
                | Signal::NativeException(_, error)
                | Signal::OperationException(_, error),
            ) => Err(*error),
            Err(_) => Err(*self.error(
                "invalid control flow outside a function or loop",
                Span::default(),
            )),
        };
        self.stack.pop();
        if let Some(path) = entry {
            self.active_modules.pop();
            if result.is_ok() {
                self.environments.scope_mut(0).loaded_modules.insert(path);
            }
        }
        self.collect_garbage();
        result
    }

    fn error(&self, message: impl Into<String>, span: Span) -> Box<RuntimeError> {
        let mut secrets = self.options.context.redacted_values.clone();
        secrets.extend(
            self.options
                .context
                .environment
                .iter()
                .filter(|(key, _)| scr_diagnostics::sensitive_name(&key.to_string_lossy()))
                .filter_map(|(_, value)| {
                    value
                        .to_str()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                }),
        );
        Box::new(RuntimeError {
            message: scr_diagnostics::redact_text(&message.into(), &secrets),
            span,
            trace: self
                .stack
                .iter()
                .map(error::ActiveFrame::snapshot)
                .collect(),
            code: "E0401",
            help: None,
            redacted_values: Arc::new(secrets),
            source: self.source.clone(),
        })
    }
    fn fail<T>(&self, message: impl Into<String>, span: Span) -> Result<T> {
        Err(Signal::Error(self.error(message, span)))
    }
    fn checked<T>(&self, result: std::result::Result<T, String>, span: Span) -> Result<T> {
        result.map_err(|message| Signal::Error(self.error(message, span)))
    }
    fn operation<T>(&self, result: ops::Result<T>, span: Span) -> Result<T> {
        result.map_err(|error| {
            let (kind, message) = match error {
                ops::Error::Type(message) => ("TypeError", message),
                ops::Error::Index(message) => ("IndexError", message),
                ops::Error::Arithmetic(message) => ("ArithmeticError", message),
                ops::Error::Fatal(message) => return Signal::Error(self.error(message, span)),
            };
            let error = self.error(format!("{kind}: {message}"), span);
            if kind == "TypeError" {
                Signal::TypeError(error)
            } else {
                Signal::OperationException(kind, error)
            }
        })
    }
    fn condition(&mut self, expr: &Expr, output: &mut dyn ScriptOutput) -> Result<bool> {
        let value = self.evaluate(expr, output)?;
        self.operation(ops::boolean(value), expr.span)
    }
    fn statements(
        &mut self,
        statements: &[Statement],
        output: &mut dyn ScriptOutput,
    ) -> Result<()> {
        for statement in statements {
            self.statement(statement, output)?;
        }
        Ok(())
    }
    fn block(&mut self, block: &Block, output: &mut dyn ScriptOutput) -> Result<()> {
        let previous = self.enter_empty_scope(self.environments.current, block.span)?;
        let result = self.statements(&block.statements, output);
        self.environments.leave_block(previous);
        result
    }
    fn expressions(
        &mut self,
        expressions: &[Expr],
        output: &mut dyn ScriptOutput,
    ) -> Result<Option<Value>> {
        let mut value = None;
        for expression in expressions {
            value = Some(self.evaluate(expression, output)?);
        }
        Ok(value)
    }

    fn statement(&mut self, statement: &Statement, output: &mut dyn ScriptOutput) -> Result<()> {
        self.checkpoint(statement.span)?;
        match &statement.kind {
            StatementKind::Class(class) => {
                self.declare_class(class, statement.span)?;
                self.check_class(&class.name.name, statement.span)?;
            }
            StatementKind::Interface(interface) => {
                self.declare_interface(interface, statement.span)?
            }
            StatementKind::Require { path, once, alias } => {
                let value = self.evaluate(path, output)?;
                let Value::String(path_value) = value else {
                    return Err(self.native_error(
                        NativeError::type_error("require path must be a string"),
                        path.span,
                    ));
                };
                self.require_module(&path_value, *once, alias.as_ref(), statement.span, output)?;
            }
            StatementKind::Expression(expr) => {
                if let ExprKind::Assignment(target, op, rhs) = &expr.kind {
                    #[cfg(test)]
                    let enabled = !self.disable_copy_validation;
                    #[cfg(not(test))]
                    let enabled = true;
                    if enabled
                        && *op == AssignmentOp::Assign
                        && matches!(&target.kind, ExprKind::Variable(_))
                        && matches!(&rhs.kind, ExprKind::Variable(_))
                    {
                        copy_validation::assignment(self, target, rhs, output)?;
                    } else {
                        self.assignment(target, *op, rhs, false, output)?;
                    }
                } else {
                    self.evaluate(expr, output)?;
                }
            }
            StatementKind::Echo(expr) => {
                let Value::String(value) = self.evaluate(expr, output)? else {
                    return self.fail("echo requires a string; use str(value)", expr.span);
                };
                self.checked(
                    output
                        .write_all(value.as_bytes())
                        .map_err(|error| format!("output failed: {error}")),
                    expr.span,
                )?;
            }
            StatementKind::Block(block) => self.block(block, output)?,
            StatementKind::If {
                branches,
                else_branch,
            } => {
                for (condition, body) in branches {
                    if self.condition(condition, output)? {
                        return self.statement(body, output);
                    }
                }
                if let Some(body) = else_branch {
                    self.statement(body, output)?;
                }
            }
            StatementKind::While { condition, body } => {
                while self.condition(condition, output)? {
                    self.checkpoint(statement.span)?;
                    match self.statement(body, output) {
                        Ok(()) | Err(Signal::Continue) => {}
                        Err(Signal::Break) => break,
                        Err(signal) => return Err(signal),
                    }
                }
            }
            StatementKind::For {
                initializer,
                condition,
                increment,
                body,
            } => {
                let previous = self.enter_scope(self.environments.current, statement.span)?;
                let result = (|| {
                    self.expressions(initializer, output)?;
                    if let Some(result) =
                        self.integer_loop(condition, increment, body, statement.span)
                    {
                        return result;
                    }
                    if let Some(result) =
                        self.float_loop(condition, increment, body, statement.span)
                    {
                        return result;
                    }
                    if let Some(result) =
                        self.numeric_call_loop(condition, increment, body, statement.span, output)
                    {
                        return result;
                    }
                    loop {
                        self.checkpoint(statement.span)?;
                        if let Some(value) = self.expressions(condition, output)? {
                            if !self.operation(ops::boolean(value), statement.span)? {
                                break;
                            }
                        }
                        match self.statement(body, output) {
                            Ok(()) | Err(Signal::Continue) => {}
                            Err(Signal::Break) => break,
                            Err(signal) => return Err(signal),
                        }
                        self.expressions(increment, output)?;
                    }
                    Ok(())
                })();
                self.environments.leave(previous);
                result?;
            }
            StatementKind::Foreach {
                iterable,
                key,
                value,
                body,
            } => {
                let entries: Vec<_> = match self.evaluate(iterable, output)? {
                    Value::List(items) => items
                        .into_iter()
                        .enumerate()
                        .map(|(index, value)| (Value::Int(index as i64), value))
                        .collect(),
                    Value::Map(entries) => entries
                        .into_iter()
                        .map(|(key, value)| (key.value(), value))
                        .collect(),
                    _ => return self.fail("foreach requires a list or map", iterable.span),
                };
                let previous = self.enter_scope(self.environments.current, statement.span)?;
                let result = (|| {
                    for (entry_key, entry_value) in entries {
                        self.checkpoint(statement.span)?;
                        let scope = self.environments.scope_mut(self.environments.current);
                        if let Some(key) = key {
                            scope.variables.insert(key.name.clone(), entry_key);
                        }
                        scope.variables.insert(value.name.clone(), entry_value);
                        match self.statement(body, output) {
                            Ok(()) | Err(Signal::Continue) => {}
                            Err(Signal::Break) => break,
                            Err(signal) => return Err(signal),
                        }
                    }
                    Ok(())
                })();
                self.environments.leave(previous);
                result?;
            }
            StatementKind::Break => return Err(Signal::Break),
            StatementKind::Continue => return Err(Signal::Continue),
            StatementKind::Function(declaration) => {
                let scope = self.environments.current;
                if self
                    .environments
                    .scope(scope)
                    .functions
                    .contains_key(&declaration.name)
                {
                    return self.fail(
                        format!(
                            "function {} is already declared in this scope",
                            declaration.name
                        ),
                        declaration.name_span,
                    );
                }
                let function = self.user_function(declaration)?;
                self.environments
                    .scope_mut(scope)
                    .functions
                    .insert(declaration.name.clone(), function);
            }
            StatementKind::Return(expr) => {
                let value = if let Some(expr) = expr {
                    self.evaluate(expr, output)?
                } else {
                    Value::Null
                };
                return Err(Signal::Return(value));
            }
            StatementKind::Try {
                body,
                catches,
                finally,
            } => {
                let pending = (|| {
                    match self.block(body, output) {
                        Err(
                            signal @ (Signal::Throw(..)
                            | Signal::TypeError(_)
                            | Signal::OperationException(..)
                            | Signal::NativeException(..)),
                        ) => {
                            for catch in catches {
                                if catch.exception_type.as_ref().is_none_or(|annotation| {
                                    self.signal_matches(&signal, &annotation.name)
                                }) {
                                    let value = match &signal {
                                        Signal::Throw(value, _) => value.clone(),
                                        Signal::TypeError(error) => self.exception_value(
                                            "TypeError",
                                            error,
                                            statement.span,
                                        )?,
                                        Signal::NativeException(error, diagnostic) => self
                                            .exception_value(
                                                &error.kind,
                                                diagnostic,
                                                statement.span,
                                            )?,
                                        Signal::OperationException(kind, error) => {
                                            self.exception_value(kind, error, statement.span)?
                                        }
                                        _ => unreachable!(),
                                    };
                                    self.validate_value(&value, 0, statement.span)?;
                                    let previous = self
                                        .enter_scope(self.environments.current, statement.span)?;
                                    self.environments
                                        .scope_mut(self.environments.current)
                                        .variables
                                        .insert(catch.binding.name.clone(), value);
                                    let result = self.statements(&catch.body.statements, output);
                                    self.environments.leave(previous);
                                    return result;
                                }
                            }
                            return Err(signal);
                        }
                        result => result?,
                    }
                    Ok(())
                })();
                if matches!(&pending, Err(Signal::Error(_) | Signal::Interrupted(_))) {
                    return pending;
                }
                if let Some(body) = finally {
                    self.block(body, output)?;
                }
                pending?;
            }
            StatementKind::Throw(expr) => {
                let value = self.evaluate(expr, output)?;
                return self.throw_value(value, expr.span);
            }
            StatementKind::Exit(expr) => {
                let code = match expr {
                    None => 0,
                    Some(expr) => match self.evaluate(expr, output)? {
                        Value::Int(code) if (0..=255).contains(&code) => code as u8,
                        _ => {
                            return self
                                .fail("exit code must be an int between 0 and 255", expr.span);
                        }
                    },
                };
                return Err(Signal::Exit(code));
            }
        }
        Ok(())
    }

    fn evaluate(&mut self, expr: &Expr, output: &mut dyn ScriptOutput) -> Result<Value> {
        match &expr.kind {
            ExprKind::Literal(literal) => {
                let value = match literal {
                    Literal::Null => Value::Null,
                    Literal::Bool(value) => Value::Bool(*value),
                    Literal::Int(value) => Value::Int(*value),
                    Literal::Float(value) => Value::Float(*value),
                    Literal::String(value) => Value::String(value.clone()),
                };
                self.validate_value(&value, 0, expr.span)?;
                Ok(value)
            }
            ExprKind::InterpolatedString(parts) => {
                let mut text = String::new();
                for part in parts {
                    self.checkpoint(part.span)?;
                    let value = self.evaluate(part, output)?;
                    let piece = match value {
                        Value::String(value) => value,
                        Value::Int(value) => value.to_string(),
                        Value::Float(value) if value.is_finite() => value.to_string(),
                        Value::Bool(value) => value.to_string(),
                        value => return Err(self.native_error(
                            NativeError::type_error(format!(
                                "string interpolation requires string, int, float, or bool; got {}",
                                value.type_name()
                            )),
                            part.span,
                        )),
                    };
                    let bytes = text
                        .len()
                        .checked_add(piece.len())
                        .and_then(|n| n.checked_add(std::mem::size_of::<Value>()));
                    if bytes.is_none_or(|bytes| {
                        self.options
                            .max_value_bytes
                            .is_some_and(|limit| bytes > limit)
                    }) {
                        return self.fail("execution value byte limit exceeded", expr.span);
                    }
                    text.try_reserve(piece.len()).map_err(|_| {
                        Signal::Error(
                            self.error("string interpolation allocation failed", expr.span),
                        )
                    })?;
                    text.push_str(&piece);
                }
                Ok(Value::String(text))
            }
            ExprKind::Variable(name) => {
                let Some((_, value)) = self.environments.lookup(name, false) else {
                    return self.fail(format!("undefined variable ${name}"), expr.span);
                };
                Ok(value.clone())
            }
            ExprKind::Identifier(name) => self.function(name, expr.span),
            ExprKind::Function(declaration) => self.user_function(declaration),
            ExprKind::Grouped(inner) => self.evaluate(inner, output),
            ExprKind::Unary(op, operand) => {
                let value = self.evaluate(operand, output)?;
                self.operation(ops::unary(*op, value), expr.span)
            }
            ExprKind::Binary(left, op, right) => {
                let left = self.evaluate(left, output)?;
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    let left = self.operation(ops::boolean(left), expr.span)?;
                    if (*op == BinaryOp::And && !left) || (*op == BinaryOp::Or && left) {
                        return Ok(Value::Bool(left));
                    }
                    let right = self.evaluate(right, output)?;
                    return self
                        .operation(ops::boolean(right), expr.span)
                        .map(Value::Bool);
                }
                let right = self.evaluate(right, output)?;
                if *op == BinaryOp::In {
                    return self.membership(&left, &right, expr.span);
                }
                if *op == BinaryOp::Concat {
                    if let (Value::String(a), Value::String(b), Some(limit)) =
                        (&left, &right, self.options.max_value_bytes)
                    {
                        let bytes = a
                            .len()
                            .checked_add(b.len())
                            .and_then(|n| n.checked_add(std::mem::size_of::<Value>()));
                        if bytes.is_none_or(|bytes| bytes > limit) {
                            return self.fail("execution value byte limit exceeded", expr.span);
                        }
                    }
                }
                self.operation(ops::binary(*op, left, right), expr.span)
            }
            ExprKind::Slice {
                base,
                start,
                stop,
                step,
            } => self.slice(
                base,
                start.as_deref(),
                stop.as_deref(),
                step.as_deref(),
                expr.span,
                output,
            ),
            ExprKind::Comprehension {
                value,
                binding,
                iterable,
                condition,
            } => self.comprehension(
                value,
                binding,
                iterable,
                condition.as_deref(),
                expr.span,
                output,
            ),
            ExprKind::Spread(_) => self.fail(
                "spread is only valid inside lists, call arguments, or destructuring",
                expr.span,
            ),
            ExprKind::New {
                namespace,
                name,
                arguments,
            } => self.construct(namespace.as_deref(), name, arguments, expr.span, output),
            ExprKind::Assignment(target, op, rhs) => {
                self.assignment(target, *op, rhs, true, output)
            }
            ExprKind::Call(callee, args) => {
                if let ExprKind::PropertyAccess(base, name) = &callee.kind {
                    return self.call_property(base, name, args, callee.span, expr.span, output);
                }
                let callee = self.evaluate(callee, output)?;
                self.call(callee, args, expr.span, output)
            }
            ExprKind::StaticCall {
                namespace,
                name,
                arguments,
            } => {
                let ExprKind::Identifier(namespace) = &namespace.kind else {
                    return self.fail("invalid namespace", expr.span);
                };
                let callable = if let Some(alias) = self.environments.find_alias(namespace) {
                    self.environments
                        .scope(alias.scope)
                        .functions
                        .get(name)
                        .cloned()
                        .ok_or_else(|| {
                            self.native_error(
                                NativeError::new(
                                    "ModuleError",
                                    format!("module alias {namespace} has no function {name}"),
                                ),
                                expr.span,
                            )
                        })?
                } else {
                    self.function(&format!("{namespace}::{name}"), expr.span)?
                };
                self.call(callable, arguments, expr.span, output)
            }
            ExprKind::PropertyAccess(base, name) => {
                let Value::NativeObject(object) = self.evaluate(base, output)? else {
                    return self.fail("property access requires a native object", expr.span);
                };
                let value = self.object_property(&object, name, expr.span)?;
                self.validate_value(&value, 0, expr.span)?;
                Ok(value)
            }
            ExprKind::IndexAccess(_, _) if borrowable(expr) => {
                let place = self.place(expr, output)?;
                self.read_place(&place, expr.span)
            }
            ExprKind::IndexAccess(base, index) => {
                let base = self.evaluate(base, output)?;
                let index = self.evaluate(index, output)?;
                self.operation(ops::index(&base, &index), expr.span)
            }
            ExprKind::AppendAccess(_) => {
                self.fail("cannot read from a list append target", expr.span)
            }
            ExprKind::List(items) => self
                .expanded_values(items, expr.span, output)
                .map(Value::List),
            ExprKind::Map(entries) => self.map_literal(entries, expr.span, output),
        }
    }

    fn function(&self, name: &str, span: Span) -> Result<Value> {
        let Some((_, value)) = self.environments.lookup(name, true) else {
            // Isolated modules have no caller/global lexical parent. Only host
            // native registrations remain visible across that boundary.
            if let Some(value @ Value::Callable(callable)) =
                self.environments.scope(0).functions.get(name)
            {
                if matches!(callable.0.as_ref(), Function::Native { .. }) {
                    return Ok(value.clone());
                }
            }
            return self.fail(format!("undefined function {name}"), span);
        };
        Ok(value.clone())
    }

    fn validate_function(&self, declaration: &scr_ast::FunctionDeclaration) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        let mut optional = false;
        for (index, parameter) in declaration.parameters.iter().enumerate() {
            if parameter.binding.name == "this" {
                return self.fail("$this is reserved for method receivers", parameter.span);
            }
            if !names.insert(&parameter.binding.name) {
                return self.fail("duplicate parameter name", parameter.span);
            }
            if parameter.variadic {
                if parameter.default.is_some() || index + 1 != declaration.parameters.len() {
                    return self.fail(
                        "variadic parameter must be last and cannot have a default",
                        parameter.span,
                    );
                }
            } else if parameter.default.is_some() {
                optional = true;
            } else if optional {
                return self.fail(
                    "required parameters must precede default parameters",
                    parameter.span,
                );
            }
        }
        Ok(())
    }

    fn user_function(&mut self, declaration: &scr_ast::FunctionDeclaration) -> Result<Value> {
        self.validate_function(declaration)?;
        self.environments.capture();
        let function = Rc::new(Function::User {
            declaration: Rc::new(value::UserDeclaration::new(declaration.clone())),
            receiver: None,
            environment: self.environments.current,
            owner: self.identity.clone(),
            source: self.source.clone(),
        });
        self.environments.track_function(&function);
        Ok(Value::Callable(Callable(function)))
    }

    fn call(
        &mut self,
        callee: Value,
        args: &[Expr],
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let Value::Callable(callable) = callee else {
            return self.fail("call requires a callable value", span);
        };
        let values = self.evaluate_arguments(args, span, output)?;
        self.call_callable(callable, values, span, output)
    }

    fn call_callable(
        &mut self,
        callable: Callable,
        values: Vec<Value>,
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        self.call_function(callable.0.as_ref(), values, span, output)
    }

    fn take_argument_buffer(&mut self) -> Vec<Value> {
        #[cfg(test)]
        if self.disable_call_reuse {
            return Vec::new();
        }
        self.argument_buffers.pop().unwrap_or_default()
    }

    fn call_function(
        &mut self,
        function: &Function,
        mut values: Vec<Value>,
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let result = self.call_function_inner(function, &mut values, span, output);
        // Never retain argument values/roots. Suspended callers keep ownership
        // of their own buffers through recursive or reentrant calls.
        values.clear();
        #[cfg(not(test))]
        let reuse = true;
        #[cfg(test)]
        let reuse = !self.disable_call_reuse;
        if reuse
            && (1..=MAX_ARGUMENT_CAPACITY).contains(&values.capacity())
            && self.argument_buffers.len() < MAX_ARGUMENT_BUFFERS
        {
            self.argument_buffers.push(values);
        }
        result
    }

    fn call_function_inner(
        &mut self,
        function: &Function,
        values: &mut Vec<Value>,
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        if self.stack.len().saturating_sub(1)
            >= self.options.recursion_limit.min(MAX_RECURSION_LIMIT)
        {
            return self.fail("maximum call depth exceeded", span);
        }
        let frame = error::ActiveFrame::call(function, span, self.source.clone());
        #[cfg(test)]
        let frame = if self.disable_call_reuse {
            frame.snapshot().into()
        } else {
            frame
        };
        self.stack.push(frame);
        let result = match function {
            Function::Native { name, function } if function.is_reentrant() => {
                let mut context = InvocationContext {
                    runtime: self,
                    output,
                    span,
                };
                function
                    .invoke_reentrant(name, &mut context, values)
                    .map_err(|error| error.signal)
            }
            Function::Native { name, function } => {
                let settings = &mut self.options.context;
                let mut context = RuntimeContext {
                    working_directory: &mut settings.working_directory,
                    script_path: settings.script_path.as_deref(),
                    cli_arguments: &settings.cli_arguments,
                    environment: &mut settings.environment,
                    permissions: &settings.permissions,
                    cancellation: self.options.interrupt_flag.clone(),
                    runtime_identity: Some(self.identity.clone()),
                    call_span: span,
                    max_value_bytes: self.options.max_value_bytes,
                    output,
                    input: self
                        .input_stream
                        .get_or_insert_with(|| Box::new(std::io::BufReader::new(std::io::stdin())))
                        .as_mut(),
                    error: self
                        .error_stream
                        .get_or_insert_with(|| Box::new(std::io::stderr()))
                        .as_mut(),
                };
                function
                    .invoke(name, &mut context, values)
                    .map_err(|error| self.native_error(error, span))
            }
            Function::User {
                declaration,
                environment,
                owner,
                source,
                receiver,
            } => {
                if !Rc::ptr_eq(owner, &self.identity) {
                    self.stack.pop();
                    return self.fail("user callable belongs to a different runtime", span);
                }
                let required = declaration.required;
                let variadic = declaration.variadic;
                let maximum = declaration.parameters.len();
                if values.len() < required || (!variadic && values.len() > maximum) {
                    let expected = if variadic {
                        format!("at least {required} arguments")
                    } else if required != maximum {
                        format!("between {required} and {maximum} arguments")
                    } else {
                        format!("{required} arguments")
                    };
                    let error = self.error(
                        format!(
                            "TypeError: {}() expects {}, got {}",
                            declaration.name,
                            expected,
                            values.len()
                        ),
                        span,
                    );
                    self.stack.pop();
                    return Err(Signal::TypeError(error));
                }
                let caller_source = self.source.clone();
                self.source = source.clone();
                let reuse_parameters = receiver.is_none()
                    && self.options.max_value_bytes.is_none()
                    && self.can_reuse_numeric_parameter_scope(declaration, values);
                let parameter_scope = if reuse_parameters {
                    self.enter_empty_scope(*environment, span)
                } else {
                    self.enter_scope(*environment, span)
                };
                let previous = match parameter_scope {
                    Ok(previous) => previous,
                    Err(error) => {
                        self.source = caller_source;
                        self.stack.pop();
                        return Err(error);
                    }
                };
                self.environments
                    .scope_mut(self.environments.current)
                    .function_boundary = true;
                let result = (|| {
                    #[cfg(test)]
                    if reuse_parameters {
                        self.numeric_parameter_scopes_reused += 1;
                    }
                    if receiver.is_none() && self.options.max_value_bytes.is_none() {
                        if let Some(result) = self.numeric_call(declaration, values) {
                            return result;
                        }
                        if let Some(result) = self.prepared_call(function, declaration, values) {
                            return result;
                        }
                        if *environment == 0 {
                            if let Some(result) = self.compact_call(function, declaration, values) {
                                return result;
                            }
                        }
                    }
                    #[cfg(not(test))]
                    let shared = true;
                    #[cfg(test)]
                    let shared = !self.disable_shared_parameter_names;
                    if let Some(receiver) = receiver {
                        let variables = &mut self
                            .environments
                            .scope_mut(self.environments.current)
                            .variables;
                        if shared {
                            variables
                                .insert_parameter(declaration.receiver_name(), receiver.clone());
                        } else {
                            variables.insert("this".into(), receiver.clone());
                        }
                    }
                    let mut next_argument = 0;
                    for (index, parameter) in declaration.parameters.iter().enumerate() {
                        let value = if parameter.variadic {
                            let items = if next_argument == values.len() {
                                Vec::new()
                            } else if values.capacity()
                                > (values.len() - next_argument).saturating_mul(2).max(4)
                            {
                                // A short retained tail must not keep a large
                                // argument allocation, including a pooled one.
                                values.drain(next_argument..).collect()
                            } else {
                                values.drain(..next_argument);
                                std::mem::take(values)
                            };
                            for value in &items {
                                self.annotation(
                                    &parameter.annotation,
                                    value,
                                    parameter.span,
                                    false,
                                )?;
                            }
                            let value = Value::List(items);
                            self.validate_value(&value, 0, parameter.span)?;
                            value
                        } else {
                            let value = if let Some(value) = values.get_mut(next_argument) {
                                next_argument += 1;
                                std::mem::replace(value, Value::Null)
                            } else {
                                self.evaluate(
                                    parameter.default.as_ref().expect("validated arity"),
                                    output,
                                )?
                            };
                            self.annotation(&parameter.annotation, &value, parameter.span, false)?;
                            value
                        };
                        let variables = &mut self
                            .environments
                            .scope_mut(self.environments.current)
                            .variables;
                        let name = if shared {
                            declaration.parameter_name(index)
                        } else {
                            None
                        };
                        if let Some(name) = name {
                            variables.insert_parameter(name, value);
                        } else {
                            variables.insert(parameter.binding.name.clone(), value);
                        }
                    }
                    let value = match self.block(&declaration.body, output) {
                        Ok(()) => Value::Null,
                        Err(Signal::Return(value)) => value,
                        Err(Signal::Break | Signal::Continue) => {
                            return self.fail("loop control cannot escape a function", span);
                        }
                        Err(signal) => return Err(signal),
                    };
                    self.annotation(
                        &declaration.return_type,
                        &value,
                        declaration.name_span,
                        true,
                    )?;
                    Ok(value)
                })();
                if reuse_parameters {
                    self.environments
                        .scope_mut(self.environments.current)
                        .function_boundary = false;
                    self.environments.leave_empty(previous);
                } else {
                    self.environments.leave_parameter(previous);
                }
                self.source = caller_source;
                result
            }
        };
        self.stack.pop();
        if self.options.max_value_bytes.is_some() {
            let value = result?;
            self.validate_value(&value, 0, span)?;
            Ok(value)
        } else {
            result
        }
    }

    fn annotation(
        &self,
        annotation: &Option<TypeAnnotation>,
        value: &Value,
        span: Span,
        returning: bool,
    ) -> Result<()> {
        if let Some(annotation) = annotation {
            #[cfg(not(test))]
            let fast = true;
            #[cfg(test)]
            let fast = !self.disable_primitive_annotations;
            // These exact names cannot be declared as classes/interfaces.
            // Native objects, qualified names, and mismatches still use normal
            // resolution, including its downcasts and diagnostic ordering.
            if fast
                && (matches!(
                    (annotation.name.as_str(), value),
                    ("int", Value::Int(_))
                        | ("float", Value::Float(_))
                        | ("string", Value::String(_))
                        | ("bytes", Value::Bytes(_))
                        | ("bool", Value::Bool(_))
                        | ("list", Value::List(_))
                        | ("map", Value::Map(_))
                        | ("callable", Value::Callable(_))
                ) || (returning && annotation.name == "void" && matches!(value, Value::Null)))
            {
                return Ok(());
            }
            let (namespace, name) = annotation
                .name
                .split_once("::")
                .map_or((None, annotation.name.as_str()), |(ns, name)| {
                    (Some(ns), name)
                });
            let instance = if let Value::NativeObject(object) = value {
                object
                    .as_any()
                    .and_then(|o| o.downcast_ref::<objects::ScriptObject>())
            } else {
                None
            };
            let matches = if let Some(class) = self.class(namespace, name) {
                instance.is_some_and(|instance| {
                    (instance.class.is_a(&class)
                        || (class.is_interface
                            && instance
                                .class
                                .interfaces
                                .get()
                                .into_iter()
                                .flatten()
                                .any(|interface| Rc::ptr_eq(interface, &class))))
                        && Rc::ptr_eq(&instance.class.owner, &self.identity)
                })
            } else {
                instance.is_none()
                    && (annotation.name == value.type_name()
                        || (returning && annotation.name == "void" && *value == Value::Null))
            };
            if !matches {
                return Err(Signal::TypeError(self.error(
                    format!(
                        "TypeError: {} type annotation requires {}, got {}",
                        if returning { "return" } else { "argument" },
                        annotation.name,
                        value.type_name()
                    ),
                    span,
                )));
            }
        }
        Ok(())
    }

    fn place<'a>(&mut self, target: &'a Expr, output: &mut dyn ScriptOutput) -> Result<Place<'a>> {
        match &target.kind {
            ExprKind::Grouped(inner) => self.place(inner, output),
            ExprKind::Variable(name) if name == "this" => {
                self.fail("$this cannot be reassigned", target.span)
            }
            ExprKind::Variable(name) => {
                let (scope, slot) = self
                    .environments
                    .resolve_variable(name)
                    .map(|(scope, slot, _)| (scope, slot))
                    .unwrap_or((self.environments.current, usize::MAX));
                Ok(Place {
                    root: Root::Variable { scope, name, slot },
                    path: Vec::new(),
                    append: false,
                })
            }
            ExprKind::IndexAccess(base, index) => {
                let mut place = self.place(base, output)?;
                // Reading the base rejects implicit creation of indexed variables.
                self.check_place(&place, base.span)?;
                place.path.push(self.evaluate(index, output)?);
                Ok(place)
            }
            ExprKind::AppendAccess(base) => {
                let mut place = self.place(base, output)?;
                let is_list = match self.variable_ref(&place, base.span)? {
                    Some(value) => matches!(value, Value::List(_)),
                    None => matches!(self.read_place(&place, base.span)?, Value::List(_)),
                };
                if !is_list {
                    return self.operation(
                        Err(ops::Error::Type("list append requires a list")),
                        target.span,
                    );
                }
                place.append = true;
                Ok(place)
            }
            ExprKind::PropertyAccess(base, name) => {
                let Value::NativeObject(object) = self.evaluate(base, output)? else {
                    return self.fail("property assignment requires a native object", target.span);
                };
                Ok(Place {
                    root: Root::Property { object, name },
                    path: Vec::new(),
                    append: false,
                })
            }
            _ => self.fail("invalid assignment target", target.span),
        }
    }

    fn variable_ref<'a>(&'a self, place: &Place<'_>, span: Span) -> Result<Option<&'a Value>> {
        let Root::Variable { scope, name, slot } = &place.root else {
            return Ok(None);
        };
        let mut value = self
            .environments
            .scope(*scope)
            .variables
            .get_slot(*slot, name)
            .ok_or_else(|| {
                Signal::Error(self.error(format!("undefined variable ${name}"), span))
            })?;
        for (depth, key) in place.path.iter().enumerate() {
            if depth == 0 {
                if let Value::Map(entries) = value {
                    if !map_index::eligible_size(entries.len()) {
                        value = self.operation(ops::index_ref(value, key), span)?;
                        continue;
                    }
                    if let Some(position) = self
                        .map_indexes
                        .borrow_mut()
                        .position(*scope, name, entries, key, true)
                    {
                        value = &entries[position].1;
                        continue;
                    }
                }
            }
            value = self.operation(ops::index_ref(value, key), span)?;
        }
        Ok(Some(value))
    }
    fn check_place(&self, place: &Place<'_>, span: Span) -> Result<()> {
        if self.variable_ref(place, span)?.is_none() {
            self.read_place(place, span)?;
        }
        Ok(())
    }
    fn read_place(&self, place: &Place<'_>, span: Span) -> Result<Value> {
        if let Some(value) = self.variable_ref(place, span)? {
            return Ok(value.clone());
        }
        let Root::Property { object, name } = &place.root else {
            unreachable!()
        };
        let owned = self.object_field(object, name, span)?;
        self.checked(owned.validate(), span)?;
        let mut value = &owned;
        for key in &place.path {
            value = self.operation(ops::index_ref(value, key), span)?;
        }
        Ok(value.clone())
    }
    #[inline]
    fn validate_value(&self, value: &Value, depth: usize, span: Span) -> Result<()> {
        if depth > MAX_VALUE_DEPTH {
            return self.fail("value nesting exceeds 128 levels", span);
        }
        if self.options.max_value_bytes.is_none() && depth == 0 {
            return self.checked(value.validate(), span);
        }
        self.checked(value.measure(depth, self.options.max_value_bytes), span)
            .map(|_| ())
    }
    fn enter_scope(&mut self, parent: usize, span: Span) -> Result<usize> {
        if self
            .options
            .max_scopes
            .is_some_and(|limit| self.environments.live >= limit)
        {
            self.collect_garbage();
        }
        if self
            .options
            .max_scopes
            .is_some_and(|limit| self.environments.live >= limit)
        {
            return Err(Signal::Interrupted(
                self.error("execution scope limit exceeded", span),
            ));
        }
        Ok(self.environments.enter(parent))
    }
    #[inline]
    fn enter_empty_scope(&mut self, parent: usize, span: Span) -> Result<usize> {
        if self
            .options
            .max_scopes
            .is_some_and(|limit| self.environments.live >= limit)
        {
            self.collect_garbage();
        }
        if self
            .options
            .max_scopes
            .is_some_and(|limit| self.environments.live >= limit)
        {
            return Err(Signal::Interrupted(
                self.error("execution scope limit exceeded", span),
            ));
        }
        Ok(self.environments.enter_empty(parent))
    }
    fn assignment(
        &mut self,
        target: &Expr,
        op: AssignmentOp,
        rhs: &Expr,
        retain: bool,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        if let ExprKind::List(items) = &target.kind {
            return self.destructure(items, op, rhs, retain, target.span, output);
        }
        let place = self.place(target, output)?;
        if place.append && op != AssignmentOp::Assign {
            return self.fail(
                "list append requires plain assignment with '='",
                target.span,
            );
        }
        // Only discarded assignments with a pure suffix may reuse a variable's
        // buffer. Effectful suffixes need the original left-value snapshot.
        if !retain && place.path.is_empty() && !place.append {
            if let Root::Variable { scope, name, slot } = &place.root {
                let append = match op {
                    AssignmentOp::Concat if pure(rhs) => Some((rhs, target.span)),
                    AssignmentOp::Assign => self_concat_suffix(rhs, name),
                    _ => None,
                };
                if let Some((suffix, span)) = append.filter(|_| {
                    matches!(
                        self.environments
                            .scope(*scope)
                            .variables
                            .get_slot(*slot, name),
                        Some(Value::String(_))
                    )
                }) {
                    // Own the suffix before mutation, including reads of the
                    // destination itself or of independent string copies.
                    let value = self.evaluate(suffix, output)?;
                    let Value::String(suffix) = value else {
                        return self.operation(
                            Err(ops::Error::Type(
                                "concatenation requires two strings; use str(value)",
                            )),
                            span,
                        );
                    };
                    let Value::String(text) = self
                        .environments
                        .scope(*scope)
                        .variables
                        .get_slot(*slot, name)
                        .unwrap()
                    else {
                        unreachable!()
                    };
                    let length = text
                        .len()
                        .checked_add(suffix.len())
                        .and_then(|n| n.checked_add(std::mem::size_of::<Value>()))
                        .ok_or_else(|| {
                            let message = if op == AssignmentOp::Concat {
                                "string size overflow"
                            } else if self.options.max_value_bytes.is_some() {
                                "execution value byte limit exceeded"
                            } else {
                                "string allocation failed"
                            };
                            Signal::Error(self.error(message, span))
                        })?;
                    if self
                        .options
                        .max_value_bytes
                        .is_some_and(|limit| length > limit)
                    {
                        return self.fail("execution value byte limit exceeded", span);
                    }
                    let Value::String(text) = self
                        .environments
                        .scope_mut(*scope)
                        .variables
                        .get_slot_mut(*slot, name)
                        .unwrap()
                    else {
                        unreachable!()
                    };
                    text.try_reserve(suffix.len())
                        .map_err(|_| Signal::Error(self.error("string allocation failed", span)))?;
                    let Value::String(text) = self
                        .environments
                        .scope_mut(*scope)
                        .variables
                        .get_slot_mut(*slot, name)
                        .unwrap()
                    else {
                        unreachable!()
                    };
                    text.push_str(&suffix);
                    return Ok(Value::Null);
                }
            }
        }
        let old = if op == AssignmentOp::Assign {
            None
        } else {
            Some(self.read_place(&place, target.span)?)
        };
        let mut value = self.evaluate(rhs, output)?;
        if let Some(old) = old {
            let binary = match op {
                AssignmentOp::Add => BinaryOp::Add,
                AssignmentOp::Subtract => BinaryOp::Subtract,
                AssignmentOp::Multiply => BinaryOp::Multiply,
                AssignmentOp::Divide => BinaryOp::Divide,
                AssignmentOp::Concat => BinaryOp::Concat,
                AssignmentOp::Assign => unreachable!(),
            };
            value = self.operation(ops::binary(binary, old, value), target.span)?;
        }
        if retain {
            self.write_place(&place, value.clone(), target.span)?;
            Ok(value)
        } else {
            self.write_place(&place, value, target.span)?;
            Ok(Value::Null)
        }
    }
    fn write_place(&mut self, place: &Place<'_>, replacement: Value, span: Span) -> Result<()> {
        self.validate_value(
            &replacement,
            place.path.len() + usize::from(place.append),
            span,
        )?;
        match &place.root {
            Root::Variable { scope, name, slot } => {
                if place.path.is_empty() && !place.append {
                    self.map_indexes.get_mut().invalidate(*scope, name);
                    let variables = &mut self.environments.scope_mut(*scope).variables;
                    if let Some(value) = variables.get_slot_mut(*slot, name) {
                        *value = replacement;
                    } else {
                        variables.insert((*name).into(), replacement);
                    }
                    return Ok(());
                }
                let hint = place.path.first().and_then(|key| {
                    let Value::Map(entries) = self
                        .environments
                        .scope(*scope)
                        .variables
                        .get_slot(*slot, name)?
                    else {
                        return None;
                    };
                    if !map_index::eligible_size(entries.len()) {
                        return None;
                    }
                    self.map_indexes
                        .borrow_mut()
                        .position(*scope, name, entries, key, false)
                });
                if self.options.max_value_bytes.is_some() {
                    // The explicitly configured budget uses a transactional candidate.
                    let mut value = self
                        .environments
                        .scope(*scope)
                        .variables
                        .get_slot(*slot, name)
                        .unwrap()
                        .clone();
                    self.operation(
                        ops::set_index_with_hint(
                            &mut value,
                            &place.path,
                            replacement,
                            place.append,
                            hint,
                        ),
                        span,
                    )?;
                    self.validate_value(&value, 0, span)?;
                    *self
                        .environments
                        .scope_mut(*scope)
                        .variables
                        .get_slot_mut(*slot, name)
                        .unwrap() = value;
                    return Ok(());
                }
                let result = ops::set_index_with_hint(
                    self.environments
                        .scope_mut(*scope)
                        .variables
                        .get_slot_mut(*slot, name)
                        .unwrap(),
                    &place.path,
                    replacement,
                    place.append,
                    hint,
                );
                self.operation(result, span)
            }
            Root::Property { object, name } => {
                if place.path.is_empty() && !place.append {
                    return self.set_object_field(object, name, replacement, span);
                }
                let mut value = self.object_field(object, name, span)?;
                self.operation(
                    ops::set_index(&mut value, &place.path, replacement, place.append),
                    span,
                )?;
                self.validate_value(&value, 0, span)?;
                self.set_object_field(object, name, value, span)
            }
        }
    }
}

#[cfg(test)]
mod tests;

fn pure(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Literal(_) | ExprKind::Variable(_) => true,
        ExprKind::Grouped(e) | ExprKind::Unary(_, e) => pure(e),
        ExprKind::Binary(a, _, b) => pure(a) && pure(b),
        ExprKind::IndexAccess(a, b) => borrowable(a) && pure(b),
        _ => false,
    }
}
fn borrowable(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Variable(_) => true,
        ExprKind::Grouped(e) => borrowable(e),
        ExprKind::IndexAccess(a, b) => borrowable(a) && pure(b),
        _ => false,
    }
}

// Parentheses do not alter evaluation. Restrict the prefix to exactly the same
// variable: flattening chains or inspecting properties could change snapshots,
// intermediate value budgets, error spans, or user-code evaluation order.
fn self_concat_suffix<'a>(rhs: &'a Expr, name: &str) -> Option<(&'a Expr, Span)> {
    fn ungroup(mut expr: &Expr) -> &Expr {
        while let ExprKind::Grouped(inner) = &expr.kind {
            expr = inner;
        }
        expr
    }
    let concat = ungroup(rhs);
    let ExprKind::Binary(left, BinaryOp::Concat, suffix) = &concat.kind else {
        return None;
    };
    if matches!(&ungroup(left).kind, ExprKind::Variable(prefix) if prefix == name) && pure(suffix) {
        Some((suffix, concat.span))
    } else {
        None
    }
}
