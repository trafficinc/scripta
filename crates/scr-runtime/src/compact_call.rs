//! Experimental bounded instructions for noncapturing scalar function bodies.
//! The public AST stays immutable. Unsupported bodies are rejected before entry.
use crate::{Result, Runtime, Value, ops, value::UserDeclaration};
use scr_ast::{AssignmentOp, BinaryOp, Expr, ExprKind, Literal, Statement, StatementKind, UnaryOp};
use scr_diagnostics::Span;

const LOCALS: usize = 16;
const STACK: usize = 32;
const DEPTH: usize = 16;
const NODES: usize = 256;

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
}
struct Compiler<'a> {
    names: Vec<&'a str>,
    code: Vec<Instruction>,
    spans: Vec<Span>,
    constants: Vec<Scalar>,
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
            nodes: 0,
        };
        c.emit(Code::Enter, 0, declaration.body.span);
        for statement in &declaration.body.statements {
            c.statement(statement, 0, true)?;
        }
        c.constant(Scalar::Null, declaration.body.span);
        c.emit(Code::Return, 0, declaration.body.span);
        // Expressions are depth-bounded, with at most one extra value per level.
        // An assignment can add one old value at each level as well.
        // DEPTH=16 therefore bounds the operand stack at 32 values.
        Some(Box::new(Self {
            code: c.code.into(),
            spans: c.spans.into(),
            constants: c.constants.into(),
        }))
    }
}
impl Runtime {
    pub(super) fn compact_call(
        &mut self,
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
