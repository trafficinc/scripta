//! Bounded specialization of side-effect-free integer expressions in for loops.
//! No calls or binding creation are allowed: slots remain valid until writeback.
use crate::{Result, Runtime, Value, ops};
use scr_ast::{AssignmentOp, BinaryOp, Expr, ExprKind, Literal, Statement, StatementKind};
use scr_diagnostics::Span;

struct Slot<'a> {
    scope: usize,
    name: &'a str,
    value: i64,
    dirty: bool,
}

enum Integer {
    Literal(i64),
    Slot(usize),
    Binary(Box<Integer>, BinaryOp, Box<Integer>, Span),
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
}

impl<'a> Builder<'a, '_> {
    fn slot(&mut self, name: &'a str) -> Option<usize> {
        if name == "this" {
            return None;
        }
        if let Some(index) = self.slots.iter().position(|slot| slot.name == name) {
            return Some(index);
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

    fn integer(&mut self, expr: &'a Expr, depth: usize) -> Option<Integer> {
        self.nodes += 1;
        if self.nodes > 128 || depth > 32 {
            return None;
        }
        Some(match &expr.kind {
            ExprKind::Literal(Literal::Int(value)) => Integer::Literal(*value),
            ExprKind::Variable(name) => Integer::Slot(self.slot(name)?),
            ExprKind::Grouped(inner) => return self.integer(inner, depth + 1),
            ExprKind::Binary(left, op, right) if arithmetic(*op) => Integer::Binary(
                Box::new(self.integer(left, depth + 1)?),
                *op,
                Box::new(self.integer(right, depth + 1)?),
                expr.span,
            ),
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
            rhs: self.integer(rhs, 0)?,
            span: target.span,
            checkpoint,
        })
    }
}

fn arithmetic(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Remainder
    )
}

fn calculate(op: BinaryOp, a: i64, b: i64) -> ops::Result<i64> {
    use BinaryOp as B;
    if matches!(op, B::Divide | B::Remainder) && b == 0 {
        return Err(ops::Error::Arithmetic("division by zero"));
    }
    match op {
        B::Add => a.checked_add(b),
        B::Subtract => a.checked_sub(b),
        B::Multiply => a.checked_mul(b),
        B::Divide => a.checked_div(b),
        B::Remainder => a.checked_rem(b),
        _ => unreachable!("verified integer operator"),
    }
    .ok_or(ops::Error::Arithmetic("integer arithmetic overflow"))
}

impl Integer {
    fn evaluate(&self, slots: &[Slot<'_>], runtime: &Runtime) -> Result<i64> {
        match self {
            Self::Literal(value) => Ok(*value),
            Self::Slot(index) => Ok(slots[*index].value),
            Self::Binary(left, op, right, span) => {
                let left = left.evaluate(slots, runtime)?;
                let right = right.evaluate(slots, runtime)?;
                runtime.operation(calculate(*op, left, right), *span)
            }
        }
    }
}

impl Assignment {
    fn execute(&self, slots: &mut [Slot<'_>], runtime: &Runtime) -> Result<()> {
        let old = slots[self.slot].value;
        let rhs = self.rhs.evaluate(slots, runtime)?;
        let value = if self.op == AssignmentOp::Assign {
            rhs
        } else {
            let op = match self.op {
                AssignmentOp::Add => BinaryOp::Add,
                AssignmentOp::Subtract => BinaryOp::Subtract,
                AssignmentOp::Multiply => BinaryOp::Multiply,
                AssignmentOp::Divide => BinaryOp::Divide,
                _ => unreachable!("verified assignment operator"),
            };
            runtime.operation(calculate(op, old, rhs), self.span)?
        };
        slots[self.slot].value = value;
        slots[self.slot].dirty = true;
        Ok(())
    }
}

impl Runtime {
    // Keep planning out of the generic evaluator's instruction cache. Unsupported
    // loops pay one bounded eligibility check per entry, never per iteration.
    #[inline(never)]
    pub(super) fn integer_loop(
        &mut self,
        condition: &[Expr],
        increment: &[Expr],
        body: &Statement,
        span: Span,
    ) -> Option<Result<()>> {
        #[cfg(test)]
        if self.disable_integer_loops {
            return None;
        }
        // The generic evaluator checks literals and writes against this ceiling.
        // Retain that path whenever a host configures a value-byte limit.
        if self.options.max_value_bytes.is_some() {
            return None;
        }
        let [condition] = condition else { return None };
        let [increment] = increment else { return None };
        let ExprKind::Binary(left, op, right) = &condition.kind else {
            return None;
        };
        if !matches!(
            op,
            BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Equal
                | BinaryOp::NotEqual
        ) {
            return None;
        }
        let StatementKind::Block(block) = &body.kind else {
            return None;
        };
        if block.statements.is_empty() || block.statements.len() > 32 {
            return None;
        }
        // Reject statement-level control flow before resolving any bindings.
        if !block.statements.iter().all(|s| {
            matches!(
                &s.kind,
                StatementKind::Expression(Expr {
                    kind: ExprKind::Assignment(_, _, _),
                    ..
                })
            )
        }) {
            return None;
        }
        let mut builder = Builder {
            runtime: self,
            slots: Vec::new(),
            nodes: 0,
        };
        let left = builder.integer(left, 0)?;
        let right = builder.integer(right, 0)?;
        let mut assignments = Vec::new();
        for statement in &block.statements {
            let StatementKind::Expression(expr) = &statement.kind else {
                unreachable!()
            };
            assignments.push(builder.assignment(expr, statement.span)?);
        }
        let increment = builder.assignment(increment, increment.span)?;
        let mut slots = builder.slots;
        #[cfg(test)]
        {
            self.integer_loops_executed += 1;
        }
        let result = (|| {
            loop {
                self.checkpoint(span)?;
                let a = left.evaluate(&slots, self)?;
                let b = right.evaluate(&slots, self)?;
                let keep_going = match op {
                    BinaryOp::Less => a < b,
                    BinaryOp::LessEqual => a <= b,
                    BinaryOp::Greater => a > b,
                    BinaryOp::GreaterEqual => a >= b,
                    BinaryOp::Equal => a == b,
                    BinaryOp::NotEqual => a != b,
                    _ => unreachable!(),
                };
                if !keep_going {
                    break;
                }
                self.checkpoint(body.span)?;
                let previous = self.enter_scope(self.environments.current, block.span)?;
                let result = (|| {
                    for assignment in &assignments {
                        self.checkpoint(assignment.checkpoint)?;
                        assignment.execute(&mut slots, self)?;
                    }
                    Ok(())
                })();
                self.environments.leave(previous);
                result?;
                increment.execute(&mut slots, self)?;
            }
            Ok(())
        })();
        // There are no callbacks, collection writes, or declarations in this path.
        // Even on failure, publish completed assignments before a catch/host runs.
        for slot in slots.into_iter().filter(|slot| slot.dirty) {
            self.map_indexes.get_mut().invalidate(slot.scope, slot.name);
            *self
                .environments
                .scope_mut(slot.scope)
                .variables
                .get_mut(slot.name)
                .expect("integer loop binding remains live") = Value::Int(slot.value);
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeError, RuntimeOptions};

    fn run(
        runtime: &mut Runtime,
        source: &str,
        output: &mut Vec<u8>,
    ) -> std::result::Result<u8, RuntimeError> {
        let tokens = scr_lexer::lex(source).unwrap();
        runtime.execute(&scr_parser::parse(&tokens).unwrap(), output)
    }

    fn compare(source: &str, options: RuntimeOptions) -> Runtime {
        let mut fast = Runtime::with_options(options.clone());
        let mut slow = Runtime::with_options(options);
        slow.disable_integer_loops = true;
        let mut fast_output = Vec::new();
        let mut slow_output = Vec::new();
        let fast_result = run(&mut fast, source, &mut fast_output);
        let slow_result = run(&mut slow, source, &mut slow_output);
        assert_eq!(fast_result, slow_result, "{source}");
        assert_eq!(fast_output, slow_output, "{source}");
        assert_eq!(fast.steps, slow.steps, "{source}");
        assert_eq!(fast.environments.live, slow.environments.live);
        let fast_globals = &fast.environments.scope(0).variables;
        let slow_globals = &slow.environments.scope(0).variables;
        assert_eq!(fast_globals.len(), slow_globals.len());
        for (name, value) in fast_globals {
            if matches!(value, Value::Callable(_)) {
                assert!(matches!(slow_globals.get(name), Some(Value::Callable(_))));
            } else {
                assert_eq!(Some(value), slow_globals.get(name), "{source}: {name}");
            }
        }
        fast
    }

    #[test]
    fn arithmetic_aliasing_dynamic_bounds_and_captured_bindings() {
        for source in [
            "$total=0; for($i=0;$i<1000;$i+=1){$total+=($i%97)*3;}",
            "$i=0;$total=1;$bound=20;for($i=0;$i<$bound;$i+=1){$total=$total*2;$bound-=1;}",
            "$i=0;$total=0;for($i=0;$i<10;$i+=1){$i+=1;$total+=$i;$total=$total+$total/2;}",
            "$i=0;$total=0;for($i=10;$i>=0;$i-=1){$total-=$i;}",
            "$i=0;$total=0;for($i=0;$i!==10;$i+=1){$total+=$i;}",
            "$i=0;$total=0;for($i=0;$i===0;$i+=1){$total+=$i;}",
            "$i=0;$total=0;for($i=0;$i>10;$i+=1){$total+=$i;}",
            "$i=0;$total=0;for($i=0;$i<=10;$i+=1){$total+=$i;}",
            "function g(){$total=0;return function(){for($i=0;$i<10;$i+=1){$total+=$i;}return $total;};}$f=g();$observed=$f();",
            "function f(){$total=0;for($i=0;$i<10;$i+=1){$total+=$i;}return $total;}$observed=f();",
        ] {
            let runtime = compare(source, RuntimeOptions::default());
            assert_eq!(runtime.integer_loops_executed, 1, "{source}");
        }
    }

    #[test]
    fn failed_operations_preserve_prior_writes_spans_traces_and_catches() {
        for source in [
            "$i=0;$total=0;for($i=0;$i<10;$i+=1){$total+=1;$total+=10/(3-$i);}",
            "$i=0;$total=9223372036854775806;for($i=0;$i<10;$i+=1){$total+=1;}",
            "$i=0;$total=0;for($i=0;$i<10;$i+=1){$total=9223372036854775807*2;}",
            "$i=0;$total=0;for($i=0;$i<10;$i+=1){$total=10%($i-$i);}",
            "$i=0;$total=0;for($i=0;$i<10/(2-$total);$i+=1){$total+=1;}",
            "$i=0;$total=0;for($i=0;$i<10;$i/=0){$total+=1;}",
            "$i=0;$min=-9223372036854775807-1;$negative=-1;$total=0;for($i=0;$i<10;$i+=1){$total=$min/$negative;}",
            "$i=0;$min=-9223372036854775807-1;$negative=-1;$total=0;for($i=0;$i<10;$i+=1){$total=$min%$negative;}",
            "$i=0;$total=0;try{for($i=0;$i<10;$i+=1){$total+=1;$total/=0;}}catch(ArithmeticError $e){$observed=$total;}",
            "function f(){$total=0;for($i=0;$i<10;$i+=1){$total/=0;}}f();",
        ] {
            let runtime = compare(source, RuntimeOptions::default());
            assert_eq!(runtime.integer_loops_executed, 1, "{source}");
        }
    }

    #[test]
    fn exact_checkpoint_and_scope_limits_match_generic_execution() {
        let source = "$i=0;$total=0;for($i=0;$i<4;$i+=1){$total+=1;$total+=$i;}";
        for limit in 0..25 {
            compare(
                source,
                RuntimeOptions {
                    instruction_limit: Some(limit),
                    ..RuntimeOptions::default()
                },
            );
        }
        for limit in 1..=4 {
            compare(
                source,
                RuntimeOptions {
                    max_scopes: Some(limit),
                    ..RuntimeOptions::default()
                },
            );
        }
    }

    #[test]
    fn unsupported_loops_and_value_limits_use_generic_execution() {
        for source in [
            "$total=0;for($i=0;$i<4;$i+=1){$total+=1.0;}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=1;}",
            "for($i=0;$i<4;$i+=1){$new=$i;}",
            "$total=0;for($i=0;$i<4;$i+=1){$total+=1;break;}",
            "$total=0;for($i=0;$i<4;$i+=1){$total+=1;continue;}",
            "$total=0;for($i=0;$i<4;$i+=1){$total+=1;echo str($total);}",
            "$total=0;for($i=0;$i<4;$i+=1){$total+=int($i);}",
            "$total=[0];for($i=0;$i<4;$i+=1){$total[0]+=$i;}",
            "$total=0;for($i=0;$i<4;$i+=1){$total+=($i=4);}",
        ] {
            assert_eq!(
                compare(source, RuntimeOptions::default()).integer_loops_executed,
                0,
                "{source}"
            );
        }
        for bytes in [
            0,
            std::mem::size_of::<Value>() - 1,
            std::mem::size_of::<Value>(),
            1024,
        ] {
            assert_eq!(
                compare(
                    "$total=0;for($i=0;$i<4;$i+=1){$total+=$i;}",
                    RuntimeOptions {
                        max_value_bytes: Some(bytes),
                        ..RuntimeOptions::default()
                    }
                )
                .integer_loops_executed,
                0
            );
        }
    }

    #[test]
    fn concurrent_cancellation_publishes_completed_assignments() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let flag = Arc::new(AtomicBool::new(false));
        let other = flag.clone();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            other.store(true, Ordering::Relaxed);
        });
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag),
            ..RuntimeOptions::default()
        });
        let result = run(
            &mut runtime,
            "$i=0;$total=0;for($i=0;$i>=0;$i+=1){$total+=1;}",
            &mut Vec::new(),
        );
        worker.join().unwrap();
        assert!(result.unwrap_err().message.contains("interrupted"));
        assert_eq!(runtime.integer_loops_executed, 1);
        let Some(Value::Int(total)) = runtime.get_global("total") else {
            panic!()
        };
        let Some(Value::Int(i)) = runtime.get_global("i") else {
            panic!()
        };
        assert!(*total == *i || *total == *i + 1);
        assert_eq!(runtime.environments.live, 1);
    }
}
