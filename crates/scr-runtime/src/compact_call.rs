//! Experimental bounded instructions for noncapturing scalar function bodies.
//! The public AST stays immutable. Unsupported bodies are rejected before entry.
use crate::{Result, Runtime, Value, ops, value::UserDeclaration};
use scr_ast::{AssignmentOp, BinaryOp, Expr, ExprKind, Literal, Statement, StatementKind, UnaryOp};
use scr_diagnostics::Span;

const LOCALS: usize = 16;
const STACK: usize = 32;
const DEPTH: usize = 16;
const NODES: usize = 256;

#[path = "compact_frames.rs"]
mod frames;
pub(crate) use frames::Stack;

#[derive(Clone, Copy)]
enum Scalar {
    Null,
    Bool(bool),
    Int(i64),
}
impl Scalar {
    fn from_value(value: &Value) -> Option<Self> {
        Some(match value {
            Value::Null => Self::Null,
            Value::Bool(v) => Self::Bool(*v),
            Value::Int(v) => Self::Int(*v),
            _ => return None,
        })
    }
    fn value(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(v) => Value::Bool(v),
            Self::Int(v) => Value::Int(v),
        }
    }
}

#[derive(Clone, Copy)]
enum Code {
    Tick,
    Enter,
    Leave,
    Constant,
    Local,
    Store,
    Unary(UnaryOp),
    Binary(BinaryOp),
    Boolean,
    Drop,
    Jump,
    False,
    Short(bool),
    Call,
    Return,
}
#[derive(Clone, Copy)]
struct Instruction {
    code: Code,
    operand: u32,
}
pub(crate) struct Plan {
    code: Box<[Instruction]>,
    spans: Box<[Span]>,
    constants: Box<[Scalar]>,
    calls: Box<[CallSite]>,
    repeats: bool,
    frames_disabled: std::cell::Cell<bool>,
    base_return: Option<Box<BaseReturn>>,
}
struct BaseReturn {
    argument: usize,
    comparison: BinaryOp,
    limit: i64,
}
impl BaseReturn {
    fn compile(declaration: &UserDeclaration) -> Option<Box<Self>> {
        let mut aliases: Vec<(&str, usize)> = declaration
            .parameters
            .iter()
            .enumerate()
            .map(|(index, p)| (p.binding.name.as_str(), index))
            .collect();
        for statement in &declaration.body.statements {
            if let StatementKind::Expression(Expr {
                kind: ExprKind::Assignment(target, AssignmentOp::Assign, rhs),
                ..
            }) = &statement.kind
            {
                let (ExprKind::Variable(target), ExprKind::Variable(source)) =
                    (&target.kind, &rhs.kind)
                else {
                    return None;
                };
                let argument = aliases.iter().find(|(name, _)| *name == source)?.1;
                if let Some((_, slot)) = aliases.iter_mut().find(|(name, _)| *name == target) {
                    *slot = argument;
                } else {
                    aliases.push((target, argument));
                }
                continue;
            }
            let StatementKind::If { branches, .. } = &statement.kind else {
                return None;
            };
            let (condition, body) = branches.first()?;
            let body = match &body.kind {
                StatementKind::Block(block) if block.statements.len() == 1 => &block.statements[0],
                _ => body,
            };
            let StatementKind::Return(value) = &body.kind else {
                return None;
            };
            if value
                .as_ref()
                .is_some_and(|e| !matches!(e.kind, ExprKind::Literal(_) | ExprKind::Variable(_)))
            {
                return None;
            }
            let ExprKind::Binary(left, comparison, right) = &condition.kind else {
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
            let (ExprKind::Variable(name), ExprKind::Literal(Literal::Int(limit))) =
                (&left.kind, &right.kind)
            else {
                return None;
            };
            let argument = aliases.iter().find(|(alias, _)| *alias == name)?.1;
            return Some(Box::new(Self {
                argument,
                comparison: *comparison,
                limit: *limit,
            }));
        }
        None
    }
    fn admits(&self, arguments: &[Value]) -> bool {
        let Some(Value::Int(value)) = arguments.get(self.argument) else {
            return false;
        };
        match self.comparison {
            BinaryOp::Equal => *value == self.limit,
            BinaryOp::NotEqual => *value != self.limit,
            BinaryOp::Less => *value < self.limit,
            BinaryOp::LessEqual => *value <= self.limit,
            BinaryOp::Greater => *value > self.limit,
            BinaryOp::GreaterEqual => *value >= self.limit,
            _ => unreachable!("compiled comparison"),
        }
    }
}
struct CallSite {
    name: String,
    arguments: usize,
}
struct Compiler<'a> {
    names: Vec<&'a str>,
    code: Vec<Instruction>,
    spans: Vec<Span>,
    constants: Vec<Scalar>,
    calls: Vec<CallSite>,
    nodes: usize,
}
impl<'a> Compiler<'a> {
    fn node(&mut self, depth: usize) -> Option<()> {
        self.nodes += 1;
        (depth <= DEPTH && self.nodes <= NODES).then_some(())
    }
    fn emit(&mut self, code: Code, operand: usize, span: Span) -> usize {
        let position = self.code.len();
        self.code.push(Instruction {
            code,
            operand: operand as u32,
        });
        self.spans.push(span);
        position
    }
    fn patch(&mut self, instruction: usize) {
        self.code[instruction].operand = self.code.len() as u32;
    }
    fn constant(&mut self, value: Scalar, span: Span) {
        let index = self.constants.len();
        self.constants.push(value);
        self.emit(Code::Constant, index, span);
    }
    fn local(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| *n == name)
    }
    fn expression(&mut self, e: &'a Expr, depth: usize) -> Option<()> {
        self.node(depth)?;
        let mut remaining = NODES;
        if stack_bound(e, 0, &mut remaining)? > STACK {
            return None;
        }
        match &e.kind {
            ExprKind::Literal(value) => self.constant(
                match value {
                    Literal::Int(v) => Scalar::Int(*v),
                    Literal::Bool(v) => Scalar::Bool(*v),
                    Literal::Null => Scalar::Null,
                    _ => return None,
                },
                e.span,
            ),
            ExprKind::Variable(name) => {
                self.emit(Code::Local, self.local(name)?, e.span);
            }
            ExprKind::Grouped(inner) => self.expression(inner, depth + 1)?,
            ExprKind::Unary(op, inner) => {
                self.expression(inner, depth + 1)?;
                self.emit(Code::Unary(*op), 0, e.span);
            }
            ExprKind::Binary(left, op, right) => {
                if matches!(op, BinaryOp::In | BinaryOp::Concat) {
                    return None;
                }
                self.expression(left, depth + 1)?;
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    let jump = self.emit(Code::Short(*op == BinaryOp::Or), 0, e.span);
                    self.expression(right, depth + 1)?;
                    self.emit(Code::Boolean, 0, e.span);
                    self.patch(jump);
                } else {
                    self.expression(right, depth + 1)?;
                    self.emit(Code::Binary(*op), 0, e.span);
                }
            }
            ExprKind::Assignment(target, op, rhs) => {
                let ExprKind::Variable(name) = &target.kind else {
                    return None;
                };
                let slot = self.local(name)?;
                let binary = match op {
                    AssignmentOp::Assign => None,
                    AssignmentOp::Add => Some(BinaryOp::Add),
                    AssignmentOp::Subtract => Some(BinaryOp::Subtract),
                    AssignmentOp::Multiply => Some(BinaryOp::Multiply),
                    AssignmentOp::Divide => Some(BinaryOp::Divide),
                    AssignmentOp::Concat => return None,
                };
                if binary.is_some() {
                    self.emit(Code::Local, slot, target.span);
                }
                self.expression(rhs, depth + 1)?;
                if let Some(op) = binary {
                    self.emit(Code::Binary(op), 0, target.span);
                }
                self.emit(Code::Store, slot, target.span);
            }
            ExprKind::Call(callee, arguments) => {
                let ExprKind::Identifier(name) = &callee.kind else {
                    return None;
                };
                if arguments.len() > 8 {
                    return None;
                }
                for argument in arguments {
                    self.expression(argument, depth + 1)?;
                }
                let site = self.calls.len();
                self.calls.push(CallSite {
                    name: name.clone(),
                    arguments: arguments.len(),
                });
                self.emit(Code::Call, site, e.span);
            }
            _ => return None,
        }
        Some(())
    }
    fn statement(&mut self, s: &'a Statement, depth: usize, root: bool) -> Option<()> {
        self.node(depth)?;
        self.emit(Code::Tick, 0, s.span);
        match &s.kind {
            StatementKind::Expression(e) => {
                // Only unconditional body-level assignments introduce locals.
                // This excludes conditional declarations, captures and lexical
                // shadowing that cannot be represented by fixed local slots.
                let new_name = if root {
                    if let ExprKind::Assignment(target, AssignmentOp::Assign, _) = &e.kind {
                        if let ExprKind::Variable(name) = &target.kind {
                            (self.local(name).is_none() && name != "this").then_some(name.as_str())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(name) = new_name {
                    let ExprKind::Assignment(_, _, rhs) = &e.kind else {
                        unreachable!()
                    };
                    // Compile RHS before introducing the name: self-referencing
                    // first assignments retain the ordinary undefined-name path.
                    self.expression(rhs, depth + 1)?;
                    if self.names.len() == LOCALS {
                        return None;
                    }
                    let slot = self.names.len();
                    self.names.push(name);
                    self.emit(Code::Store, slot, e.span);
                } else {
                    self.expression(e, depth + 1)?;
                }
                self.emit(Code::Drop, 0, e.span);
            }
            StatementKind::Return(value) => {
                if let Some(e) = value {
                    self.expression(e, depth + 1)?;
                } else {
                    self.constant(Scalar::Null, s.span);
                }
                self.emit(Code::Return, 0, s.span);
            }
            StatementKind::Block(block) => {
                self.emit(Code::Enter, 0, block.span);
                for statement in &block.statements {
                    self.statement(statement, depth + 1, false)?;
                }
                self.emit(Code::Leave, 0, block.span);
            }
            StatementKind::If {
                branches,
                else_branch,
            } => {
                let mut ends = Vec::new();
                for (condition, body) in branches {
                    self.expression(condition, depth + 1)?;
                    let next = self.emit(Code::False, 0, condition.span);
                    self.statement(body, depth + 1, false)?;
                    ends.push(self.emit(Code::Jump, 0, s.span));
                    self.patch(next);
                }
                if let Some(body) = else_branch {
                    self.statement(body, depth + 1, false)?;
                }
                for end in ends {
                    self.patch(end);
                }
            }
            StatementKind::While { condition, body } => {
                let head = self.code.len();
                self.expression(condition, depth + 1)?;
                let end = self.emit(Code::False, 0, condition.span);
                self.emit(Code::Tick, 0, s.span);
                self.statement(body, depth + 1, false)?;
                self.emit(Code::Jump, head, s.span);
                self.patch(end);
            }
            _ => return None,
        }
        Some(())
    }
}
// Include already evaluated call arguments when bounding the operand stack.
// Tree depth alone is insufficient for nested multi-argument calls.
fn stack_bound(expression: &Expr, depth: usize, remaining: &mut usize) -> Option<usize> {
    *remaining = remaining.checked_sub(1)?;
    if depth > DEPTH {
        return None;
    }
    Some(match &expression.kind {
        ExprKind::Grouped(e) | ExprKind::Unary(_, e) => stack_bound(e, depth + 1, remaining)?,
        ExprKind::Binary(left, op, right) => stack_bound(left, depth + 1, remaining)?.max(
            usize::from(!matches!(op, BinaryOp::And | BinaryOp::Or))
                + stack_bound(right, depth + 1, remaining)?,
        ),
        ExprKind::Assignment(_, op, rhs) => {
            usize::from(*op != AssignmentOp::Assign) + stack_bound(rhs, depth + 1, remaining)?
        }
        ExprKind::Call(_, arguments) => {
            if arguments.len() > 8 {
                return None;
            }
            let mut needed = 1;
            for (index, argument) in arguments.iter().enumerate() {
                needed = needed.max(index + stack_bound(argument, depth + 1, remaining)?);
            }
            needed
        }
        _ => 1,
    })
}

impl Plan {
    fn compile(declaration: &UserDeclaration) -> Option<Box<Self>> {
        if declaration.parameters.len() > 8
            || declaration.parameters.iter().enumerate().any(|(i, p)| {
                p.default.is_some()
                    || p.variadic
                    || p.binding.name == "this"
                    || declaration.parameters[..i]
                        .iter()
                        .any(|q| q.binding.name == p.binding.name)
            })
        {
            return None;
        }
        let mut c = Compiler {
            names: declaration
                .parameters
                .iter()
                .map(|p| p.binding.name.as_str())
                .collect(),
            code: Vec::new(),
            spans: Vec::new(),
            constants: Vec::new(),
            calls: Vec::new(),
            nodes: 0,
        };
        c.emit(Code::Enter, 0, declaration.body.span);
        for statement in &declaration.body.statements {
            c.statement(statement, 0, true)?;
        }
        c.constant(Scalar::Null, declaration.body.span);
        c.emit(Code::Return, 0, declaration.body.span);
        // Every expression's operand demand is checked against STACK, including
        // pending arguments of nested calls. Compilation is also node/depth bounded.
        let repeats = c.code.iter().enumerate().any(|(index, instruction)| {
            matches!(instruction.code, Code::Jump) && instruction.operand as usize <= index
        });
        let base_return = if c.calls.is_empty() {
            None
        } else {
            BaseReturn::compile(declaration)
        };
        Some(Box::new(Self {
            code: c.code.into(),
            spans: c.spans.into(),
            constants: c.constants.into(),
            calls: c.calls.into(),
            repeats,
            frames_disabled: std::cell::Cell::new(false),
            base_return,
        }))
    }
}
impl Runtime {
    pub(super) fn compact_call(
        &mut self,
        function: &crate::value::Function,
        declaration: &UserDeclaration,
        arguments: &[Value],
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_compact_calls {
            return None;
        }
        if arguments.iter().any(|v| Scalar::from_value(v).is_none()) {
            return None;
        }
        let plan = declaration
            .compact_call
            .get_or_init(|| Plan::compile(declaration))
            .as_deref()?;
        if !plan.calls.is_empty() {
            #[cfg(test)]
            let force = self.force_compact_frames;
            #[cfg(not(test))]
            let force = false;
            if !force
                && (plan.frames_disabled.get()
                    || plan
                        .base_return
                        .as_ref()
                        .is_some_and(|guard| guard.admits(arguments)))
            {
                return None;
            }
            return self.compact_frames(function, arguments);
        }
        #[cfg(test)]
        {
            self.compact_calls_executed += 1;
        }
        Some(self.run_compact(declaration, plan, arguments))
    }
    #[inline(never)]
    fn run_compact(
        &mut self,
        declaration: &UserDeclaration,
        plan: &Plan,
        arguments: &[Value],
    ) -> Result<Value> {
        for (parameter, value) in declaration.parameters.iter().zip(arguments) {
            self.annotation(&parameter.annotation, value, parameter.span, false)?;
        }
        let mut locals = [Scalar::Null; LOCALS];
        for (local, value) in locals.iter_mut().zip(arguments) {
            *local = Scalar::from_value(value).unwrap();
        }
        let mut values = [Scalar::Null; STACK];
        let mut top = 0;
        let mut scopes = [0usize; DEPTH + 3];
        let mut scope_count = 0;
        let mut ip = 0;
        let result = (|| {
            loop {
                let Instruction { code, operand } = plan.code[ip];
                let span = plan.spans[ip];
                let operand = operand as usize;
                ip += 1;
                match code {
                    Code::Tick => self.checkpoint(span)?,
                    Code::Enter => {
                        scopes[scope_count] =
                            self.enter_empty_scope(self.environments.current, span)?;
                        scope_count += 1;
                    }
                    Code::Leave => {
                        scope_count -= 1;
                        self.environments.leave_block(scopes[scope_count]);
                    }
                    Code::Constant | Code::Local => {
                        values[top] = if matches!(code, Code::Constant) {
                            plan.constants[operand]
                        } else {
                            locals[operand]
                        };
                        top += 1;
                    }
                    Code::Store => locals[operand] = values[top - 1],
                    Code::Drop => top -= 1,
                    Code::Unary(op) => {
                        values[top - 1] = Scalar::from_value(
                            &self.operation(ops::unary(op, values[top - 1].value()), span)?,
                        )
                        .unwrap();
                    }
                    Code::Binary(op) => {
                        top -= 1;
                        values[top - 1] = Scalar::from_value(&self.operation(
                            ops::binary(op, values[top - 1].value(), values[top].value()),
                            span,
                        )?)
                        .unwrap();
                    }
                    Code::Boolean => {
                        values[top - 1] = Scalar::Bool(
                            self.operation(ops::boolean(values[top - 1].value()), span)?,
                        );
                    }
                    Code::Jump => ip = operand,
                    Code::False => {
                        top -= 1;
                        if !self.operation(ops::boolean(values[top].value()), span)? {
                            ip = operand;
                        }
                    }
                    Code::Short(stop) => {
                        let value = self.operation(ops::boolean(values[top - 1].value()), span)?;
                        if value == stop {
                            values[top - 1] = Scalar::Bool(value);
                            ip = operand;
                        } else {
                            top -= 1;
                        }
                    }
                    Code::Return => break Ok(values[top - 1].value()),
                    Code::Call => unreachable!("call plans use explicit VM frames"),
                }
            }
        })();
        while scope_count > 0 {
            scope_count -= 1;
            self.environments.leave_block(scopes[scope_count]);
        }
        let value = result?;
        self.annotation(
            &declaration.return_type,
            &value,
            declaration.name_span,
            true,
        )?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilation_bounds_fall_back_and_plan_storage_is_reported() {
        for count in [LOCALS, LOCALS + 1] {
            let assignments = (0..count)
                .map(|i| format!("$v{i}={i};"))
                .collect::<String>();
            let mut runtime = Runtime::new();
            runtime
                .execute_source(
                    &format!("function f(){{{assignments}return $v0;}}$seen=f();"),
                    &mut Vec::new(),
                )
                .unwrap();
            assert_eq!(runtime.compact_calls_executed, usize::from(count == LOCALS));
            assert_eq!(runtime.get_global("seen"), Some(&Value::Int(0)));
            let Value::Callable(function) =
                runtime.environments.scope(0).functions.get("f").unwrap()
            else {
                panic!("function")
            };
            let crate::value::Function::User { declaration, .. } = function.0.as_ref() else {
                panic!("user")
            };
            if let Some(Some(plan)) = declaration.compact_call.get() {
                println!(
                    "instruction_bytes={} span_bytes={} scalar_bytes={} cache_cell_bytes={} instructions={} plan_payload_bytes={} fixed_value_stack_bytes={}",
                    std::mem::size_of::<Instruction>(),
                    std::mem::size_of::<Span>(),
                    std::mem::size_of::<Scalar>(),
                    std::mem::size_of_val(&declaration.compact_call),
                    plan.code.len(),
                    std::mem::size_of_val(plan.code.as_ref())
                        + std::mem::size_of_val(plan.spans.as_ref())
                        + std::mem::size_of_val(plan.constants.as_ref()),
                    std::mem::size_of::<Scalar>() * (LOCALS + STACK)
                );
            }
        }
        // Bound compiler work by node count even for otherwise eligible syntax.
        let source = format!(
            "function f(){{$v=0;{}return $v;}}$seen=f();",
            "$v+=1;".repeat(NODES)
        );
        let mut runtime = Runtime::new();
        runtime.execute_source(&source, &mut Vec::new()).unwrap();
        assert_eq!(runtime.compact_calls_executed, 0);
        assert_eq!(runtime.get_global("seen"), Some(&Value::Int(NODES as i64)));
    }
}
