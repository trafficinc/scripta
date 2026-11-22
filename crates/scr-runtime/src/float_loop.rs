//! Bounded numeric plans with stable integer/float slots and a trusted converter.
use crate::{Callable, Function, NativeError, Result, Runtime, StackFrame, Value, ops};
use scr_ast::{AssignmentOp, BinaryOp, Expr, ExprKind, Literal, Statement, StatementKind, UnaryOp};
use scr_diagnostics::Span;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Int,
    Float,
}

fn kind(value: &Value) -> Option<Kind> {
    match value {
        Value::Int(_) => Some(Kind::Int),
        Value::Float(_) => Some(Kind::Float),
        _ => None,
    }
}

struct Slot<'a> {
    scope: usize,
    name: &'a str,
    value: Value,
    dirty: bool,
}

enum Numeric {
    Literal(Value),
    Slot(usize),
    Unary(UnaryOp, Box<Numeric>, Span),
    Binary(Box<Numeric>, BinaryOp, Box<Numeric>, Span),
    Convert(Callable, Box<Numeric>, Span),
}

struct Assignment {
    slot: usize,
    op: AssignmentOp,
    rhs: Numeric,
    span: Span,
    checkpoint: Span,
}

struct Builder<'a, 'r> {
    runtime: &'r Runtime,
    slots: Vec<Slot<'a>>,
    nodes: usize,
    has_float: bool,
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
        let (scope, value) = self.runtime.environments.lookup(name, false)?;
        let ty = kind(value)?;
        self.has_float |= ty == Kind::Float;
        self.slots.push(Slot {
            scope,
            name,
            value: value.clone(),
            dirty: false,
        });
        Some(self.slots.len() - 1)
    }

    fn numeric(&mut self, expr: &'a Expr, depth: usize) -> Option<(Numeric, Kind)> {
        self.nodes += 1;
        if self.nodes > 128 || depth > 32 {
            return None;
        }
        Some(match &expr.kind {
            ExprKind::Literal(Literal::Int(value)) => {
                (Numeric::Literal(Value::Int(*value)), Kind::Int)
            }
            ExprKind::Literal(Literal::Float(value)) => {
                self.has_float = true;
                (Numeric::Literal(Value::Float(*value)), Kind::Float)
            }
            ExprKind::Variable(name) => {
                let index = self.slot(name)?;
                (Numeric::Slot(index), kind(&self.slots[index].value)?)
            }
            ExprKind::Grouped(inner) => return self.numeric(inner, depth + 1),
            ExprKind::Unary(op @ (UnaryOp::Positive | UnaryOp::Negative), inner) => {
                let (inner, ty) = self.numeric(inner, depth + 1)?;
                (Numeric::Unary(*op, Box::new(inner), expr.span), ty)
            }
            ExprKind::Binary(left, op, right) if arithmetic(*op) => {
                let (left, ty) = self.numeric(left, depth + 1)?;
                let (right, rhs) = self.numeric(right, depth + 1)?;
                if ty != rhs {
                    return None;
                }
                (
                    Numeric::Binary(Box::new(left), *op, Box::new(right), expr.span),
                    ty,
                )
            }
            ExprKind::Call(callee, args) => {
                let ExprKind::Identifier(name) = &callee.kind else {
                    return None;
                };
                let Value::Callable(callable) = self.runtime.function(name, callee.span).ok()?
                else {
                    return None;
                };
                let Function::Native { function, .. } = callable.0.as_ref() else {
                    return None;
                };
                // Only a runtime-owned converter can carry this private identity.
                // A same-named host callback or lexical function is never folded.
                if !function.is_float_conversion() {
                    return None;
                }
                let [arg] = args.as_slice() else {
                    return None;
                };
                let (arg, ty) = self.numeric(arg, depth + 1)?;
                if ty != Kind::Int {
                    return None;
                }
                self.has_float = true;
                (
                    Numeric::Convert(callable, Box::new(arg), expr.span),
                    Kind::Float,
                )
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
        let slot = self.slot(name)?;
        let (rhs, ty) = self.numeric(rhs, 0)?;
        // Changing slot types would invalidate later expressions in the plan.
        if kind(&self.slots[slot].value)? != ty {
            return None;
        }
        Some(Assignment {
            slot,
            op: *op,
            rhs,
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

impl Numeric {
    fn evaluate(&self, slots: &[Slot<'_>], runtime: &mut Runtime) -> Result<Value> {
        match self {
            Self::Literal(value) => Ok(value.clone()),
            Self::Slot(index) => Ok(slots[*index].value.clone()),
            Self::Unary(op, inner, span) => {
                let value = inner.evaluate(slots, runtime)?;
                runtime.operation(ops::unary(*op, value), *span)
            }
            Self::Binary(left, op, right, span) => {
                let left = left.evaluate(slots, runtime)?;
                let right = right.evaluate(slots, runtime)?;
                runtime.operation(ops::binary(*op, left, right), *span)
            }
            Self::Convert(callable, inner, span) => {
                let value = inner.evaluate(slots, runtime)?;
                let Value::Int(number) = value else {
                    unreachable!("verified integer conversion")
                };
                // Argument evaluation precedes the call-depth check. This
                // private converter always accepts one int and returns a finite
                // float, with no callbacks or instruction checkpoints.
                if runtime.stack.len().saturating_sub(1)
                    >= runtime
                        .options
                        .recursion_limit
                        .min(crate::MAX_RECURSION_LIMIT)
                {
                    return runtime.fail("maximum call depth exceeded", *span);
                }
                let interrupted = || {
                    runtime
                        .options
                        .interrupt_flag
                        .as_ref()
                        .is_some_and(|flag| flag.load(Ordering::Relaxed))
                };
                let cancelled = interrupted();
                let result = Value::Float(number as f64);
                if cancelled || interrupted() {
                    // Match native-call diagnostics without re-reading a flag
                    // already observed set (a host may subsequently reset it).
                    runtime.stack.push(
                        StackFrame {
                            function: callable.name().into(),
                            source: runtime.source.clone(),
                            span: *span,
                        }
                        .into(),
                    );
                    let error = runtime.native_error(
                        NativeError::new("Interrupted", "execution interrupted"),
                        *span,
                    );
                    runtime.stack.pop();
                    return Err(error);
                }
                Ok(result)
            }
        }
    }
}

impl Assignment {
    fn execute(&self, slots: &mut [Slot<'_>], runtime: &mut Runtime) -> Result<()> {
        let old = slots[self.slot].value.clone();
        let rhs = self.rhs.evaluate(slots, runtime)?;
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
            runtime.operation(ops::binary(op, old, rhs), self.span)?
        };
        slots[self.slot].value = value;
        slots[self.slot].dirty = true;
        Ok(())
    }
}

impl Runtime {
    #[inline(never)]
    pub(super) fn float_loop(
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
        if self.options.max_value_bytes.is_some() {
            return None;
        }
        let [condition] = condition else {
            return None;
        };
        let [increment] = increment else {
            return None;
        };
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
        let mut builder = Builder {
            runtime: self,
            slots: Vec::new(),
            nodes: 0,
            has_float: false,
        };
        let (left, left_kind) = builder.numeric(left, 0)?;
        let (right, right_kind) = builder.numeric(right, 0)?;
        if left_kind != right_kind {
            return None;
        }
        let mut assignments = Vec::new();
        for statement in &block.statements {
            let StatementKind::Expression(expr) = &statement.kind else {
                return None;
            };
            assignments.push(builder.assignment(expr, statement.span)?);
        }
        let increment = builder.assignment(increment, increment.span)?;
        if !builder.has_float {
            return None;
        }
        let mut slots = builder.slots;
        #[cfg(test)]
        {
            self.float_loops_executed += 1;
        }
        let result = (|| {
            loop {
                self.checkpoint(span)?;
                let a = left.evaluate(&slots, self)?;
                let b = right.evaluate(&slots, self)?;
                let keep_going = self.operation(ops::binary(*op, a, b), condition.span)?;
                if !self.operation(ops::boolean(keep_going), span)? {
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
        for slot in slots.into_iter().filter(|slot| slot.dirty) {
            self.map_indexes.get_mut().invalidate(slot.scope, slot.name);
            *self
                .environments
                .scope_mut(slot.scope)
                .variables
                .get_mut(slot.name)
                .expect("numeric binding remains live") = slot.value;
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NativeFunction, NativeSignature, NativeType, RuntimeError, RuntimeOptions};

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
        for runtime in [&mut fast, &mut slow] {
            runtime
                .register("float", NativeFunction::float_conversion())
                .unwrap();
        }
        slow.disable_integer_loops = true;
        let mut a = Vec::new();
        let mut b = Vec::new();
        assert_eq!(
            run(&mut fast, source, &mut a),
            run(&mut slow, source, &mut b),
            "{source}"
        );
        assert_eq!(a, b, "{source}");
        assert_eq!(fast.steps, slow.steps, "{source}");
        assert_eq!(fast.environments.live, slow.environments.live);
        let globals = &fast.environments.scope(0).variables;
        let others = &slow.environments.scope(0).variables;
        assert_eq!(globals.len(), others.len());
        for (name, value) in globals {
            if let Value::Float(number) = value {
                let Some(Value::Float(other)) = others.get(name) else {
                    panic!("{name}")
                };
                assert_eq!(number.to_bits(), other.to_bits(), "{source}: {name}");
            } else if !matches!(value, Value::Callable(_)) {
                assert_eq!(Some(value), others.get(name), "{source}: {name}");
            }
        }
        fast
    }

    #[test]
    fn arithmetic_conversions_bounds_aliasing_and_captures() {
        for source in [
            "$total=0.0;for($i=0;$i<20000;$i+=1){$total+=float($i%97)*0.5;}",
            "$total=0.0;for($i=0;$i<1;$i+=1){$total=-0.0;$total*=0.5;}",
            "$total=0.0;for($i=0.0;$i<4.0;$i+=0.5){$total+=$i;}",
            "$total=0.0;for($i=4.0;$i>=0.0;$i-=0.5){$total-=$i;}",
            "$total=0.0;$bound=5;for($i=0;$i<$bound;$i+=1){$total+=float($i);$bound-=1;}",
            "$total=1.0;for($i=0;$i<3;$i+=1){$total=$total+$total;$total=$total/2.0;$total*=1.5;}",
            "$total=0.0;for($i=0;$i<3;$i+=1){$total=-(-$total)+(+float(-$i));}",
            "$total=0.0;for($i=0;$i<3;$i+=1){$total+=float(9223372036854775807);}",
            "$total=0.0;for($i=0;$i<0;$i+=1){$total/=0.0;}",
            "$total=0.0;for($i=0.0;$i!==3.0;$i+=1.0){$total+=$i%2.0;}",
            "function make(){$total=0.0;return function(){for($i=0;$i<4;$i+=1){$total+=float($i);}return $total;};}$f=make();$a=$f();$b=$f();",
        ] {
            assert!(
                compare(source, RuntimeOptions::default()).float_loops_executed > 0,
                "{source}"
            );
        }
    }

    #[test]
    fn failures_preserve_assignments_error_spans_and_traces() {
        for source in [
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=1.0;$total/=float(2-$i);}",
            "$total=1.0;for($i=0;$i<4;$i+=1){$total=1e308*1e308;}",
            "$total=1.0;for($i=0;$i<4;$i+=1){$total=1.0%-0.0;}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float(9223372036854775807+$i);}",
            "$total=0.0;for($i=0.0;$i<1.0/0.0;$i+=1.0){$total+=1.0;}",
            "$total=0.0;for($i=0.0;$i<4.0;$i/=0.0){$total+=1.0;}",
            "$total=0.0;try{for($i=0;$i<4;$i+=1){$total+=1.0;$total/=float(2-$i);}}catch(ArithmeticError $e){$seen=$total;}",
            "function f(){$total=1.0;for($i=0;$i<4;$i+=1){$total/=0.0;}}f();",
        ] {
            assert_eq!(
                compare(source, RuntimeOptions::default()).float_loops_executed,
                1,
                "{source}"
            );
        }
    }

    #[test]
    fn exact_instruction_scope_byte_and_call_limits() {
        let source = "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float($i);$total+=0.5;}";
        for limit in 0..35 {
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
        for bytes in [
            0,
            std::mem::size_of::<Value>() - 1,
            std::mem::size_of::<Value>(),
            std::mem::size_of::<Value>() + 1,
            4096,
        ] {
            assert_eq!(
                compare(
                    source,
                    RuntimeOptions {
                        max_value_bytes: Some(bytes),
                        ..RuntimeOptions::default()
                    }
                )
                .float_loops_executed,
                0
            );
        }
        for limit in 0..=3 {
            for source in [
                source,
                "function f(){$total=0.0;for($i=0;$i<4;$i+=1){$total+=float($i);}}f();",
            ] {
                compare(
                    source,
                    RuntimeOptions {
                        recursion_limit: limit,
                        ..RuntimeOptions::default()
                    },
                );
            }
        }
    }

    #[test]
    fn unsupported_types_calls_and_control_flow_fall_back() {
        for source in [
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=1;}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total=1;$total+=float($i);}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float(true);}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float(1.0);}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float(...[$i]);}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float();}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float($i,1);}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$new=float($i);}",
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float($i);break;}",
            "$total=0.0;for($i=0;$i<4;$i+=1){if($i<2){$total+=float($i);}}",
            "$total=0.0;$convert=float;for($i=0;$i<4;$i+=1){$total+=$convert($i);}",
            "function f(){function float($x){return 42.0;}$total=0.0;for($i=0;$i<4;$i+=1){$total+=float($i);}return $total;}$observed=f();",
        ] {
            assert_eq!(
                compare(source, RuntimeOptions::default()).float_loops_executed,
                0,
                "{source}"
            );
        }
        let mut runtime = Runtime::new();
        runtime
            .register(
                "float",
                NativeFunction::new(
                    NativeSignature::new(vec![NativeType::Any], NativeType::Float),
                    |_, _| Ok(Value::Float(42.0)),
                ),
            )
            .unwrap();
        run(
            &mut runtime,
            "$total=0.0;for($i=0;$i<4;$i+=1){$total+=float($i);}",
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(runtime.float_loops_executed, 0);
        assert_eq!(runtime.get_global("total"), Some(&Value::Float(168.0)));
    }

    #[test]
    fn planning_boundaries_fall_back_without_partial_effects() {
        for count in [32, 33] {
            let source = format!(
                "$total=0.0;for($i=0;$i<2;$i+=1){{{}}}",
                "$total+=0.5;".repeat(count)
            );
            assert_eq!(
                compare(&source, RuntimeOptions::default()).float_loops_executed,
                usize::from(count == 32)
            );
        }
        for count in [14, 15] {
            let declarations = (0..count)
                .map(|i| format!("$x{i}=0.0;"))
                .collect::<String>();
            let body = (0..count)
                .map(|i| format!("$x{i}+=1.0;"))
                .collect::<String>();
            // i plus total plus x slots: 16 accepted, 17 rejected.
            let source =
                format!("{declarations}$total=0.0;for($i=0;$i<2;$i+=1){{$total+=1.0;{body}}}");
            assert_eq!(
                compare(&source, RuntimeOptions::default()).float_loops_executed,
                usize::from(count == 14)
            );
        }
        for depth in [32, 33] {
            let source = format!(
                "$total=0.0;for($i=0;$i<2;$i+=1){{$total+={}0.5{};}}",
                "(".repeat(depth),
                ")".repeat(depth)
            );
            assert_eq!(
                compare(&source, RuntimeOptions::default()).float_loops_executed,
                usize::from(depth == 32)
            );
        }
        fn balanced(n: usize) -> String {
            if n == 1 {
                "0.5".into()
            } else {
                format!("({}+{})", balanced(n / 2), balanced(n - n / 2))
            }
        }
        for extra in [1, 2] {
            // 127 nodes across condition, RHS and increment, plus grouping.
            let source = format!(
                "$total=0.0;for($i=0;$i<2;$i+=1){{$total+={}{}{};}}",
                "(".repeat(extra),
                balanced(42),
                ")".repeat(extra)
            );
            assert_eq!(
                compare(&source, RuntimeOptions::default()).float_loops_executed,
                usize::from(extra == 1)
            );
        }
    }

    #[test]
    fn conversion_cancellation_and_depth_errors_match_native_diagnostics() {
        use std::sync::{Arc, atomic::AtomicBool};
        for recursion_limit in [0, 64] {
            let flag = Arc::new(AtomicBool::new(true));
            let options = RuntimeOptions {
                recursion_limit,
                interrupt_flag: Some(flag),
                ..RuntimeOptions::default()
            };
            let mut fast = Runtime::with_options(options.clone());
            let mut slow = Runtime::with_options(options);
            fast.register("float", NativeFunction::float_conversion())
                .unwrap();
            let span = Span::new(12, 20);
            let Value::Callable(callable) = fast.function("float", span).ok().unwrap() else {
                panic!()
            };
            let plan = Numeric::Convert(
                callable.clone(),
                Box::new(Numeric::Literal(Value::Int(7))),
                span,
            );
            let a = plan.evaluate(&[], &mut fast).unwrap_err();
            let b = slow
                .call_callable(callable, vec![Value::Int(7)], span, &mut Vec::new())
                .unwrap_err();
            match (a, b) {
                (crate::Signal::Interrupted(a), crate::Signal::Interrupted(b))
                | (crate::Signal::Error(a), crate::Signal::Error(b)) => assert_eq!(a, b),
                _ => panic!("different failure kinds"),
            }
            assert!(fast.stack.is_empty());
            assert!(slow.stack.is_empty());
        }
    }

    #[test]
    fn registered_converter_keeps_the_full_conversion_contract() {
        for (expression, expected) in [
            ("float(0)", 0.0),
            ("float(-9223372036854775807-1)", i64::MIN as f64),
            ("float(9223372036854775807)", i64::MAX as f64),
            ("float(-0.0)", -0.0),
            ("float(true)", 1.0),
            ("float(false)", 0.0),
            ("float('1.25')", 1.25),
            ("float('1e308')", 1e308),
        ] {
            let mut runtime = Runtime::new();
            runtime
                .register("float", NativeFunction::float_conversion())
                .unwrap();
            run(
                &mut runtime,
                &format!("$value={expression};"),
                &mut Vec::new(),
            )
            .unwrap();
            let Some(Value::Float(value)) = runtime.get_global("value") else {
                panic!("{expression}")
            };
            assert_eq!(value.to_bits(), expected.to_bits(), "{expression}");
        }
        for expression in [
            "float()",
            "float(1,2)",
            "float(null)",
            "float([])",
            "float('NaN')",
            "float('inf')",
            "float('1e999')",
            "float('no')",
        ] {
            let mut runtime = Runtime::new();
            runtime
                .register("float", NativeFunction::float_conversion())
                .unwrap();
            let error = run(&mut runtime, &format!("{expression};"), &mut Vec::new()).unwrap_err();
            assert!(
                error.message.contains("TypeError"),
                "{expression}: {error:?}"
            );
        }
    }

    #[test]
    fn cancellation_publishes_completed_writes_and_releases_scopes() {
        use std::sync::{Arc, atomic::AtomicBool};
        use std::time::Duration;
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..RuntimeOptions::default()
        });
        runtime
            .register("float", NativeFunction::float_conversion())
            .unwrap();
        let tokens = scr_lexer::lex(
            "$i=0;$total=0.0;for($i=0;$i<9223372036854775807;$i+=1){$total+=float(1);}",
        )
        .unwrap();
        let program = scr_parser::parse(&tokens).unwrap();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        let error = runtime.execute(&program, &mut Vec::new()).unwrap_err();
        worker.join().unwrap();
        assert!(error.message.contains("execution interrupted"));
        assert_eq!(runtime.float_loops_executed, 1);
        assert_eq!(runtime.environments.live, 1);
        let Value::Int(i) = runtime.get_global("i").unwrap() else {
            panic!()
        };
        let Value::Float(total) = runtime.get_global("total").unwrap() else {
            panic!()
        };
        assert!(*total == *i as f64 || *total == (*i + 1) as f64);
    }
}
