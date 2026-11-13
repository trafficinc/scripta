//! Safe, synchronous AST interpretation with lexical scopes and injectable output.
#![forbid(unsafe_code)]

mod environment;
mod error;
mod modules;
mod native;
mod ops;
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
    /// Logical bytes per value, including collection slots and string contents.
    /// None preserves unrestricted payload sizes; this is not a process heap cap.
    pub max_value_bytes: Option<usize>,
    /// Live lexical scopes, including retained closure environments and globals.
    pub max_scopes: Option<usize>,
    pub max_source_bytes: usize,
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

#[derive(Default)]
pub struct Runtime {
    environments: Environments,
    stack: Vec<StackFrame>,
    identity: Rc<()>,
    options: RuntimeOptions,
    steps: u64,
    error_stream: Option<Box<dyn Write>>,
    input_stream: Option<Box<dyn std::io::BufRead>>,
    source: Option<Arc<SourceFile>>,
    active_modules: Vec<std::path::PathBuf>,
    module_policy: Option<Rc<modules::ModulePolicy>>,
    module_resolver: Option<Rc<modules::ModuleResolver>>,
}

/// A resolved assignment target. Index expressions and object receivers are
/// evaluated once, before the RHS, even for compound assignment.
enum Root {
    Variable {
        scope: usize,
        name: String,
    },
    Property {
        object: Rc<dyn NativeObject>,
        name: String,
    },
}
struct Place {
    root: Root,
    path: Vec<Value>,
    append: bool,
}

impl Runtime {
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
        functions.insert(
            name.clone(),
            Value::Callable(Callable(Rc::new(Function::Native { name, function }))),
        );
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
        self.environments
            .scope_mut(0)
            .variables
            .insert(name.into(), value);
        Ok(())
    }

    /// Execute in this runtime's global environment. Exit is returned to the host;
    /// neither script exit nor script errors terminate the embedding process.
    pub fn execute(
        &mut self,
        program: &Program,
        output: &mut impl Write,
    ) -> std::result::Result<u8, RuntimeError> {
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
        self.stack.push(StackFrame {
            function: "<main>".into(),
            source: self.source.clone(),
            span: program
                .statements
                .first()
                .map_or(Span::default(), |statement| statement.span),
        });
        let result = (|| {
            // Only direct top-level declarations are hoisted. Conditional and
            // nested declarations retain their execution-dependent behavior.
            for statement in &program.statements {
                if matches!(statement.kind, StatementKind::Function(_)) {
                    self.statement(statement, output)?;
                }
            }
            for statement in &program.statements {
                if !matches!(statement.kind, StatementKind::Function(_)) {
                    self.statement(statement, output)?;
                }
            }
            Ok(())
        })();
        let result = match result {
            Ok(()) => Ok(0),
            Err(Signal::Exit(code)) => Ok(code),
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
            trace: self.stack.clone(),
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
    fn condition(&mut self, expr: &Expr, output: &mut impl Write) -> Result<bool> {
        let value = self.evaluate(expr, output)?;
        self.operation(ops::boolean(value), expr.span)
    }
    fn statements(&mut self, statements: &[Statement], output: &mut impl Write) -> Result<()> {
        for statement in statements {
            self.statement(statement, output)?;
        }
        Ok(())
    }
    fn block(&mut self, block: &Block, output: &mut impl Write) -> Result<()> {
        let previous = self.enter_scope(self.environments.current, block.span)?;
        let result = self.statements(&block.statements, output);
        self.environments.leave(previous);
        result
    }
    fn expressions(
        &mut self,
        expressions: &[Expr],
        output: &mut impl Write,
    ) -> Result<Option<Value>> {
        let mut value = None;
        for expression in expressions {
            value = Some(self.evaluate(expression, output)?);
        }
        Ok(value)
    }

    fn statement(&mut self, statement: &Statement, output: &mut impl Write) -> Result<()> {
        self.checkpoint(statement.span)?;
        match &statement.kind {
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
                    self.assignment(target, *op, rhs, false, output)?;
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
                            let (value, exception_type) = match &signal {
                                Signal::Throw(value, _) => {
                                    (value.clone(), value.type_name().to_string())
                                }
                                Signal::TypeError(error) => {
                                    (Value::String(error.message.clone()), "TypeError".into())
                                }
                                Signal::NativeException(error, _) => {
                                    (Value::String(error.message.clone()), error.kind.clone())
                                }
                                Signal::OperationException(kind, error) => {
                                    (Value::String(error.message.clone()), (*kind).into())
                                }
                                _ => unreachable!(),
                            };
                            for catch in catches {
                                if catch.exception_type.as_ref().is_none_or(|annotation| {
                                    annotation.name == "Error" || annotation.name == exception_type
                                }) {
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
                let message = match &value {
                    Value::String(message) => format!("uncaught thrown value: {message}"),
                    _ => format!("uncaught thrown {} value", value.type_name()),
                };
                return Err(Signal::Throw(value, self.error(message, expr.span)));
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

    fn evaluate(&mut self, expr: &Expr, output: &mut impl Write) -> Result<Value> {
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
            ExprKind::Variable(name) => {
                let Some(scope) = self.environments.find(name, false) else {
                    return self.fail(format!("undefined variable ${name}"), expr.span);
                };
                Ok(self.environments.scope(scope).variables[name].clone())
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
            ExprKind::Assignment(target, op, rhs) => {
                self.assignment(target, *op, rhs, true, output)
            }
            ExprKind::Call(callee, args) => {
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
                let value = self.property_result(object.get_property(name), expr.span)?;
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
            ExprKind::List(items) => {
                let value = Value::List(
                    items
                        .iter()
                        .map(|item| self.evaluate(item, output))
                        .collect::<Result<Vec<_>>>()?,
                );
                self.validate_value(&value, 0, expr.span)?;
                Ok(value)
            }
            ExprKind::Map(entries) => {
                let mut map = Vec::new();
                for (key, value) in entries {
                    let key_value = self.evaluate(key, output)?;
                    let key = self.operation(
                        MapKey::from_value(key_value)
                            .map_err(|_| ops::Error::Type("map keys must be int or string")),
                        key.span,
                    )?;
                    let value = self.evaluate(value, output)?;
                    if let Some((_, old)) = map.iter_mut().find(|(candidate, _)| candidate == &key)
                    {
                        *old = value;
                    } else {
                        map.push((key, value));
                    }
                }
                let value = Value::Map(map);
                self.validate_value(&value, 0, expr.span)?;
                Ok(value)
            }
        }
    }

    fn function(&self, name: &str, span: Span) -> Result<Value> {
        let Some(scope) = self.environments.find(name, true) else {
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
        Ok(self.environments.scope(scope).functions[name].clone())
    }

    fn user_function(&mut self, declaration: &scr_ast::FunctionDeclaration) -> Result<Value> {
        let mut names = std::collections::HashSet::new();
        let mut optional = false;
        for (index, parameter) in declaration.parameters.iter().enumerate() {
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
        self.environments.capture();
        let function = Rc::new(Function::User {
            declaration: declaration.clone(),
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
        output: &mut impl Write,
    ) -> Result<Value> {
        let Value::Callable(callable) = callee else {
            return self.fail("call requires a callable value", span);
        };
        let values = args
            .iter()
            .map(|arg| self.evaluate(arg, output))
            .collect::<Result<Vec<_>>>()?;
        if self.stack.len().saturating_sub(1)
            >= self.options.recursion_limit.min(MAX_RECURSION_LIMIT)
        {
            return self.fail("maximum call depth exceeded", span);
        }
        self.stack.push(StackFrame {
            function: callable.name().into(),
            source: self.source.clone(),
            span,
        });
        let result = match callable.0.as_ref() {
            Function::Native { name, function } => {
                let settings = &mut self.options.context;
                let mut context = RuntimeContext {
                    working_directory: &mut settings.working_directory,
                    script_path: settings.script_path.as_deref(),
                    cli_arguments: &settings.cli_arguments,
                    environment: &mut settings.environment,
                    permissions: &settings.permissions,
                    cancellation: self.options.interrupt_flag.clone(),
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
                    .invoke(name, &mut context, &values)
                    .map_err(|error| self.native_error(error, span))
            }
            Function::User {
                declaration,
                environment,
                owner,
                source,
            } => {
                if !Rc::ptr_eq(owner, &self.identity) {
                    self.stack.pop();
                    return self.fail("user callable belongs to a different runtime", span);
                }
                let required = declaration
                    .parameters
                    .iter()
                    .filter(|p| p.default.is_none() && !p.variadic)
                    .count();
                let variadic = declaration.parameters.last().is_some_and(|p| p.variadic);
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
                let previous = match self.enter_scope(*environment, span) {
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
                    let mut values = values.into_iter();
                    for parameter in &declaration.parameters {
                        let value = if parameter.variadic {
                            let items: Vec<_> = values.by_ref().collect();
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
                            let value = if let Some(value) = values.next() {
                                value
                            } else {
                                self.evaluate(
                                    parameter.default.as_ref().expect("validated arity"),
                                    output,
                                )?
                            };
                            self.annotation(&parameter.annotation, &value, parameter.span, false)?;
                            value
                        };
                        self.environments
                            .scope_mut(self.environments.current)
                            .variables
                            .insert(parameter.binding.name.clone(), value);
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
                self.environments.leave(previous);
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
            if !(annotation.name == value.type_name()
                || (returning && annotation.name == "void" && *value == Value::Null))
            {
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

    fn place(&mut self, target: &Expr, output: &mut impl Write) -> Result<Place> {
        match &target.kind {
            ExprKind::Grouped(inner) => self.place(inner, output),
            ExprKind::Variable(name) => Ok(Place {
                root: Root::Variable {
                    scope: self
                        .environments
                        .find(name, false)
                        .unwrap_or(self.environments.current),
                    name: name.clone(),
                },
                path: Vec::new(),
                append: false,
            }),
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
                    root: Root::Property {
                        object,
                        name: name.clone(),
                    },
                    path: Vec::new(),
                    append: false,
                })
            }
            _ => self.fail("invalid assignment target", target.span),
        }
    }

    fn variable_ref<'a>(&'a self, place: &Place, span: Span) -> Result<Option<&'a Value>> {
        let Root::Variable { scope, name } = &place.root else {
            return Ok(None);
        };
        let mut value = self
            .environments
            .scope(*scope)
            .variables
            .get(name)
            .ok_or_else(|| {
                Signal::Error(self.error(format!("undefined variable ${name}"), span))
            })?;
        for key in &place.path {
            value = self.operation(ops::index_ref(value, key), span)?;
        }
        Ok(Some(value))
    }
    fn check_place(&self, place: &Place, span: Span) -> Result<()> {
        if self.variable_ref(place, span)?.is_none() {
            self.read_place(place, span)?;
        }
        Ok(())
    }
    fn read_place(&self, place: &Place, span: Span) -> Result<Value> {
        if let Some(value) = self.variable_ref(place, span)? {
            return Ok(value.clone());
        }
        let Root::Property { object, name } = &place.root else {
            unreachable!()
        };
        let owned = self.property_result(object.get_property(name), span)?;
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
    fn assignment(
        &mut self,
        target: &Expr,
        op: AssignmentOp,
        rhs: &Expr,
        retain: bool,
        output: &mut impl Write,
    ) -> Result<Value> {
        let place = self.place(target, output)?;
        if place.append && op != AssignmentOp::Assign {
            return self.fail(
                "list append requires plain assignment with '='",
                target.span,
            );
        }
        // Only pure RHS expressions can use the existing string buffer: a call
        // or assignment could mutate the LHS after its original value was read.
        if !retain && op == AssignmentOp::Concat && pure(rhs) && place.path.is_empty() {
            if let Root::Variable { scope, name } = &place.root {
                if matches!(
                    self.environments.scope(*scope).variables.get(name),
                    Some(Value::String(_))
                ) {
                    let value = self.evaluate(rhs, output)?;
                    let Value::String(suffix) = value else {
                        return self.operation(
                            Err(ops::Error::Type(
                                "concatenation requires two strings; use str(value)",
                            )),
                            target.span,
                        );
                    };
                    let Value::String(text) = &self.environments.scope(*scope).variables[name]
                    else {
                        unreachable!()
                    };
                    let length = text
                        .len()
                        .checked_add(suffix.len())
                        .and_then(|n| n.checked_add(std::mem::size_of::<Value>()))
                        .ok_or_else(|| {
                            Signal::Error(self.error("string size overflow", target.span))
                        })?;
                    if self
                        .options
                        .max_value_bytes
                        .is_some_and(|limit| length > limit)
                    {
                        return self.fail("execution value byte limit exceeded", target.span);
                    }
                    let Value::String(text) = self
                        .environments
                        .scope_mut(*scope)
                        .variables
                        .get_mut(name)
                        .unwrap()
                    else {
                        unreachable!()
                    };
                    text.try_reserve(suffix.len()).map_err(|_| {
                        Signal::Error(self.error("string allocation failed", target.span))
                    })?;
                    let Value::String(text) = self
                        .environments
                        .scope_mut(*scope)
                        .variables
                        .get_mut(name)
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
    fn write_place(&mut self, place: &Place, replacement: Value, span: Span) -> Result<()> {
        self.validate_value(
            &replacement,
            place.path.len() + usize::from(place.append),
            span,
        )?;
        match &place.root {
            Root::Variable { scope, name } => {
                if place.path.is_empty() && !place.append {
                    self.environments
                        .scope_mut(*scope)
                        .variables
                        .insert(name.clone(), replacement);
                    return Ok(());
                }
                if self.options.max_value_bytes.is_some() {
                    // The explicitly configured budget uses a transactional candidate.
                    let mut value = self
                        .environments
                        .scope(*scope)
                        .variables
                        .get(name)
                        .unwrap()
                        .clone();
                    self.operation(
                        ops::set_index(&mut value, &place.path, replacement, place.append),
                        span,
                    )?;
                    self.validate_value(&value, 0, span)?;
                    self.environments
                        .scope_mut(*scope)
                        .variables
                        .insert(name.clone(), value);
                    return Ok(());
                }
                let result = ops::set_index(
                    self.environments
                        .scope_mut(*scope)
                        .variables
                        .get_mut(name)
                        .unwrap(),
                    &place.path,
                    replacement,
                    place.append,
                );
                self.operation(result, span)
            }
            Root::Property { object, name } => {
                if place.path.is_empty() && !place.append {
                    return self.property_result(object.set_property(name, replacement), span);
                }
                let mut value = self.property_result(object.get_property(name), span)?;
                self.operation(
                    ops::set_index(&mut value, &place.path, replacement, place.append),
                    span,
                )?;
                self.validate_value(&value, 0, span)?;
                self.property_result(object.set_property(name, value), span)
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
