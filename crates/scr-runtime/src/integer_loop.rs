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

struct Comparison {
    left: Integer,
    op: BinaryOp,
    right: Integer,
}

enum Action {
    Assign(Assignment),
    Block {
        checkpoint: Span,
        scope: Span,
        statements: Vec<Action>,
    },
    If {
        checkpoint: Span,
        branches: Vec<(Comparison, Action)>,
        otherwise: Option<Box<Action>>,
    },
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

    fn comparison(&mut self, expr: &'a Expr, depth: usize) -> Option<Comparison> {
        self.nodes += 1;
        if self.nodes > 128 || depth > 32 {
            return None;
        }
        if let ExprKind::Grouped(inner) = &expr.kind {
            return self.comparison(inner, depth + 1);
        }
        let ExprKind::Binary(left, op, right) = &expr.kind else {
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
        Some(Comparison {
            left: self.integer(left, depth + 1)?,
            op: *op,
            right: self.integer(right, depth + 1)?,
        })
    }

    fn action(
        &mut self,
        statement: &'a Statement,
        depth: usize,
        remaining: &mut usize,
    ) -> Option<Action> {
        // Bound all branches, including untaken ones, before allocating a plan.
        if depth > 32 || *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        Some(match &statement.kind {
            StatementKind::Expression(expr) => {
                Action::Assign(self.assignment(expr, statement.span)?)
            }
            StatementKind::Block(block) => {
                let mut statements = Vec::new();
                for statement in &block.statements {
                    statements.push(self.action(statement, depth + 1, remaining)?);
                }
                Action::Block {
                    checkpoint: statement.span,
                    scope: block.span,
                    statements,
                }
            }
            StatementKind::If {
                branches,
                else_branch,
            } => {
                let mut compiled = Vec::new();
                for (condition, body) in branches {
                    compiled.push((
                        self.comparison(condition, 0)?,
                        self.action(body, depth + 1, remaining)?,
                    ));
                }
                let otherwise = match else_branch {
                    Some(body) => Some(Box::new(self.action(body, depth + 1, remaining)?)),
                    None => None,
                };
                Action::If {
                    checkpoint: statement.span,
                    branches: compiled,
                    otherwise,
                }
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

pub(super) fn calculate(op: BinaryOp, a: i64, b: i64) -> ops::Result<i64> {
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

impl Comparison {
    fn evaluate(&self, slots: &[Slot<'_>], runtime: &Runtime) -> Result<bool> {
        let a = self.left.evaluate(slots, runtime)?;
        let b = self.right.evaluate(slots, runtime)?;
        Ok(match self.op {
            BinaryOp::Less => a < b,
            BinaryOp::LessEqual => a <= b,
            BinaryOp::Greater => a > b,
            BinaryOp::GreaterEqual => a >= b,
            BinaryOp::Equal => a == b,
            BinaryOp::NotEqual => a != b,
            _ => unreachable!("verified integer comparison"),
        })
    }
}

impl Action {
    fn execute(&self, slots: &mut [Slot<'_>], runtime: &mut Runtime) -> Result<()> {
        match self {
            Self::Assign(assignment) => {
                runtime.checkpoint(assignment.checkpoint)?;
                assignment.execute(slots, runtime)
            }
            Self::Block {
                checkpoint,
                scope,
                statements,
            } => {
                runtime.checkpoint(*checkpoint)?;
                let previous = runtime.enter_empty_scope(runtime.environments.current, *scope)?;
                let result = (|| {
                    for statement in statements {
                        statement.execute(slots, runtime)?;
                    }
                    Ok(())
                })();
                runtime.environments.leave_empty(previous);
                result
            }
            Self::If {
                checkpoint,
                branches,
                otherwise,
            } => {
                runtime.checkpoint(*checkpoint)?;
                for (condition, body) in branches {
                    if condition.evaluate(slots, runtime)? {
                        return body.execute(slots, runtime);
                    }
                }
                if let Some(body) = otherwise {
                    body.execute(slots, runtime)?;
                }
                Ok(())
            }
        }
    }
}

impl Runtime {
    #[inline(never)]
    fn conditional_integer_loop(
        &mut self,
        condition: &Expr,
        increment: &Expr,
        body: &Statement,
        span: Span,
    ) -> Option<Result<()>> {
        // Called only after the ordinary integer-loop eligibility guards. Keep
        // the existing assignment-only loop's iteration path unchanged.
        let mut builder = Builder {
            runtime: self,
            slots: Vec::new(),
            nodes: 0,
        };
        let condition = builder.comparison(condition, 0)?;
        let mut remaining = 32;
        let body = builder.action(body, 0, &mut remaining)?;
        let increment = builder.assignment(increment, increment.span)?;
        let mut slots = builder.slots;
        #[cfg(test)]
        {
            self.integer_loops_executed += 1;
        }
        let result = (|| {
            loop {
                self.checkpoint(span)?;
                if !condition.evaluate(&slots, self)? {
                    break;
                }
                body.execute(&mut slots, self)?;
                increment.execute(&mut slots, self)?;
            }
            Ok(())
        })();
        // All operations are integer-only and cannot call script/host code.
        // Publish completed writes even when a branch or checkpoint failed.
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
        // Preserve the tight assignment-only path; compile supported branches
        // separately when statement-level control flow is present.
        if !block.statements.iter().all(|s| {
            matches!(
                &s.kind,
                StatementKind::Expression(Expr {
                    kind: ExprKind::Assignment(_, _, _),
                    ..
                })
            )
        }) {
            return self.conditional_integer_loop(condition, increment, body, span);
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
                let previous = self.enter_empty_scope(self.environments.current, block.span)?;
                let result = (|| {
                    for assignment in &assignments {
                        self.checkpoint(assignment.checkpoint)?;
                        assignment.execute(&mut slots, self)?;
                    }
                    Ok(())
                })();
                self.environments.leave_empty(previous);
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
    fn conditional_integer_branches_match_generic_values_and_selection() {
        for source in [
            "$i=0;$total=0;$bound=200;for($i=0;$i<$bound;$i+=1){if($i%11===0){$bound-=1;}$total+=$i;}",
            "$i=0;$total=0;for($i=0;$i<8;$i+=1){if($i<2)$total+=3;else if($i<=4){$total+=2;}else{$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<6;$i+=1){if($i>1){if($i>=4){$total+=4;}else{$total+=2;}}else{$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<6;$i+=1){if(($i===2)){$i+=1;}else if($i!==4){$total+=$i;}}",
            "$i=0;$total=0;for($i=0;$i<4;$i+=1){if($i>=0){$total+=1;}else if(1/0===0){$total/=0;}}",
            "$i=0;$total=0;for($i=0;$i<4;$i+=1){if($i<0){$total/=0;}else{{$total+=1;}}}",
            "$i=0;$total=0;$limit=0-3;for($i=0;$i<3;$i+=1){if($i>$limit){$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<0;$i+=1){if(1/0===0){$total+=1;}}",
        ] {
            assert_eq!(
                compare(source, RuntimeOptions::default()).integer_loops_executed,
                1,
                "{source}"
            );
        }
        let source = "function make(){$total=0;return function(){for($i=0;$i<4;$i+=1){if($i%2===0){$total+=2;}else{$total+=1;}}return $total;};}$f=make();$a=$f();$b=$f();";
        let runtime = compare(source, RuntimeOptions::default());
        assert_eq!(runtime.integer_loops_executed, 2);
        assert_eq!(runtime.get_global("a"), Some(&Value::Int(6)));
        assert_eq!(runtime.get_global("b"), Some(&Value::Int(12)));
    }

    #[test]
    fn conditional_integer_errors_preserve_completed_writes_spans_and_traces() {
        for source in [
            "$i=0;$total=0;for($i=0;$i<5;$i+=1){$total+=1;if($i===2){$total/=0;}}",
            "$i=0;$total=0;for($i=0;$i<5;$i+=1){$total+=1;if($i/(2-$i)>0){$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<5;$i+=1){if($i<2){$total+=1;}else if(10/($i-2)>0){$total+=2;}}",
            "$i=0;$total=9223372036854775806;for($i=0;$i<5;$i+=1){if($i>=0){$total+=1;}}",
            "$i=0;$total=0;$min=0-9223372036854775807-1;$negative=0-1;for($i=0;$i<5;$i+=1){if($i>=0){$total=$min%$negative;}}",
            "$i=0;$total=0;for($i=0;$i<5;$i/=0){if($i===0){$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<10/(2-$total);$i+=1){if($i>=0){$total+=1;}}",
            "$i=0;$total=0;$observed=0;try{for($i=0;$i<5;$i+=1){if($i>=0){$total+=1;$total/=0;}}}catch(ArithmeticError $e){$observed=$total;}",
            "function fail(){$total=0;for($i=0;$i<5;$i+=1){if($i>=0){$total+=1;$total/=0;}}}fail();",
        ] {
            assert_eq!(
                compare(source, RuntimeOptions::default()).integer_loops_executed,
                1,
                "{source}"
            );
        }
    }

    #[test]
    fn conditional_integer_checkpoints_scope_ceilings_and_recovery_match() {
        for source in [
            "$i=0;$total=0;for($i=0;$i<5;$i+=1){if($i%2===0){$total+=1;}else{$total+=2;}}",
            "$i=0;$total=0;for($i=0;$i<5;$i+=1){$total+=1;if($i<2){if($i===0){{$total+=2;}}}else if($i===2)$total+=3;else{$total-=1;}}",
            "$i=0;$total=0;for($i=0;$i<4;$i+=1){if($i<0){{{$total+=1;}}}else $total+=1;}",
        ] {
            for limit in 0..140 {
                compare(
                    source,
                    RuntimeOptions {
                        instruction_limit: Some(limit),
                        ..RuntimeOptions::default()
                    },
                );
            }
            for limit in 1..=8 {
                compare(
                    source,
                    RuntimeOptions {
                        max_scopes: Some(limit),
                        ..RuntimeOptions::default()
                    },
                );
            }
            assert_eq!(
                compare(source, RuntimeOptions::default()).integer_loops_executed,
                1
            );
        }
        let source = "$i=0;$total=0;for($i=0;$i<5;$i+=1){if($i>=0){$total+=1;}}";
        let mut fast = Runtime::with_options(RuntimeOptions {
            instruction_limit: Some(15),
            ..RuntimeOptions::default()
        });
        let mut slow = Runtime::with_options(fast.options.clone());
        slow.disable_integer_loops = true;
        assert!(run(&mut fast, source, &mut Vec::new()).is_err());
        assert!(run(&mut slow, source, &mut Vec::new()).is_err());
        fast.options.instruction_limit = None;
        slow.options.instruction_limit = None;
        for runtime in [&mut fast, &mut slow] {
            run(
                runtime,
                "$after=$total+$i;{ $temporary=9; }",
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(runtime.environments.live, 1);
        }
        assert_eq!(fast.get_global("after"), slow.get_global("after"));
    }

    #[test]
    fn conditional_integer_unsupported_effects_and_value_budgets_fall_back() {
        for source in [
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0){break;}$total+=1;}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0){continue;}$total+=1;}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0){$new=1;}$total+=1;}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i<0){$new=1;}$total+=1;}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0){$total+=int($i);}}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i<2.0){$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if(true){$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0||$i===1){$total+=1;}}",
            "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0){$i='changed';}}",
            "$i=0;$total=[0];for($i=0;$i<3;$i+=1){if($i>=0){$total[0]+=$i;}}",
        ] {
            assert_eq!(
                compare(source, RuntimeOptions::default()).integer_loops_executed,
                0,
                "{source}"
            );
        }
        for limit in [
            0,
            std::mem::size_of::<Value>() - 1,
            std::mem::size_of::<Value>(),
            1024,
        ] {
            assert_eq!(
                compare(
                    "$i=0;$total=0;for($i=0;$i<3;$i+=1){if($i===0){$total+=1;}}",
                    RuntimeOptions {
                        max_value_bytes: Some(limit),
                        ..RuntimeOptions::default()
                    }
                )
                .integer_loops_executed,
                0
            );
        }
    }

    #[test]
    fn conditional_integer_plan_limits_preserve_generic_results_at_boundaries() {
        // Root block + if + assignment + 29 trailing assignments = 32 actions.
        for (extra, selected) in [(29, 1), (30, 0)] {
            let source = format!(
                "$i=0;$total=0;for($i=0;$i<1;$i+=1){{if($i===0)$total+=1;{}}}",
                "$total+=0;".repeat(extra)
            );
            assert_eq!(
                compare(&source, RuntimeOptions::default()).integer_loops_executed,
                selected
            );
        }
        for (variables, selected) in [(14, 1), (15, 0)] {
            let prefix: String = (0..variables).map(|i| format!("$v{i}=1;")).collect();
            let body: String = (0..variables).map(|i| format!("$total+=$v{i};")).collect();
            let source = format!(
                "$i=0;$total=0;{prefix}for($i=0;$i<1;$i+=1){{if($i===0){{$total+=1;}}{body}}}"
            );
            assert_eq!(
                compare(&source, RuntimeOptions::default()).integer_loops_executed,
                selected
            );
        }
        fn balanced(leaves: usize) -> String {
            if leaves == 1 {
                return "1".into();
            }
            format!(
                "({}+{})",
                balanced(leaves / 2),
                balanced(leaves - leaves / 2)
            )
        }
        // Comparison nodes plus the grouped balanced expression reach 128.
        for (leaves, selected) in [(41, 1), (42, 0)] {
            let source = format!(
                "$i=0;$total=0;for($i=0;$i<1;$i+=1){{if($i===0)$total={};}}",
                balanced(leaves)
            );
            let runtime = compare(&source, RuntimeOptions::default());
            assert_eq!(runtime.integer_loops_executed, selected);
            assert_eq!(
                runtime.get_global("total"),
                Some(&Value::Int(leaves as i64))
            );
        }
        for (groups, selected) in [(31, 1), (32, 0)] {
            let source = format!(
                "$i=0;$total=0;for($i=0;$i<1;$i+=1){{if({}$i===0{})$total+=1;}}",
                "(".repeat(groups),
                ")".repeat(groups)
            );
            assert_eq!(
                compare(&source, RuntimeOptions::default()).integer_loops_executed,
                selected
            );
        }
    }

    #[test]
    fn conditional_integer_cancellation_unwinds_and_publishes_prior_writes() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let source =
            "$i=0;$total=0;for($i=0;$i>=0;$i+=1){if($i%2===0){$total+=1;}else{$total+=1;}}";
        let program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        let worker = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        let result = runtime.execute(&program, &mut Vec::new());
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
