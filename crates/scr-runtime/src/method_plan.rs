//! Bounded straight-line scalar methods; unsupported entry states do no work.
use crate::{
    AssignmentOp, BinaryOp, Expr, ExprKind, Result, Runtime, StatementKind, Value,
    compact_call::Expression, objects::ScriptObject, value::UserDeclaration,
};
use scr_diagnostics::Span;

const PARAMETERS: usize = 8;
const FIELDS: usize = 8;
const STATEMENTS: usize = 16;
const NODES: usize = 256;
const DEPTH: usize = 16;

enum Step {
    Assign {
        field: usize,
        op: AssignmentOp,
        value: Expression,
        span: Span,
        target: Span,
    },
    Return {
        value: Option<Expression>,
        span: Span,
    },
}
pub(crate) struct Plan {
    fields: Box<[String]>,
    steps: Box<[Step]>,
}

fn field<'a>(expression: &'a Expr, fields: &mut Vec<String>) -> Option<&'a str> {
    let ExprKind::PropertyAccess(base, name) = &expression.kind else {
        return None;
    };
    if !matches!(&base.kind, ExprKind::Variable(name) if name == "this") {
        return None;
    }
    if !fields.contains(name) {
        if fields.len() == FIELDS {
            return None;
        }
        fields.push(name.clone());
    }
    Some(name)
}

fn inspect(
    expression: &Expr,
    fields: &mut Vec<String>,
    nodes: &mut usize,
    depth: usize,
) -> Option<()> {
    *nodes = nodes.checked_add(1)?;
    if *nodes > NODES || depth > DEPTH {
        return None;
    }
    match &expression.kind {
        ExprKind::Literal(_) | ExprKind::Variable(_) => {}
        ExprKind::Grouped(inner) | ExprKind::Unary(_, inner) => {
            inspect(inner, fields, nodes, depth + 1)?
        }
        ExprKind::Binary(left, _, right) => {
            inspect(left, fields, nodes, depth + 1)?;
            inspect(right, fields, nodes, depth + 1)?;
        }
        ExprKind::PropertyAccess(_, _) => {
            field(expression, fields)?;
        }
        _ => return None,
    }
    Some(())
}

impl Plan {
    fn compile(declaration: &UserDeclaration) -> Option<Box<Self>> {
        if declaration.parameters.len() > PARAMETERS
            || declaration.body.statements.len() > STATEMENTS
            || declaration
                .parameters
                .iter()
                .any(|p| p.default.is_some() || p.variadic)
        {
            return None;
        }
        let mut fields = Vec::new();
        let mut nodes = 0;
        for statement in &declaration.body.statements {
            match &statement.kind {
                StatementKind::Expression(Expr {
                    kind: ExprKind::Assignment(target, op, value),
                    ..
                }) if *op != AssignmentOp::Concat => {
                    field(target, &mut fields)?;
                    inspect(value, &mut fields, &mut nodes, 0)?;
                }
                StatementKind::Return(value) => {
                    if let Some(value) = value {
                        inspect(value, &mut fields, &mut nodes, 0)?;
                    }
                }
                _ => return None,
            }
        }
        let names: Vec<_> = declaration
            .parameters
            .iter()
            .map(|p| p.binding.name.as_str())
            .collect();
        let field_names: Vec<_> = fields.iter().map(String::as_str).collect();
        let compile = |value| Expression::compile_inputs(value, &names, &field_names);
        let mut steps = Vec::new();
        let mut instructions = 0;
        for statement in &declaration.body.statements {
            steps.push(match &statement.kind {
                StatementKind::Expression(Expr {
                    kind: ExprKind::Assignment(target, op, value),
                    ..
                }) => {
                    let ExprKind::PropertyAccess(_, name) = &target.kind else {
                        unreachable!()
                    };
                    let value = compile(value)?;
                    instructions += value.instruction_count();
                    Step::Assign {
                        field: fields.iter().position(|field| field == name)?,
                        op: *op,
                        value,
                        span: statement.span,
                        target: target.span,
                    }
                }
                StatementKind::Return(value) => {
                    let value = match value {
                        Some(value) => Some(compile(value)?),
                        None => None,
                    };
                    instructions += value.as_ref().map_or(0, Expression::instruction_count);
                    Step::Return {
                        value,
                        span: statement.span,
                    }
                }
                _ => unreachable!("validated straight-line body"),
            });
            if instructions > NODES {
                return None;
            }
        }
        Some(Box::new(Self {
            fields: fields.into(),
            steps: steps.into(),
        }))
    }
}

impl Runtime {
    pub(super) fn method_call(
        &mut self,
        declaration: &UserDeclaration,
        receiver: &Value,
        arguments: &[Value],
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_method_plans {
            return None;
        }
        if self.options.max_value_bytes.is_some() {
            return None;
        }
        // Reject unsupported bodies/defaults/variadics before scanning arguments
        // or receiver identities. Eligible arities are bounded by PARAMETERS.
        let plan = declaration
            .method_call
            .get_or_init(|| Plan::compile(declaration))
            .as_deref()?;
        if arguments.iter().any(|value| !Expression::scalar(value)) {
            return None;
        }
        let Value::NativeObject(object) = receiver else {
            return None;
        };
        // Test allocation provenance before a downcast. Arbitrary host proxies
        // keep exactly the old number and order of observable as_any/getter calls.
        if !self.environments.recent_script_object(object) {
            return None;
        }
        let instance = object.as_any()?.downcast_ref::<ScriptObject>()?;
        if !instance.class.parent.get().is_some_and(Option::is_none)
            || instance.exception.borrow().is_some()
            || instance.requires_snapshot.get()
        {
            return None;
        }
        let mut inputs: [Value; PARAMETERS + FIELDS] = std::array::from_fn(|_| Value::Null);
        for (input, value) in inputs.iter_mut().zip(arguments) {
            *input = value.clone();
        }
        // All referenced fields must already exist and be scalar. Otherwise
        // ordinary evaluation preserves missing-field errors and earlier writes.
        for (index, name) in plan.fields.iter().enumerate() {
            let slot = instance
                .class
                .declaration
                .fields
                .iter()
                .position(|f| f.binding.name == *name)?;
            let value = self
                .environments
                .scope(instance.scope)
                .variables
                .get_slot(slot, name)?;
            if !Expression::scalar(value) {
                return None;
            }
            inputs[arguments.len() + index] = value.clone();
        }
        #[cfg(test)]
        {
            self.method_plans_executed += 1;
        }
        Some((|| {
            for (parameter, value) in declaration.parameters.iter().zip(arguments) {
                self.annotation(&parameter.annotation, value, parameter.span, false)?;
            }
            let previous =
                self.enter_empty_scope(self.environments.current, declaration.body.span)?;
            let result = (|| {
                for step in &plan.steps {
                    match step {
                        Step::Assign {
                            field,
                            op,
                            value,
                            span,
                            target,
                        } => {
                            self.checkpoint(*span)?;
                            let old = &inputs[arguments.len() + field];
                            let mut value = value.evaluate_inputs(self, &inputs)?;
                            if *op != AssignmentOp::Assign {
                                let binary = match op {
                                    AssignmentOp::Add => BinaryOp::Add,
                                    AssignmentOp::Subtract => BinaryOp::Subtract,
                                    AssignmentOp::Multiply => BinaryOp::Multiply,
                                    AssignmentOp::Divide => BinaryOp::Divide,
                                    _ => unreachable!("scalar compound assignment"),
                                };
                                value = self.operation(
                                    crate::ops::binary(binary, old.clone(), value),
                                    *target,
                                )?;
                            }
                            // Keep the ordinary transactional field setter and
                            // its annotation, payload and instruction checks.
                            self.set_object_field(
                                object,
                                &plan.fields[*field],
                                value.clone(),
                                *target,
                            )?;
                            inputs[arguments.len() + field] = value;
                        }
                        Step::Return { value, span } => {
                            self.checkpoint(*span)?;
                            return match value {
                                Some(value) => value.evaluate_inputs(self, &inputs),
                                None => Ok(Value::Null),
                            };
                        }
                    }
                }
                Ok(Value::Null)
            })();
            self.environments.leave_empty(previous);
            let value = result?;
            self.annotation(
                &declaration.return_type,
                &value,
                declaration.name_span,
                true,
            )?;
            Ok(value)
        })())
    }
}

#[cfg(test)]
#[path = "method_plan_tests.rs"]
mod tests;
