//! Bounded straight-line scalar methods; unsupported entry states do no work.
use crate::{
    AssignmentOp, BinaryOp, Expr, ExprKind, Result, Runtime, StatementKind, Value,
    compact_call::Expression,
    objects::{ScriptClass, ScriptObject},
    value::{SpecialCall, UserDeclaration},
};
use scr_diagnostics::Span;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

const PARAMETERS: usize = 8;
const FIELDS: usize = 8;
const CACHED_LAYOUT_FIELDS: usize = 64;
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
    positions: RefCell<Option<FieldPositions>>,
}

struct FieldPositions {
    class: Weak<ScriptClass>,
    slots: [usize; FIELDS],
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
            positions: RefCell::new(None),
        }))
    }

    fn field_positions(&self, class: &Rc<ScriptClass>) -> Option<[usize; FIELDS]> {
        let mut slots = [0; FIELDS];
        if let Some(cached) = self.positions.borrow().as_ref() {
            // The weak allocation identity cannot be reused while cached. It
            // retains neither the class value nor any receiver or environment.
            if cached.class.as_ptr() == Rc::as_ptr(class) {
                return Some(cached.slots);
            }
        }
        for (slot, name) in slots.iter_mut().zip(&self.fields) {
            *slot = class
                .declaration
                .fields
                .iter()
                .position(|field| field.binding.name == *name)?;
        }
        *self.positions.borrow_mut() = Some(FieldPositions {
            class: Rc::downgrade(class),
            slots,
        });
        Some(slots)
    }
}

impl Runtime {
    // Speculative syntax/state proof only: do not warm plans, resolve methods
    // through host hooks, or retain receivers while planning a caller loop.
    pub(super) fn integer_loop_method_scope(
        &self,
        base: &Expr,
        name: &str,
        arguments: usize,
    ) -> Option<usize> {
        let ExprKind::Variable(binding) = &crate::literal_read::ungroup(base).kind else {
            return None;
        };
        if binding == "this" {
            return None;
        }
        let (_, Value::NativeObject(object)) = self.environments.lookup(binding, false)? else {
            return None;
        };
        if !self.environments.recent_script_object(object) {
            return None;
        }
        let instance = object.as_any()?.downcast_ref::<ScriptObject>()?;
        let class = &instance.class;
        if !Rc::ptr_eq(&class.owner, &self.identity)
            || !class.parent.get().is_some_and(Option::is_none)
            || instance.exception.borrow().is_some()
            || instance.requires_snapshot.get()
            || class
                .declaration
                .fields
                .iter()
                .any(|field| field.binding.name == name)
        {
            return None;
        }
        let declaration = class
            .declaration
            .methods
            .iter()
            .find(|method| method.name == name)?;
        let primitive = |annotation: &Option<crate::TypeAnnotation>| {
            annotation
                .as_ref()
                .is_none_or(|ty| matches!(ty.name.as_str(), "int" | "bool"))
        };
        if declaration.parameters.len() != arguments
            || declaration.return_type.as_ref()?.name != "int"
            || declaration.parameters.iter().any(|parameter| {
                parameter.binding.name == "this" || !primitive(&parameter.annotation)
            })
        {
            return None;
        }
        // Successful returns must be real ints. The scalar body proof excludes
        // callbacks, free-variable access, declarations and caller writes.
        let proof = Plan::compile(declaration)?;
        for name in &proof.fields {
            let (slot, field) = class
                .declaration
                .fields
                .iter()
                .enumerate()
                .find(|(_, field)| field.binding.name == *name)?;
            if !primitive(&field.annotation)
                || !self
                    .environments
                    .scope(instance.scope)
                    .variables
                    .get_slot(slot, name)
                    .is_some_and(Expression::scalar)
            {
                return None;
            }
        }
        Some(instance.scope)
    }

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
        let cached = declaration
            .special_call
            .get_or_init(|| Plan::compile(declaration).map(SpecialCall::Method));
        let Some(SpecialCall::Method(plan)) = cached else {
            return None;
        };
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
            || !Rc::ptr_eq(&instance.class.owner, &self.identity)
        {
            return None;
        }
        let single_field = instance.class.declaration.fields.len() == 1;
        // Short layout walks are cheaper than maintaining a class hint.
        let positions = if instance.class.declaration.fields.len() < CACHED_LAYOUT_FIELDS
            || plan.fields.is_empty()
        {
            None
        } else {
            Some(plan.field_positions(&instance.class)?)
        };
        let mut inputs: [Value; PARAMETERS + FIELDS] = std::array::from_fn(|_| Value::Null);
        for (input, value) in inputs.iter_mut().zip(arguments) {
            *input = value.clone();
        }
        // All referenced fields must already exist and be scalar. Otherwise
        // ordinary evaluation preserves missing-field errors and earlier writes.
        for (index, name) in plan.fields.iter().enumerate() {
            let slot = if let Some(positions) = &positions {
                let slot = positions[index];
                instance
                    .class
                    .declaration
                    .fields
                    .get(slot)
                    .filter(|field| field.binding.name == *name)?;
                slot
            } else {
                instance
                    .class
                    .declaration
                    .fields
                    .iter()
                    .position(|field| field.binding.name == *name)?
            };
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
                            let fixed_scalar = single_field && Expression::scalar(&value);
                            #[cfg(test)]
                            let fixed_scalar = fixed_scalar && !self.force_field_snapshot;
                            if fixed_scalar {
                                self.set_scalar_method_field(instance, value.clone(), *target)?;
                            } else {
                                self.set_object_field(
                                    object,
                                    &plan.fields[*field],
                                    value.clone(),
                                    *target,
                                )?;
                            }
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

    // Entry guards prove a runtime-owned, non-exception, snapshot-free object
    // with one initialized scalar field and no byte quota. Like the setter's
    // fixed-scalar branch, this write cannot change its logical payload size.
    fn set_scalar_method_field(
        &mut self,
        instance: &ScriptObject,
        value: Value,
        span: Span,
    ) -> Result<()> {
        let field = &instance.class.declaration.fields[0];
        let previous = self.environments.current;
        self.environments.current = instance.class.environment;
        let annotation = self.annotation(&field.annotation, &value, span, false);
        self.environments.current = previous;
        annotation?;
        self.checkpoint(span)?;
        // Reborrow storage after the checkpoint/collector, preserving the
        // name check for promoted or non-positional binding stores.
        *self
            .environments
            .scope_mut(instance.scope)
            .variables
            .get_slot_mut(0, &field.binding.name)
            .expect("guarded initialized scalar field") = value;
        Ok(())
    }
}

#[cfg(test)]
#[path = "method_plan_tests.rs"]
mod tests;
