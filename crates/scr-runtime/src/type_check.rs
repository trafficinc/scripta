use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use scr_ast::{
    AssignmentOp, BinaryOp, Block, ClassDeclaration, Expr, ExprKind, FunctionDeclaration, Literal,
    Program, Statement, StatementKind, TypeAnnotation, UnaryOp,
};
use scr_diagnostics::{Diagnostic, Span};

use crate::{NativeSignature, NativeType, Runtime, Value, value::Function};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Null,
    Bool,
    Int,
    Float,
    String,
    Bytes,
    List,
    Map,
    Callable,
    Object(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Type {
    // None is incomplete information, distinct from a known set of alternatives.
    kinds: Option<BTreeSet<Kind>>,
    signature: Option<Rc<Signature>>,
}

impl Type {
    fn unknown() -> Self {
        Self {
            kinds: None,
            signature: None,
        }
    }

    fn of(kind: Kind) -> Self {
        Self {
            kinds: Some(BTreeSet::from([kind])),
            signature: None,
        }
    }

    fn callable(mut signature: Signature) -> Self {
        // Function-reference chains need no nested AST. Bound structural
        // inference so comparisons and drops cannot exhaust the stack.
        signature.returns = signature.returns.bounded(8);
        Self {
            kinds: Some(BTreeSet::from([Kind::Callable])),
            signature: Some(Rc::new(signature)),
        }
    }

    fn bounded(mut self, depth: usize) -> Self {
        if depth == 0 {
            self.signature = None;
        } else if let Some(signature) = &self.signature {
            let mut signature = (**signature).clone();
            signature.returns = signature.returns.bounded(depth - 1);
            self.signature = Some(Rc::new(signature));
        }
        self
    }

    fn union(&self, other: &Self) -> Self {
        let (Some(a), Some(b)) = (&self.kinds, &other.kinds) else {
            return Self::unknown();
        };
        Self {
            kinds: Some(a.union(b).cloned().collect()),
            signature: if self.signature == other.signature {
                self.signature.clone()
            } else {
                None
            },
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        match (&self.kinds, &other.kinds) {
            (Some(a), Some(b)) => a.iter().any(|a| {
                b.iter().any(|b| {
                    // Class ancestry, interfaces and host identities remain runtime checks.
                    a == b || matches!((a, b), (Kind::Object(_), Kind::Object(_)))
                })
            }),
            _ => true,
        }
    }

    fn display(&self) -> String {
        let Some(kinds) = &self.kinds else {
            return "unknown".into();
        };
        kinds
            .iter()
            .map(|kind| match kind {
                Kind::Null => "null",
                Kind::Bool => "bool",
                Kind::Int => "int",
                Kind::Float => "float",
                Kind::String => "string",
                Kind::Bytes => "bytes",
                Kind::List => "list",
                Kind::Map => "map",
                Kind::Callable => "callable",
                Kind::Object(name) => name,
            })
            .collect::<Vec<_>>()
            .join(" or ")
    }

    fn annotation(annotation: &TypeAnnotation, returning: bool) -> Self {
        Self::of(match annotation.name.as_str() {
            "null" => Kind::Null,
            "void" if returning => Kind::Null,
            "bool" => Kind::Bool,
            "int" => Kind::Int,
            "float" => Kind::Float,
            "string" => Kind::String,
            "bytes" => Kind::Bytes,
            "list" => Kind::List,
            "map" => Kind::Map,
            "callable" => Kind::Callable,
            name => Kind::Object(name.into()),
        })
    }

    fn native(native: &NativeType) -> Self {
        Self::of(match native {
            NativeType::Any => return Self::unknown(),
            NativeType::Null => Kind::Null,
            NativeType::Bool => Kind::Bool,
            NativeType::Int => Kind::Int,
            NativeType::Float => Kind::Float,
            NativeType::String => Kind::String,
            NativeType::Bytes => Kind::Bytes,
            NativeType::List => Kind::List,
            NativeType::Map => Kind::Map,
            NativeType::Callable => Kind::Callable,
            NativeType::Object(name) => Kind::Object((*name).into()),
            NativeType::OneOf(types) => {
                return types
                    .iter()
                    .map(Self::native)
                    .reduce(|a, b| a.union(&b))
                    .unwrap_or_else(Self::unknown);
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    parameters: Vec<Type>,
    required: usize,
    variadic: Option<Type>,
    returns: Type,
}

impl Signature {
    fn script(function: &FunctionDeclaration) -> Self {
        Self {
            parameters: function
                .parameters
                .iter()
                .filter(|p| !p.variadic)
                .map(|p| {
                    p.annotation
                        .as_ref()
                        .map_or_else(Type::unknown, |a| Type::annotation(a, false))
                })
                .collect(),
            required: function
                .parameters
                .iter()
                .filter(|p| !p.variadic && p.default.is_none())
                .count(),
            variadic: function.parameters.iter().find(|p| p.variadic).map(|p| {
                p.annotation
                    .as_ref()
                    .map_or_else(Type::unknown, |a| Type::annotation(a, false))
            }),
            returns: function
                .return_type
                .as_ref()
                .map_or_else(Type::unknown, |a| Type::annotation(a, true)),
        }
    }

    fn native(signature: &NativeSignature) -> Self {
        Self {
            parameters: signature
                .parameters
                .iter()
                .chain(&signature.optional)
                .map(Type::native)
                .collect(),
            required: signature.parameters.len(),
            variadic: signature.variadic.as_ref().map(Type::native),
            returns: Type::native(&signature.returns),
        }
    }
}

#[derive(Clone, Default)]
struct Scope {
    variables: HashMap<String, Type>,
    functions: HashMap<String, Type>,
    classes: HashMap<String, ClassDeclaration>,
    aliases: HashSet<String>,
}

struct Checker {
    scopes: Vec<Scope>,
    errors: Vec<Diagnostic>,
    returns: Vec<(Type, Span)>,
    // A call can invoke any escaped closure. Discard facts about bindings that
    // those closures may change, including through native callbacks.
    captured_writes: HashSet<String>,
    function_depth: usize,
}

impl Runtime {
    /// Check an AST without executing it or loading modules, using this runtime's native signatures.
    pub fn check_program(&self, program: &Program) -> Vec<Diagnostic> {
        let mut root = Scope::default();
        for (name, value) in &self.environments.scope(0).functions {
            if let Value::Callable(callable) = value {
                if let Function::Native { function, .. } = callable.0.as_ref() {
                    root.functions.insert(
                        name.clone(),
                        Type::callable(Signature::native(function.signature())),
                    );
                }
            }
        }
        let mut checker = Checker {
            scopes: vec![root],
            errors: Vec::new(),
            returns: Vec::new(),
            captured_writes: HashSet::new(),
            function_depth: 0,
        };
        for statement in &program.statements {
            checker.collect_captures(statement);
            if let StatementKind::Function(function) = &statement.kind {
                checker.scopes[0].functions.insert(
                    function.name.clone(),
                    Type::callable(Signature::script(function)),
                );
            }
            if let StatementKind::Class(class) = &statement.kind {
                checker.scopes[0]
                    .classes
                    .insert(class.name.name.clone(), class.clone());
            }
        }
        // Two bounded sweeps infer straightforward forward-call chains. Cycles
        // without annotations retain unknown results; no body is ever executed.
        for _ in 0..2 {
            for statement in program.statements.iter().rev() {
                if let StatementKind::Function(function) = &statement.kind {
                    let ty = checker.function(function, None);
                    checker.scopes[0]
                        .functions
                        .insert(function.name.clone(), ty);
                }
            }
        }
        checker.errors.clear();
        // Hoisted declarations remain reachable even after a top-level exit.
        for statement in &program.statements {
            if matches!(
                statement.kind,
                StatementKind::Function(_) | StatementKind::Class(_) | StatementKind::Interface(_)
            ) {
                checker.statement(statement);
            }
        }
        for statement in &program.statements {
            if !matches!(
                statement.kind,
                StatementKind::Function(_) | StatementKind::Class(_) | StatementKind::Interface(_)
            ) && !checker.statement(statement)
            {
                break;
            }
        }
        checker.errors
    }
}

impl Checker {
    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.errors
            .push(Diagnostic::new(message, span).with_code("E0310").with_help(
            "use matching types or an explicit conversion; dynamic values are checked at runtime",
        ));
    }

    fn expect(&mut self, actual: &Type, expected: &Type, span: Span, context: &str) {
        if !actual.overlaps(expected) {
            self.error(
                format!(
                    "{context} requires {}, got {}",
                    expected.display(),
                    actual.display()
                ),
                span,
            );
        }
    }

    fn lookup(&self, name: &str, function: bool) -> Type {
        self.scopes
            .iter()
            .rev()
            .enumerate()
            .find_map(|(index, scope)| {
                if !function && self.function_depth > 0 && index + 1 == self.scopes.len() {
                    return None;
                }
                if function {
                    scope.functions.get(name)
                } else {
                    scope.variables.get(name)
                }
                .cloned()
            })
            .unwrap_or_else(Type::unknown)
    }

    fn assign(&mut self, name: &str, ty: Type) {
        let floor = usize::from(self.function_depth > 0);
        for scope in self.scopes[floor..].iter_mut().rev() {
            if let Some(value) = scope.variables.get_mut(name) {
                *value = ty;
                return;
            }
        }
        self.scopes
            .last_mut()
            .unwrap()
            .variables
            .insert(name.into(), ty);
    }

    fn invalidate_calls(&mut self) {
        for scope in &mut self.scopes {
            for (name, ty) in &mut scope.variables {
                if self.captured_writes.contains(name) {
                    *ty = Type::unknown();
                }
            }
        }
    }

    fn statements(&mut self, statements: &[Statement]) -> bool {
        for statement in statements {
            if !self.statement(statement) {
                return false;
            }
        }
        true
    }

    fn block(&mut self, block: &Block) -> bool {
        self.scopes.push(Scope::default());
        let falls = self.statements(&block.statements);
        self.scopes.pop();
        falls
    }

    fn condition(&mut self, expression: &Expr) {
        let ty = self.expression(expression);
        self.expect(&ty, &Type::of(Kind::Bool), expression.span, "condition");
    }

    fn merge(&mut self, paths: &[Vec<Scope>], baseline: &[Scope]) {
        self.scopes = baseline.to_vec();
        if paths.is_empty() {
            return;
        }
        for (i, scope) in self.scopes.iter_mut().enumerate() {
            let names: HashSet<_> = paths
                .iter()
                .flat_map(|p| p[i].variables.keys().cloned())
                .collect();
            for name in names {
                let ty = paths
                    .iter()
                    .map(|p| {
                        p[i].variables
                            .get(&name)
                            .cloned()
                            .unwrap_or_else(Type::unknown)
                    })
                    .reduce(|a, b| a.union(&b))
                    .unwrap();
                scope.variables.insert(name, ty);
            }
            // Conditional declarations are callable only when every continuing path agrees.
            scope
                .functions
                .retain(|name, ty| paths.iter().all(|p| p[i].functions.get(name) == Some(ty)));
            scope
                .aliases
                .extend(paths.iter().flat_map(|p| p[i].aliases.iter().cloned()));
        }
    }

    fn statement(&mut self, statement: &Statement) -> bool {
        match &statement.kind {
            StatementKind::Echo(expr) => {
                let ty = self.expression(expr);
                self.expect(&ty, &Type::of(Kind::String), expr.span, "echo");
            }
            StatementKind::Expression(expr) => {
                self.expression(expr);
            }
            StatementKind::Block(block) => return self.block(block),
            StatementKind::Function(function) => {
                // Only direct program declarations are hoisted by the runtime.
                if self.scopes.len() != 1 {
                    self.scopes.last_mut().unwrap().functions.insert(
                        function.name.clone(),
                        Type::callable(Signature::script(function)),
                    );
                }
                let ty = self.function(function, None);
                self.scopes
                    .last_mut()
                    .unwrap()
                    .functions
                    .insert(function.name.clone(), ty);
            }
            StatementKind::Class(class) => {
                self.scopes
                    .last_mut()
                    .unwrap()
                    .classes
                    .insert(class.name.name.clone(), class.clone());
                for field in &class.fields {
                    if let Some(expr) = &field.default {
                        let saved = self.scopes.clone();
                        for scope in &mut self.scopes {
                            for ty in scope.variables.values_mut() {
                                *ty = Type::unknown();
                            }
                        }
                        let ty = self.expression(expr);
                        if let Some(annotation) = &field.annotation {
                            self.expect(
                                &ty,
                                &Type::annotation(annotation, false),
                                expr.span,
                                "field default",
                            );
                        }
                        self.scopes = saved;
                    }
                }
                for method in &class.methods {
                    self.function(method, Some(&class.name.name));
                }
            }
            StatementKind::Interface(_) => {}
            StatementKind::If {
                branches,
                else_branch,
            } => {
                let mut paths = Vec::new();
                let mut exhausted = false;
                for (condition, body) in branches {
                    self.condition(condition);
                    if literal_bool(condition) == Some(false) {
                        continue;
                    }
                    let remaining = self.scopes.clone();
                    if self.statement(body) {
                        paths.push(self.scopes.clone());
                    }
                    self.scopes = remaining;
                    if literal_bool(condition) == Some(true) {
                        exhausted = true;
                        break;
                    }
                }
                let baseline = self.scopes.clone();
                if exhausted {
                    // All remaining paths entered the last true branch.
                } else if let Some(body) = else_branch {
                    if self.statement(body) {
                        paths.push(self.scopes.clone());
                    }
                } else {
                    paths.push(baseline.clone());
                }
                self.merge(&paths, &baseline);
                return !paths.is_empty();
            }
            StatementKind::While { condition, body } => {
                if literal_bool(condition) == Some(false) {
                    self.condition(condition);
                    return true;
                }
                self.forget_writes(statement);
                self.condition(condition);
                self.statement(body);
                self.forget_writes(statement);
                if literal_bool(condition) == Some(true) && !has_break(body) {
                    return false;
                }
            }
            StatementKind::For {
                initializer,
                condition,
                increment,
                body,
            } => {
                self.scopes.push(Scope::default());
                for expr in initializer {
                    self.expression(expr);
                }
                if condition.last().and_then(literal_bool) == Some(false) {
                    for expr in condition {
                        self.expression(expr);
                    }
                    self.scopes.pop();
                    return true;
                }
                self.forget_writes(statement);
                for (i, expr) in condition.iter().enumerate() {
                    if i + 1 == condition.len() {
                        self.condition(expr);
                    } else {
                        self.expression(expr);
                    }
                }
                self.statement(body);
                for expr in increment {
                    self.expression(expr);
                }
                self.forget_writes(statement);
                self.scopes.pop();
                if (condition.is_empty() || condition.last().and_then(literal_bool) == Some(true))
                    && !has_break(body)
                {
                    return false;
                }
            }
            StatementKind::Foreach {
                iterable,
                key,
                value,
                body,
            } => {
                let ty = self.expression(iterable);
                self.expect(
                    &ty,
                    &Type::of(Kind::List).union(&Type::of(Kind::Map)),
                    iterable.span,
                    "foreach",
                );
                self.forget_writes(statement);
                self.scopes.push(Scope::default());
                if let Some(key) = key {
                    self.scopes.last_mut().unwrap().variables.insert(
                        key.name.clone(),
                        Type::of(Kind::Int).union(&Type::of(Kind::String)),
                    );
                }
                self.scopes
                    .last_mut()
                    .unwrap()
                    .variables
                    .insert(value.name.clone(), Type::unknown());
                self.statement(body);
                self.scopes.pop();
                self.forget_writes(statement);
            }
            StatementKind::Return(expr) => {
                let ty = expr
                    .as_ref()
                    .map_or_else(|| Type::of(Kind::Null), |e| self.expression(e));
                self.returns
                    .push((ty, expr.as_ref().map_or(statement.span, |e| e.span)));
                return false;
            }
            StatementKind::Throw(expr) => {
                self.expression(expr);
                return false;
            }
            StatementKind::Exit(expr) => {
                if let Some(expr) = expr {
                    let ty = self.expression(expr);
                    self.expect(&ty, &Type::of(Kind::Int), expr.span, "exit");
                }
                return false;
            }
            StatementKind::Break | StatementKind::Continue => return false,
            StatementKind::Try {
                body,
                catches,
                finally,
            } => {
                let baseline = self.scopes.clone();
                let return_start = self.returns.len();
                // A throw may occur after any completed write. Widen writes before
                // catch/finally analysis rather than pretending try is atomic.
                self.forget_writes(statement);
                self.invalidate_calls();
                let exceptional = self.scopes.clone();
                self.scopes = baseline.clone();
                let mut paths = Vec::new();
                if self.block(body) {
                    paths.push(self.scopes.clone());
                }
                for catch in catches {
                    self.scopes = exceptional.clone();
                    self.scopes.push(Scope::default());
                    let ty = catch
                        .exception_type
                        .as_ref()
                        .map_or_else(Type::unknown, |a| Type::annotation(a, false));
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .variables
                        .insert(catch.binding.name.clone(), ty);
                    let falls = self.block(&catch.body);
                    self.scopes.pop();
                    if falls {
                        paths.push(self.scopes.clone());
                    }
                }
                self.merge(&paths, &baseline);
                if let Some(finally) = finally {
                    // Finally also runs on return/throw/break/continue paths.
                    self.forget_writes(statement);
                    let pending = self.returns.split_off(return_start);
                    if !self.block(finally) {
                        return false;
                    }
                    self.returns.extend(pending);
                }
                return !paths.is_empty();
            }
            StatementKind::Require { path, alias, .. } => {
                let ty = self.expression(path);
                self.expect(&ty, &Type::of(Kind::String), path.span, "require");
                if let Some(alias) = alias {
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .aliases
                        .insert(alias.name.clone());
                } else {
                    for scope in &mut self.scopes {
                        for ty in scope.variables.values_mut() {
                            *ty = Type::unknown();
                        }
                    }
                }
            }
        }
        true
    }

    fn function(&mut self, function: &FunctionDeclaration, receiver: Option<&str>) -> Type {
        let saved = self.scopes.clone();
        let saved_returns = std::mem::take(&mut self.returns);
        self.function_depth += 1;
        // Captures are live references, not snapshots at declaration time.
        for scope in &mut self.scopes {
            for ty in scope.variables.values_mut() {
                *ty = Type::unknown();
            }
        }
        self.scopes.push(Scope::default());
        if let Some(name) = receiver {
            self.scopes
                .last_mut()
                .unwrap()
                .variables
                .insert("this".into(), Type::of(Kind::Object(name.into())));
        }
        for parameter in &function.parameters {
            let expected = parameter
                .annotation
                .as_ref()
                .map_or_else(Type::unknown, |a| Type::annotation(a, false));
            if let Some(default) = &parameter.default {
                // Defaults may be bypassed by a supplied argument, including their writes.
                let before = self.scopes.clone();
                let actual = self.expression(default);
                self.expect(&actual, &expected, default.span, "parameter default");
                let after = self.scopes.clone();
                self.merge(&[before.clone(), after], &before);
            }
            self.scopes.last_mut().unwrap().variables.insert(
                parameter.binding.name.clone(),
                if parameter.variadic {
                    Type::of(Kind::List)
                } else {
                    expected
                },
            );
        }
        let falls = self.block(&function.body);
        if falls {
            self.returns
                .push((Type::of(Kind::Null), function.body.span));
        }
        let mut signature = Signature::script(function);
        let constructor = receiver.is_some() && function.name == "__construct";
        if function.return_type.is_some() || constructor {
            for (ty, span) in self.returns.clone() {
                if constructor {
                    self.expect(&ty, &Type::of(Kind::Null), span, "constructor return");
                }
                if function.return_type.is_some() {
                    self.expect(&ty, &signature.returns, span, "return");
                }
            }
        } else {
            signature.returns = self
                .returns
                .iter()
                .map(|(ty, _)| ty.clone())
                .reduce(|a, b| a.union(&b))
                .unwrap_or_else(Type::unknown);
        }
        self.function_depth -= 1;
        self.scopes = saved;
        self.returns = saved_returns;
        Type::callable(signature)
    }

    fn expression(&mut self, expr: &Expr) -> Type {
        match &expr.kind {
            ExprKind::Literal(literal) => Type::of(match literal {
                Literal::Null => Kind::Null,
                Literal::Bool(_) => Kind::Bool,
                Literal::Int(_) => Kind::Int,
                Literal::Float(_) => Kind::Float,
                Literal::String(_) => Kind::String,
            }),
            ExprKind::Variable(name) => self.lookup(name, false),
            ExprKind::Identifier(name) => self.lookup(name, true),
            ExprKind::Grouped(inner) => self.expression(inner),
            ExprKind::Function(function) => self.function(function, None),
            ExprKind::Unary(op, inner) => {
                let ty = self.expression(inner);
                let expected = if *op == UnaryOp::Not {
                    Type::of(Kind::Bool)
                } else {
                    Type::of(Kind::Int).union(&Type::of(Kind::Float))
                };
                self.expect(&ty, &expected, expr.span, "unary operator");
                if *op == UnaryOp::Not {
                    Type::of(Kind::Bool)
                } else {
                    ty
                }
            }
            ExprKind::Binary(left, op, right) => {
                let a = self.expression(left);
                let before = self.scopes.clone();
                let short = matches!(op, BinaryOp::And | BinaryOp::Or);
                // A literal short-circuit makes the right side unreachable.
                if short
                    && matches!(
                        (literal_bool(left), op),
                        (Some(false), BinaryOp::And) | (Some(true), BinaryOp::Or)
                    )
                {
                    return Type::of(Kind::Bool);
                }
                let b = self.expression(right);
                if short {
                    let after = self.scopes.clone();
                    self.merge(&[before.clone(), after], &before);
                }
                self.binary(*op, &a, &b, expr.span)
            }
            ExprKind::Assignment(target, op, value) => {
                let target = ungroup(target);
                let old = if let ExprKind::Variable(name) = &target.kind {
                    self.lookup(name, false)
                } else {
                    self.expression(target)
                };
                let field_contract = if let ExprKind::PropertyAccess(base, name) = &target.kind {
                    self.field_type(&self.receiver_type(base), name)
                } else {
                    None
                };
                let value_ty = self.expression(value);
                let result = match op {
                    AssignmentOp::Assign => value_ty,
                    _ => self.binary(
                        match op {
                            AssignmentOp::Add => BinaryOp::Add,
                            AssignmentOp::Subtract => BinaryOp::Subtract,
                            AssignmentOp::Multiply => BinaryOp::Multiply,
                            AssignmentOp::Divide => BinaryOp::Divide,
                            AssignmentOp::Concat => BinaryOp::Concat,
                            AssignmentOp::Assign => unreachable!(),
                        },
                        &old,
                        &value_ty,
                        expr.span,
                    ),
                };
                if let ExprKind::Variable(name) = &target.kind {
                    self.assign(name, result.clone());
                }
                if let Some(expected) = field_contract {
                    self.expect(&result, &expected, expr.span, "field assignment");
                }
                result
            }
            ExprKind::Call(callee, arguments) => {
                let callable = self.expression(callee);
                self.call(callable, arguments, expr.span)
            }
            ExprKind::StaticCall {
                namespace,
                name,
                arguments,
            } => {
                let callable = if let ExprKind::Identifier(namespace) = &namespace.kind {
                    if self.scopes.iter().any(|s| s.aliases.contains(namespace)) {
                        Type::unknown()
                    } else {
                        self.lookup(&format!("{namespace}::{name}"), true)
                    }
                } else {
                    self.expression(namespace);
                    Type::unknown()
                };
                self.call(callable, arguments, expr.span)
            }
            ExprKind::New {
                namespace,
                name,
                arguments,
            } => {
                let class = if namespace.is_none() {
                    self.class(name)
                } else {
                    None
                };
                let constructor = class
                    .as_ref()
                    .and_then(|c| c.methods.iter().find(|m| m.name == "__construct"))
                    .map(|f| Type::callable(Signature::script(f)))
                    .unwrap_or_else(|| {
                        if class.as_ref().is_some_and(|c| c.parent.is_none()) {
                            Type::callable(Signature {
                                parameters: Vec::new(),
                                required: 0,
                                variadic: None,
                                returns: Type::of(Kind::Null),
                            })
                        } else {
                            Type::unknown()
                        }
                    });
                self.call(constructor, arguments, expr.span);
                Type::of(Kind::Object(
                    namespace
                        .as_ref()
                        .map_or_else(|| name.clone(), |ns| format!("{ns}::{name}")),
                ))
            }
            ExprKind::PropertyAccess(base, name) => {
                let ty = self.expression(base);
                let objects = Type::of(Kind::Object("object".into()));
                self.expect(&ty, &objects, base.span, "property access");
                if let Some(class) = self.type_class(&ty) {
                    if let Some(method) = class.methods.iter().find(|m| m.name == *name) {
                        return Type::callable(Signature::script(method));
                    }
                }
                // A successful read of an annotated field always satisfies its contract.
                self.field_type(&ty, name).unwrap_or_else(Type::unknown)
            }
            ExprKind::IndexAccess(base, index) => {
                let base_ty = self.expression(base);
                let key = self.expression(index);
                self.expect(
                    &base_ty,
                    &Type::of(Kind::List).union(&Type::of(Kind::Map)),
                    base.span,
                    "index access",
                );
                let expected = if base_ty == Type::of(Kind::List) {
                    Type::of(Kind::Int)
                } else {
                    Type::of(Kind::Int).union(&Type::of(Kind::String))
                };
                self.expect(&key, &expected, index.span, "index");
                Type::unknown()
            }
            ExprKind::AppendAccess(base) => {
                let ty = self.expression(base);
                self.expect(&ty, &Type::of(Kind::List), base.span, "append");
                Type::unknown()
            }
            ExprKind::Slice {
                base,
                start,
                stop,
                step,
            } => {
                let ty = self.expression(base);
                self.expect(&ty, &Type::of(Kind::List), base.span, "slice");
                for bound in [start, stop, step].into_iter().flatten() {
                    let ty = self.expression(bound);
                    self.expect(&ty, &Type::of(Kind::Int), bound.span, "slice bound");
                }
                Type::of(Kind::List)
            }
            ExprKind::List(items) => {
                for item in items {
                    self.expression(item);
                }
                Type::of(Kind::List)
            }
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    let ty = self.expression(key);
                    self.expect(
                        &ty,
                        &Type::of(Kind::Int).union(&Type::of(Kind::String)),
                        key.span,
                        "map key",
                    );
                    self.expression(value);
                }
                Type::of(Kind::Map)
            }
            ExprKind::Spread(inner) => {
                let ty = self.expression(inner);
                self.expect(&ty, &Type::of(Kind::List), inner.span, "spread");
                Type::of(Kind::List)
            }
            ExprKind::InterpolatedString(items) => {
                for item in items {
                    let ty = self.expression(item);
                    self.expect(
                        &ty,
                        &Type::of(Kind::String)
                            .union(&Type::of(Kind::Int))
                            .union(&Type::of(Kind::Float))
                            .union(&Type::of(Kind::Bool)),
                        item.span,
                        "interpolation",
                    );
                }
                Type::of(Kind::String)
            }
            ExprKind::Comprehension {
                value,
                binding,
                iterable,
                condition,
            } => {
                let ty = self.expression(iterable);
                self.expect(
                    &ty,
                    &Type::of(Kind::List).union(&Type::of(Kind::Map)),
                    iterable.span,
                    "comprehension",
                );
                let before = self.scopes.clone();
                self.scopes.push(Scope::default());
                self.scopes
                    .last_mut()
                    .unwrap()
                    .variables
                    .insert(binding.name.clone(), Type::unknown());
                if let Some(condition) = condition {
                    self.condition(condition);
                }
                self.expression(value);
                self.scopes.pop();
                let after = self.scopes.clone();
                self.merge(&[before.clone(), after], &before);
                Type::of(Kind::List)
            }
        }
    }

    fn binary(&mut self, op: BinaryOp, a: &Type, b: &Type, span: Span) -> Type {
        use BinaryOp as B;
        if matches!(op, B::Equal | B::NotEqual) {
            return Type::of(Kind::Bool);
        }
        if matches!(op, B::And | B::Or) {
            for ty in [a, b] {
                self.expect(ty, &Type::of(Kind::Bool), span, "logical operator");
            }
            return Type::of(Kind::Bool);
        }
        if op == B::In {
            self.expect(
                b,
                &Type::of(Kind::List)
                    .union(&Type::of(Kind::Map))
                    .union(&Type::of(Kind::String)),
                span,
                "membership",
            );
            if *b == Type::of(Kind::String) {
                self.expect(a, &Type::of(Kind::String), span, "string membership");
            }
            if *b == Type::of(Kind::Map) {
                self.expect(
                    a,
                    &Type::of(Kind::Int).union(&Type::of(Kind::String)),
                    span,
                    "map membership",
                );
            }
            return Type::of(Kind::Bool);
        }
        let comparison = matches!(op, B::Less | B::LessEqual | B::Greater | B::GreaterEqual);
        let allowed = if op == B::Concat {
            vec![Kind::String]
        } else if comparison {
            vec![Kind::Int, Kind::Float, Kind::String]
        } else {
            vec![Kind::Int, Kind::Float]
        };
        let possible: BTreeSet<_> = allowed
            .into_iter()
            .filter(|kind| {
                a.overlaps(&Type::of(kind.clone())) && b.overlaps(&Type::of(kind.clone()))
            })
            .collect();
        if possible.is_empty() {
            self.error(
                format!(
                    "incompatible operands for {op:?}: {} and {}; matching types required",
                    a.display(),
                    b.display()
                ),
                span,
            );
            return Type::unknown();
        }
        if comparison {
            Type::of(Kind::Bool)
        } else {
            Type {
                kinds: Some(possible),
                signature: None,
            }
        }
    }

    fn call(&mut self, callable: Type, arguments: &[Expr], span: Span) -> Type {
        self.expect(&callable, &Type::of(Kind::Callable), span, "call");
        let mut unpacked = false;
        let mut values = Vec::new();
        for argument in arguments {
            if let ExprKind::Spread(inner) = &argument.kind {
                if let ExprKind::List(items) = &inner.kind {
                    if !items.iter().any(|e| matches!(e.kind, ExprKind::Spread(_))) {
                        for item in items {
                            values.push((self.expression(item), item.span));
                        }
                        continue;
                    }
                }
                unpacked = true;
            }
            values.push((self.expression(argument), argument.span));
        }
        self.invalidate_calls();
        let Some(signature) = callable.signature else {
            return Type::unknown();
        };
        if !unpacked {
            if values.len() < signature.required
                || signature.variadic.is_none() && values.len() > signature.parameters.len()
            {
                self.error(
                    format!(
                        "call expects {} to {} arguments, got {}",
                        signature.required,
                        signature.variadic.as_ref().map_or_else(
                            || signature.parameters.len().to_string(),
                            |_| "unlimited".into()
                        ),
                        values.len()
                    ),
                    span,
                );
            }
            for (index, (actual, span)) in values.iter().enumerate() {
                if let Some(expected) = signature
                    .parameters
                    .get(index)
                    .or(signature.variadic.as_ref())
                {
                    self.expect(actual, expected, *span, &format!("argument {}", index + 1));
                }
            }
        }
        signature.returns.clone()
    }

    fn class(&self, name: &str) -> Option<ClassDeclaration> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.classes.get(name))
            .cloned()
    }

    fn type_class(&self, ty: &Type) -> Option<ClassDeclaration> {
        let kinds = ty.kinds.as_ref()?;
        if kinds.len() != 1 {
            return None;
        }
        if let Kind::Object(name) = kinds.first()? {
            self.class(name)
        } else {
            None
        }
    }

    fn receiver_type(&self, expr: &Expr) -> Type {
        match &expr.kind {
            ExprKind::Variable(name) => self.lookup(name, false),
            ExprKind::Grouped(inner) => self.receiver_type(inner),
            _ => Type::unknown(),
        }
    }

    fn field_type(&self, ty: &Type, name: &str) -> Option<Type> {
        self.type_class(ty)?
            .fields
            .iter()
            .find(|f| f.binding.name == name)?
            .annotation
            .as_ref()
            .map(|a| Type::annotation(a, false))
    }

    fn forget_writes(&mut self, statement: &Statement) {
        let mut names = HashSet::new();
        visit_statement(
            statement,
            &mut |expr| {
                if let ExprKind::Assignment(target, _, _) = &expr.kind {
                    if let ExprKind::Variable(name) = &ungroup(target).kind {
                        names.insert(name.clone());
                    }
                }
            },
            false,
        );
        for scope in &mut self.scopes {
            for (name, ty) in &mut scope.variables {
                if names.contains(name) {
                    *ty = Type::unknown();
                }
            }
        }
    }

    fn collect_captures(&mut self, statement: &Statement) {
        let names = &mut self.captured_writes;
        let mut collect = |expr: &Expr| {
            if let ExprKind::Assignment(target, _, _) = &expr.kind {
                if let ExprKind::Variable(name) = &ungroup(target).kind {
                    names.insert(name.clone());
                }
            }
        };
        statement.visit_expressions(|expr| {
            visit_expression(
                expr,
                &mut |expr| {
                    if let ExprKind::Function(function) = &expr.kind {
                        visit_function(function, &mut collect);
                    }
                },
                false,
            )
        });
        match &statement.kind {
            StatementKind::Function(function) => visit_function(function, &mut collect),
            StatementKind::Class(class) => {
                for field in &class.fields {
                    if let Some(expr) = &field.default {
                        visit_expression(expr, &mut collect, true);
                    }
                }
                for method in &class.methods {
                    visit_function(method, &mut collect);
                }
            }
            StatementKind::Block(block) => {
                for s in &block.statements {
                    self.collect_captures(s);
                }
            }
            StatementKind::If {
                branches,
                else_branch,
            } => {
                for (_, body) in branches {
                    self.collect_captures(body);
                }
                if let Some(body) = else_branch {
                    self.collect_captures(body);
                }
            }
            StatementKind::While { body, .. }
            | StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. } => self.collect_captures(body),
            StatementKind::Try {
                body,
                catches,
                finally,
            } => {
                for block in std::iter::once(body)
                    .chain(catches.iter().map(|c| &c.body))
                    .chain(finally)
                {
                    for s in &block.statements {
                        self.collect_captures(s);
                    }
                }
            }
            _ => {}
        }
    }
}

fn visit_expression(expr: &Expr, visit: &mut impl FnMut(&Expr), functions: bool) {
    visit(expr);
    if let ExprKind::Function(function) = &expr.kind {
        if functions {
            visit_function(function, visit);
        }
    } else {
        expr.visit_children(|child| visit_expression(child, visit, functions));
    }
}

fn visit_function(function: &FunctionDeclaration, visit: &mut impl FnMut(&Expr)) {
    for parameter in &function.parameters {
        if let Some(default) = &parameter.default {
            visit_expression(default, visit, true);
        }
    }
    for statement in &function.body.statements {
        visit_statement(statement, visit, true);
    }
}

fn visit_statement(statement: &Statement, visit: &mut impl FnMut(&Expr), functions: bool) {
    statement.visit_expressions(|expr| visit_expression(expr, visit, functions));
    match &statement.kind {
        StatementKind::Function(f) if functions => visit_function(f, visit),
        StatementKind::Class(c) if functions => {
            for field in &c.fields {
                if let Some(default) = &field.default {
                    visit_expression(default, visit, true);
                }
            }
            for method in &c.methods {
                visit_function(method, visit);
            }
        }
        StatementKind::Block(block) => {
            for s in &block.statements {
                visit_statement(s, visit, functions);
            }
        }
        StatementKind::If {
            branches,
            else_branch,
        } => {
            for (_, body) in branches {
                visit_statement(body, visit, functions);
            }
            if let Some(body) = else_branch {
                visit_statement(body, visit, functions);
            }
        }
        StatementKind::While { body, .. }
        | StatementKind::For { body, .. }
        | StatementKind::Foreach { body, .. } => visit_statement(body, visit, functions),
        StatementKind::Try {
            body,
            catches,
            finally,
        } => {
            for block in std::iter::once(body)
                .chain(catches.iter().map(|c| &c.body))
                .chain(finally)
            {
                for statement in &block.statements {
                    visit_statement(statement, visit, functions);
                }
            }
        }
        _ => {}
    }
}

fn ungroup(expr: &Expr) -> &Expr {
    if let ExprKind::Grouped(inner) = &expr.kind {
        ungroup(inner)
    } else {
        expr
    }
}

fn literal_bool(expr: &Expr) -> Option<bool> {
    if let ExprKind::Literal(Literal::Bool(value)) = &ungroup(expr).kind {
        Some(*value)
    } else {
        None
    }
}

fn has_break(statement: &Statement) -> bool {
    match &statement.kind {
        StatementKind::Break => true,
        StatementKind::Block(block) => block.statements.iter().any(has_break),
        StatementKind::If {
            branches,
            else_branch,
        } => {
            branches.iter().any(|(_, s)| has_break(s))
                || else_branch.as_ref().is_some_and(|s| has_break(s))
        }
        StatementKind::Try {
            body,
            catches,
            finally,
        } => std::iter::once(body)
            .chain(catches.iter().map(|c| &c.body))
            .chain(finally)
            .any(|b| b.statements.iter().any(has_break)),
        _ => false,
    }
}
