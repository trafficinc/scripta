//! Bounded plans for pure, global, self-recursive scalar functions.
use crate::{
    Result, Runtime, Value,
    error::ActiveFrame,
    value::{Function, UserDeclaration},
};
use scr_ast::{BinaryOp, Expr, ExprKind, Literal, Statement, StatementKind, UnaryOp};
use scr_diagnostics::Span;
use std::rc::Rc;

const MAX_PARAMETERS: usize = 8;
const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scalar {
    Int(i64),
    Bool(bool),
    Null,
}
impl Scalar {
    fn value(self) -> Value {
        match self {
            Self::Int(v) => Value::Int(v),
            Self::Bool(v) => Value::Bool(v),
            Self::Null => Value::Null,
        }
    }
}
impl Scalar {
    fn boolean(self) -> crate::ops::Result<bool> {
        if let Self::Bool(v) = self {
            Ok(v)
        } else {
            Err(crate::ops::Error::Type(
                "boolean condition requires bool; use bool(value) explicitly",
            ))
        }
    }
    fn unary(self, op: UnaryOp) -> crate::ops::Result<Self> {
        match (op, self) {
            (UnaryOp::Not, value) => value.boolean().map(|v| Self::Bool(!v)),
            (UnaryOp::Positive, value @ Self::Int(_)) => Ok(value),
            (UnaryOp::Negative, Self::Int(v)) => v
                .checked_neg()
                .map(Self::Int)
                .ok_or(crate::ops::Error::Arithmetic("integer arithmetic overflow")),
            _ => Err(crate::ops::Error::Type(
                "unary arithmetic requires a numeric value",
            )),
        }
    }
    fn binary(self, op: BinaryOp, other: Self) -> crate::ops::Result<Self> {
        if matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
            return Ok(Self::Bool(if op == BinaryOp::Equal {
                self == other
            } else {
                self != other
            }));
        }
        let (Self::Int(a), Self::Int(b)) = (self, other) else {
            return Err(crate::ops::Error::Type(
                if matches!(
                    op,
                    BinaryOp::Less
                        | BinaryOp::LessEqual
                        | BinaryOp::Greater
                        | BinaryOp::GreaterEqual
                ) {
                    "comparison requires matching numeric types or two strings; convert explicitly"
                } else {
                    "arithmetic requires matching numeric types; use int() or float() explicitly"
                },
            ));
        };
        Ok(match op {
            BinaryOp::Less => Self::Bool(a < b),
            BinaryOp::LessEqual => Self::Bool(a <= b),
            BinaryOp::Greater => Self::Bool(a > b),
            BinaryOp::GreaterEqual => Self::Bool(a >= b),
            _ => Self::Int(crate::ops::integer(op, a, b)?),
        })
    }
}

enum Op {
    Tick(Span),
    Constant(Scalar),
    Parameter(usize),
    Unary(UnaryOp, Span),
    Binary(BinaryOp, Span),
    Boolean(Span),
    Jump(usize),
    False(usize, Span),
    Short(bool, usize, Span),
    Enter(Span),
    Leave,
    Call(Span),
    Return,
}
struct EntryGuard {
    parameter: usize,
    comparison: BinaryOp,
    limit: i64,
    recursive_when: bool,
}
impl EntryGuard {
    fn admits(&self, arguments: &[Value]) -> bool {
        let Some(Value::Int(value)) = arguments.get(self.parameter) else {
            return false;
        };
        let condition = match self.comparison {
            BinaryOp::Equal => *value == self.limit,
            BinaryOp::NotEqual => *value != self.limit,
            BinaryOp::Less => *value < self.limit,
            BinaryOp::LessEqual => *value <= self.limit,
            BinaryOp::Greater => *value > self.limit,
            BinaryOp::GreaterEqual => *value >= self.limit,
            _ => unreachable!("compiled integer comparison"),
        };
        condition == self.recursive_when
    }
    fn compile(declaration: &UserDeclaration) -> Option<Self> {
        let [first, last] = declaration.body.statements.as_slice() else {
            return None;
        };
        let StatementKind::If {
            branches,
            else_branch: None,
        } = &first.kind
        else {
            return None;
        };
        let [(condition, branch)] = branches.as_slice() else {
            return None;
        };
        fn returned(s: &Statement) -> Option<&Expr> {
            match &s.kind {
                StatementKind::Return(Some(e)) => Some(e),
                StatementKind::Block(b) if b.statements.len() == 1 => returned(&b.statements[0]),
                _ => None,
            }
        }
        fn calls(e: &Expr) -> bool {
            if matches!(e.kind, ExprKind::Call(_, _)) {
                return true;
            }
            let mut found = false;
            e.visit_children(|child| found |= calls(child));
            found
        }
        // Compilation has already bounded every statement/expression by depth
        // and node count, and only self calls were accepted.
        let branch_calls = calls(returned(branch)?);
        if branch_calls == calls(returned(last)?) {
            return None;
        }
        fn must_call(e: &Expr) -> bool {
            match &e.kind {
                ExprKind::Call(_, _) => true,
                ExprKind::Grouped(inner) | ExprKind::Unary(_, inner) => must_call(inner),
                ExprKind::Binary(left, BinaryOp::And | BinaryOp::Or, _) => must_call(left),
                ExprKind::Binary(left, _, right) => must_call(left) || must_call(right),
                _ => false,
            }
        }
        if !must_call(if branch_calls {
            returned(branch)?
        } else {
            returned(last)?
        }) {
            return None;
        }
        let ExprKind::Binary(left, comparison, right) = &condition.kind else {
            return None;
        };
        let ExprKind::Variable(name) = &left.kind else {
            return None;
        };
        let ExprKind::Literal(Literal::Int(limit)) = right.kind else {
            return None;
        };
        if !matches!(
            comparison,
            BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
        ) {
            return None;
        }
        Some(Self {
            parameter: declaration
                .parameters
                .iter()
                .position(|p| p.binding.name == *name)?,
            comparison: *comparison,
            limit,
            recursive_when: branch_calls,
        })
    }
}
pub(crate) struct Plan {
    code: Vec<Op>,
    entry: EntryGuard,
}
struct Compiler<'a> {
    declaration: &'a UserDeclaration,
    code: Vec<Op>,
    nodes: usize,
    recursive: bool,
}
impl Compiler<'_> {
    fn node(&mut self, depth: usize) -> Option<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES || depth > MAX_DEPTH {
            None
        } else {
            Some(())
        }
    }
    fn emit(&mut self, op: Op) -> usize {
        let i = self.code.len();
        self.code.push(op);
        i
    }
    fn patch(&mut self, i: usize) {
        let dest = self.code.len();
        match &mut self.code[i] {
            Op::Jump(v) | Op::False(v, _) | Op::Short(_, v, _) => *v = dest,
            _ => unreachable!("compiled branch"),
        }
    }
    fn expression(&mut self, e: &Expr, depth: usize) -> Option<()> {
        self.node(depth)?;
        match &e.kind {
            ExprKind::Literal(l) => {
                self.emit(Op::Constant(match l {
                    Literal::Int(v) => Scalar::Int(*v),
                    Literal::Bool(v) => Scalar::Bool(*v),
                    Literal::Null => Scalar::Null,
                    _ => return None,
                }));
            }
            ExprKind::Variable(name) => {
                self.emit(Op::Parameter(
                    self.declaration
                        .parameters
                        .iter()
                        .position(|p| p.binding.name == *name)?,
                ));
            }
            ExprKind::Grouped(inner) => self.expression(inner, depth + 1)?,
            ExprKind::Unary(op, inner) => {
                self.expression(inner, depth + 1)?;
                self.emit(Op::Unary(*op, e.span));
            }
            ExprKind::Binary(a, op, b) if matches!(op, BinaryOp::And | BinaryOp::Or) => {
                self.expression(a, depth + 1)?;
                let jump = self.emit(Op::Short(*op == BinaryOp::Or, 0, e.span));
                self.expression(b, depth + 1)?;
                self.emit(Op::Boolean(e.span));
                self.patch(jump);
            }
            ExprKind::Binary(a, op, b) if !matches!(op, BinaryOp::Concat | BinaryOp::In) => {
                self.expression(a, depth + 1)?;
                self.expression(b, depth + 1)?;
                self.emit(Op::Binary(*op, e.span));
            }
            ExprKind::Call(callee, args)
                if matches!(&callee.kind,ExprKind::Identifier(name) if name==&self.declaration.name)
                    && args.len() == self.declaration.parameters.len() =>
            {
                for arg in args {
                    self.expression(arg, depth + 1)?;
                }
                self.recursive = true;
                self.emit(Op::Call(e.span));
            }
            _ => return None,
        }
        Some(())
    }
    fn statement(&mut self, s: &Statement, depth: usize) -> Option<()> {
        self.node(depth)?;
        self.emit(Op::Tick(s.span));
        match &s.kind {
            StatementKind::Return(expr) => {
                if let Some(e) = expr {
                    self.expression(e, depth + 1)?;
                } else {
                    self.emit(Op::Constant(Scalar::Null));
                }
                self.emit(Op::Return);
            }
            StatementKind::Block(b) => {
                self.emit(Op::Enter(b.span));
                for s in &b.statements {
                    self.statement(s, depth + 1)?;
                }
                self.emit(Op::Leave);
            }
            StatementKind::If {
                branches,
                else_branch,
            } => {
                let mut ends = Vec::new();
                for (condition, body) in branches {
                    self.expression(condition, depth + 1)?;
                    let skip = self.emit(Op::False(0, condition.span));
                    self.statement(body, depth + 1)?;
                    ends.push(self.emit(Op::Jump(0)));
                    self.patch(skip);
                }
                if let Some(body) = else_branch {
                    self.statement(body, depth + 1)?;
                }
                for i in ends {
                    self.patch(i);
                }
            }
            _ => return None,
        }
        Some(())
    }
}
impl Plan {
    fn compile(declaration: &UserDeclaration) -> Option<Box<Self>> {
        if declaration.parameters.len() > MAX_PARAMETERS
            || declaration.parameters.iter().any(|p| {
                p.default.is_some()
                    || p.variadic
                    || p.annotation.as_ref().is_some_and(|a| a.name != "int")
            })
            || declaration
                .return_type
                .as_ref()
                .is_some_and(|a| a.name != "int")
        {
            return None;
        }
        let mut c = Compiler {
            declaration,
            code: Vec::new(),
            nodes: 0,
            recursive: false,
        };
        c.emit(Op::Enter(declaration.body.span));
        for s in &declaration.body.statements {
            c.statement(s, 0)?;
        }
        c.emit(Op::Constant(Scalar::Null));
        c.emit(Op::Return);
        if !c.recursive {
            return None;
        }
        let entry = EntryGuard::compile(declaration)?;
        Some(Box::new(Self {
            code: c.code,
            entry,
        }))
    }
}
struct Frame {
    ip: usize,
    base: usize,
    scopes: usize,
    parameter: bool,
}
struct Scope {
    previous: usize,
    parameter: bool,
}
fn unwind(runtime: &mut Runtime, scopes: &mut Vec<Scope>, depth: usize) {
    while scopes.len() > depth {
        let scope = scopes.pop().unwrap();
        if scope.parameter {
            runtime
                .environments
                .scope_mut(runtime.environments.current)
                .function_boundary = false;
            runtime.environments.leave_empty(scope.previous);
        } else {
            runtime.environments.leave_block(scope.previous);
        }
    }
}

impl Runtime {
    pub(super) fn prepared_call(
        &mut self,
        function: &Function,
        declaration: &UserDeclaration,
        arguments: &[Value],
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_prepared_calls {
            return None;
        }
        if let Some(Some(plan)) = declaration.prepared_call.get() {
            // Trivial base cases stay on the AST without allocating VM storage.
            // This comparison cannot fail, spend instructions, or run script code.
            if !plan.entry.admits(arguments) {
                return None;
            }
        }
        // Existing numeric plans take precedence and supply the cold-call gate.
        // Root identity is rechecked even for a cached plan or host-held callable.
        if matches!(declaration.prepared_call.get(), Some(None))
            || self.options.max_scopes.is_some()
            || self.options.max_value_bytes.is_some()
            || !matches!(declaration.numeric_call.get(), Some(None))
            || arguments.iter().any(|v| !matches!(v, Value::Int(_)))
        {
            return None;
        }
        let Function::User {
            environment: 0,
            receiver: None,
            owner,
            ..
        } = function
        else {
            return None;
        };
        if !Rc::ptr_eq(owner, &self.identity) {
            return None;
        }
        let Some(Value::Callable(current)) =
            self.environments.scope(0).functions.get(&declaration.name)
        else {
            return None;
        };
        if !std::ptr::eq(current.0.as_ref(), function) {
            return None;
        }
        let plan = declaration
            .prepared_call
            .get_or_init(|| Plan::compile(declaration))
            .as_deref()?;
        if !plan.entry.admits(arguments) {
            return None;
        }
        #[cfg(test)]
        {
            self.prepared_calls_executed += 1;
        }
        Some(self.run_prepared(function, declaration, plan, arguments))
    }

    fn run_prepared(
        &mut self,
        function: &Function,
        declaration: &UserDeclaration,
        plan: &Plan,
        arguments: &[Value],
    ) -> Result<Value> {
        let stack_depth = self.stack.len();
        let mut values: Vec<Scalar> = arguments
            .iter()
            .map(|v| match v {
                Value::Int(v) => Scalar::Int(*v),
                _ => unreachable!("guarded integer arguments"),
            })
            .collect();
        let mut frames = vec![Frame {
            ip: 0,
            base: 0,
            scopes: 0,
            parameter: false,
        }];
        let mut scopes = Vec::new();
        let count = declaration.parameters.len();
        let result = (|| {
            for (parameter, value) in declaration.parameters.iter().zip(arguments) {
                self.annotation(&parameter.annotation, value, parameter.span, false)?;
            }
            loop {
                let frame = frames.last_mut().unwrap();
                let op = &plan.code[frame.ip];
                frame.ip += 1;
                let base = frame.base;
                match op {
                    Op::Tick(span) => self.checkpoint(*span)?,
                    Op::Constant(value) => values.push(*value),
                    Op::Parameter(index) => values.push(values[base + index]),
                    Op::Unary(op, span) => {
                        let value = values.pop().unwrap();
                        values.push(self.operation(value.unary(*op), *span)?);
                    }
                    Op::Binary(op, span) => {
                        let b = values.pop().unwrap();
                        let a = values.pop().unwrap();
                        values.push(self.operation(a.binary(*op, b), *span)?);
                    }
                    Op::Boolean(span) => {
                        let value = values.pop().unwrap();
                        values.push(Scalar::Bool(self.operation(value.boolean(), *span)?));
                    }
                    Op::Jump(ip) => frames.last_mut().unwrap().ip = *ip,
                    Op::False(ip, span) => {
                        let value = values.pop().unwrap();
                        if !self.operation(value.boolean(), *span)? {
                            frames.last_mut().unwrap().ip = *ip;
                        }
                    }
                    Op::Short(stop, ip, span) => {
                        let value = values.pop().unwrap();
                        let b = self.operation(value.boolean(), *span)?;
                        if b == *stop {
                            values.push(Scalar::Bool(b));
                            frames.last_mut().unwrap().ip = *ip;
                        }
                    }
                    Op::Enter(span) => {
                        let previous = self.enter_empty_scope(self.environments.current, *span)?;
                        scopes.push(Scope {
                            previous,
                            parameter: false,
                        });
                    }
                    Op::Leave => {
                        let depth = scopes.len() - 1;
                        unwind(self, &mut scopes, depth);
                    }
                    Op::Call(span) => {
                        if self.stack.len().saturating_sub(1)
                            >= self.options.recursion_limit.min(crate::MAX_RECURSION_LIMIT)
                        {
                            return self.fail("maximum call depth exceeded", *span);
                        }
                        self.stack
                            .push(ActiveFrame::call(function, *span, self.source.clone()));
                        let depth = scopes.len();
                        let previous = self.enter_empty_scope(0, *span)?;
                        self.environments
                            .scope_mut(self.environments.current)
                            .function_boundary = true;
                        scopes.push(Scope {
                            previous,
                            parameter: true,
                        });
                        let base = values.len() - count;
                        for (parameter, value) in declaration.parameters.iter().zip(&values[base..])
                        {
                            self.annotation(
                                &parameter.annotation,
                                &value.value(),
                                parameter.span,
                                false,
                            )?;
                        }
                        frames.push(Frame {
                            ip: 0,
                            base,
                            scopes: depth,
                            parameter: true,
                        });
                    }
                    Op::Return => {
                        let value = values.pop().unwrap();
                        let frame = frames.last().unwrap();
                        let depth = frame.scopes;
                        let parameter = frame.parameter;
                        unwind(self, &mut scopes, depth + usize::from(parameter));
                        self.annotation(
                            &declaration.return_type,
                            &value.value(),
                            declaration.name_span,
                            true,
                        )?;
                        unwind(self, &mut scopes, depth);
                        frames.pop();
                        values.truncate(base);
                        if frames.is_empty() {
                            return Ok(value.value());
                        }
                        self.stack.pop();
                        values.push(value);
                    }
                }
            }
        })();
        // Snapshot diagnostics/signals before dropping internal scopes and frames.
        unwind(self, &mut scopes, 0);
        self.stack.truncate(stack_depth);
        result
    }
}

#[cfg(test)]
mod tests;
