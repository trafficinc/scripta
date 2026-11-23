//! Integer caller slots around proven leaf calls; normal call semantics remain.
use crate::{Callable, Function, Result, Runtime, Value, numeric_call::Plan, ops};
use scr_ast::{AssignmentOp, BinaryOp, Expr, ExprKind, Literal, Statement, StatementKind, UnaryOp};
use scr_diagnostics::Span;
use std::rc::Rc;

const MIN_ITERATIONS: i128 = 128;
struct Slot<'a> {
    scope: usize,
    name: &'a str,
    value: i64,
    dirty: bool,
}
enum Integer {
    Literal(i64),
    Slot(usize),
    Unary(UnaryOp, Box<Integer>, Span),
    Binary(Box<Integer>, BinaryOp, Box<Integer>, Span),
    Call(Callable, Vec<Integer>, Span),
}
struct Assignment {
    slot: usize,
    op: AssignmentOp,
    rhs: Integer,
    span: Span,
    checkpoint: Span,
}
struct Builder<'a, 'r> {
    runtime: &'r Runtime,
    slots: Vec<Slot<'a>>,
    nodes: usize,
    calls: usize,
}
impl<'a> Builder<'a, '_> {
    fn slot(&mut self, name: &'a str) -> Option<usize> {
        if name == "this" {
            return None;
        }
        if let Some(i) = self.slots.iter().position(|s| s.name == name) {
            return Some(i);
        }
        if self.slots.len() == 16 {
            return None;
        }
        let (scope, Value::Int(value)) = self.runtime.environments.lookup(name, false)? else {
            return None;
        };
        self.slots.push(Slot {
            scope,
            name,
            value: *value,
            dirty: false,
        });
        Some(self.slots.len() - 1)
    }
    fn integer(&mut self, expr: &'a Expr, depth: usize, allow_calls: bool) -> Option<Integer> {
        self.nodes += 1;
        if self.nodes > 128 || depth > 32 {
            return None;
        }
        Some(match &expr.kind {
            ExprKind::Literal(Literal::Int(v)) => Integer::Literal(*v),
            ExprKind::Variable(name) => Integer::Slot(self.slot(name)?),
            ExprKind::Grouped(inner) => return self.integer(inner, depth + 1, allow_calls),
            ExprKind::Unary(op @ (UnaryOp::Positive | UnaryOp::Negative), inner) => Integer::Unary(
                *op,
                Box::new(self.integer(inner, depth + 1, allow_calls)?),
                expr.span,
            ),
            ExprKind::Binary(
                left,
                op @ (BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Remainder),
                right,
            ) => Integer::Binary(
                Box::new(self.integer(left, depth + 1, allow_calls)?),
                *op,
                Box::new(self.integer(right, depth + 1, allow_calls)?),
                expr.span,
            ),
            ExprKind::Call(callee, args) if allow_calls => {
                if args.len() > 8 || self.calls == 8 {
                    return None;
                }
                let ExprKind::Identifier(name) = &callee.kind else {
                    return None;
                };
                let Value::Callable(callable) = self.runtime.function(name, callee.span).ok()?
                else {
                    return None;
                };
                let Function::User {
                    declaration,
                    owner,
                    receiver: None,
                    ..
                } = callable.0.as_ref()
                else {
                    return None;
                };
                if !Rc::ptr_eq(owner, &self.runtime.identity)
                    || declaration.parameters.len() != args.len()
                {
                    return None;
                }
                // Only syntax is inspected. Do not change the declaration's
                // cold-call admission or cache during speculative planning.
                let proof = Plan::compile(declaration)?;
                if !proof.integer_only() {
                    return None;
                }
                self.calls += 1;
                let mut compiled = Vec::new();
                for arg in args {
                    compiled.push(self.integer(arg, depth + 1, false)?);
                }
                Integer::Call(callable, compiled, expr.span)
            }
            _ => return None,
        })
    }
    fn assignment(&mut self, expr: &'a Expr, checkpoint: Span) -> Option<Assignment> {
        let ExprKind::Assignment(target, op, rhs) = &expr.kind else {
            return None;
        };
        let ExprKind::Variable(name) = &target.kind else {
            return None;
        };
        if *op == AssignmentOp::Concat {
            return None;
        }
        Some(Assignment {
            slot: self.slot(name)?,
            op: *op,
            rhs: self.integer(rhs, 0, true)?,
            span: target.span,
            checkpoint,
        })
    }
}
impl Integer {
    fn evaluate(
        &self,
        slots: &[Slot<'_>],
        runtime: &mut Runtime,
        output: &mut dyn crate::ScriptOutput,
    ) -> Result<i64> {
        match self {
            Self::Literal(v) => Ok(*v),
            Self::Slot(i) => Ok(slots[*i].value),
            Self::Unary(op, inner, span) => {
                let value = inner.evaluate(slots, runtime, output)?;
                runtime.operation(
                    match op {
                        UnaryOp::Positive => Ok(value),
                        UnaryOp::Negative => value
                            .checked_neg()
                            .ok_or(ops::Error::Arithmetic("integer arithmetic overflow")),
                        _ => unreachable!("verified integer unary"),
                    },
                    *span,
                )
            }
            Self::Binary(left, op, right, span) => {
                let a = left.evaluate(slots, runtime, output)?;
                let b = right.evaluate(slots, runtime, output)?;
                runtime.operation(crate::integer_loop::calculate(*op, a, b), *span)
            }
            Self::Call(callee, args, span) => {
                let mut values = if args.is_empty() {
                    Vec::new()
                } else {
                    runtime.take_argument_buffer()
                };
                for arg in args {
                    let value = arg.evaluate(slots, runtime, output)?;
                    values.try_reserve(1).map_err(|_| {
                        crate::Signal::Error(runtime.error("argument allocation failed", *span))
                    })?;
                    values.push(Value::Int(value));
                }
                let value = runtime.call_callable(callee.clone(), values, *span, output)?;
                let Value::Int(value) = value else {
                    unreachable!("proven integer-only leaf return");
                };
                Ok(value)
            }
        }
    }
}
impl Assignment {
    fn execute(
        &self,
        slots: &mut [Slot<'_>],
        runtime: &mut Runtime,
        output: &mut dyn crate::ScriptOutput,
    ) -> Result<()> {
        let old = slots[self.slot].value;
        let rhs = self.rhs.evaluate(slots, runtime, output)?;
        let value = if self.op == AssignmentOp::Assign {
            rhs
        } else {
            let op = match self.op {
                AssignmentOp::Add => BinaryOp::Add,
                AssignmentOp::Subtract => BinaryOp::Subtract,
                AssignmentOp::Multiply => BinaryOp::Multiply,
                AssignmentOp::Divide => BinaryOp::Divide,
                _ => unreachable!("verified assignment"),
            };
            runtime.operation(crate::integer_loop::calculate(op, old, rhs), self.span)?
        };
        slots[self.slot].value = value;
        slots[self.slot].dirty = true;
        Ok(())
    }
}
impl Runtime {
    #[inline(never)]
    pub(super) fn numeric_call_loop(
        &mut self,
        condition: &[Expr],
        increment: &[Expr],
        body: &Statement,
        span: Span,
        output: &mut dyn crate::ScriptOutput,
    ) -> Option<Result<()>> {
        #[cfg(test)]
        if self.disable_numeric_call_loops || self.disable_integer_loops {
            return None;
        }
        if self.options.max_value_bytes.is_some() {
            return None;
        }
        let [condition] = condition else {
            return None;
        };
        let [increment] = increment else {
            return None;
        };
        let ExprKind::Binary(left, BinaryOp::Less, right) = &condition.kind else {
            return None;
        };
        let ExprKind::Variable(counter) = &left.kind else {
            return None;
        };
        let (_, Value::Int(start)) = self.environments.lookup(counter, false)? else {
            return None;
        };
        let bound = match &right.kind {
            ExprKind::Literal(Literal::Int(v)) => *v,
            ExprKind::Variable(name) => {
                let (_, Value::Int(v)) = self.environments.lookup(name, false)? else {
                    return None;
                };
                *v
            }
            _ => return None,
        };
        if i128::from(bound) - i128::from(*start) < MIN_ITERATIONS {
            return None;
        }
        let ExprKind::Assignment(target, AssignmentOp::Add, rhs) = &increment.kind else {
            return None;
        };
        if !matches!(&target.kind,ExprKind::Variable(name) if name==counter)
            || !matches!(rhs.kind, ExprKind::Literal(Literal::Int(1)))
        {
            return None;
        }
        let StatementKind::Block(block) = &body.kind else {
            return None;
        };
        if block.statements.is_empty() || block.statements.len() > 32 {
            return None;
        }
        let mut builder = Builder {
            runtime: self,
            slots: Vec::new(),
            nodes: 0,
            calls: 0,
        };
        let counter_slot = builder.slot(counter)?;
        let bound_slot = if let ExprKind::Variable(name) = &right.kind {
            Some(builder.slot(name)?)
        } else {
            None
        };
        let left = builder.integer(left, 0, false)?;
        let right = builder.integer(right, 0, false)?;
        let mut assignments = Vec::new();
        for statement in &block.statements {
            let StatementKind::Expression(expr) = &statement.kind else {
                return None;
            };
            let assignment = builder.assignment(expr, statement.span)?;
            if assignment.slot == counter_slot || Some(assignment.slot) == bound_slot {
                return None;
            }
            assignments.push(assignment);
        }
        if builder.calls == 0 {
            return None;
        }
        let increment = builder.assignment(increment, increment.span)?;
        let mut slots = builder.slots;
        #[cfg(test)]
        {
            self.numeric_call_loops_executed += 1;
        }
        let result = (|| {
            loop {
                self.checkpoint(span)?;
                let a = left.evaluate(&slots, self, output)?;
                let b = right.evaluate(&slots, self, output)?;
                if a >= b {
                    break;
                }
                self.checkpoint(body.span)?;
                let previous = self.enter_empty_scope(self.environments.current, block.span)?;
                let result = (|| {
                    for assignment in &assignments {
                        self.checkpoint(assignment.checkpoint)?;
                        assignment.execute(&mut slots, self, output)?;
                    }
                    Ok(())
                })();
                self.environments.leave_empty(previous);
                result?;
                increment.execute(&mut slots, self, output)?;
            }
            Ok(())
        })();
        // Calls cannot read or mutate caller bindings. Publish all completed
        // writes before propagating a callee failure, limit or cancellation.
        for slot in slots.into_iter().filter(|s| s.dirty) {
            self.map_indexes.get_mut().invalidate(slot.scope, slot.name);
            *self
                .environments
                .scope_mut(slot.scope)
                .variables
                .get_mut(slot.name)
                .expect("active caller binding") = Value::Int(slot.value);
        }
        Some(result)
    }
}
#[cfg(test)]
mod tests;
