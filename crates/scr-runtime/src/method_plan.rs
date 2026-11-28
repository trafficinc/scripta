//! Bounded scalar methods; unsupported entry states do no work.
use crate::{
    AssignmentOp, BinaryOp, Expr, ExprKind, Result, Runtime, StatementKind, Value,
    compact_call::{Expression, Scalar},
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

#[path = "method_plan_compiler.rs"]
mod compiler;

enum Flow {
    Action(usize),
    Tick(Span),
    Test {
        value: Expression,
        span: Span,
        next: usize,
    },
    Jump(usize),
    Enter(Span),
    Leave,
}

struct MethodFields<'a> {
    object: &'a Rc<dyn crate::NativeObject>,
    instance: &'a ScriptObject,
    single_field: bool,
}

enum Step {
    Assign {
        field: usize,
        op: AssignmentOp,
        value: Expression,
        span: Span,
        target: Span,
    },
    Local {
        slot: usize,
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
    // Parameters and definitely initialized locals share the existing bounded
    // input region. Fields retain a fixed offset after all of these bindings.
    locals: Box<[String]>,
    steps: Box<[Step]>,
    flow: Option<Box<[Flow]>>,
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
    names: &[&str],
) -> Option<()> {
    *nodes = nodes.checked_add(1)?;
    if *nodes > NODES || depth > DEPTH {
        return None;
    }
    match &expression.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Variable(name) if names.contains(&name.as_str()) => {}
        ExprKind::Grouped(inner) | ExprKind::Unary(_, inner) => {
            inspect(inner, fields, nodes, depth + 1, names)?
        }
        ExprKind::Binary(left, _, right) => {
            inspect(left, fields, nodes, depth + 1, names)?;
            inspect(right, fields, nodes, depth + 1, names)?;
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
        compiler::compile(declaration)
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
        // Local bodies are optimized independently first. Do not silently widen
        // caller-loop admission or its capture/ownership proof in this change.
        if !proof.locals.is_empty() || proof.flow.is_some() {
            return None;
        }
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
        #[cfg(test)]
        if self.disable_method_local_plans && !plan.locals.is_empty() {
            return None;
        }
        // A variable assigned in a method can refer to a captured module name.
        // Only genuinely fresh body bindings may stay in the private input slots.
        if instance.class.environment != 0
            && plan.locals.iter().any(|name| {
                self.environments
                    .capture_value(instance.class.environment, name)
                    .is_some()
            })
        {
            return None;
        }
        #[cfg(test)]
        if self.disable_method_branch_plans && plan.flow.is_some() {
            return None;
        }
        let bindings = arguments.len() + plan.locals.len();
        let single_field = instance.class.declaration.fields.len() == 1;
        // Short layout walks are cheaper than maintaining a class hint.
        let positions = if instance.class.declaration.fields.len() < CACHED_LAYOUT_FIELDS
            || plan.fields.is_empty()
        {
            None
        } else {
            Some(plan.field_positions(&instance.class)?)
        };
        let mut inputs = [Scalar::Null; PARAMETERS + FIELDS];
        for (input, value) in inputs.iter_mut().zip(arguments) {
            *input = Scalar::from_value(value).expect("guarded scalar arguments");
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
            inputs[bindings + index] = Scalar::from_value(value)?;
        }
        #[cfg(test)]
        {
            self.method_plans_executed += 1;
            self.method_local_plans_executed += usize::from(!plan.locals.is_empty());
            self.method_branch_plans_executed += usize::from(plan.flow.is_some());
        }
        Some((|| {
            for (parameter, value) in declaration.parameters.iter().zip(arguments) {
                self.annotation(&parameter.annotation, value, parameter.span, false)?;
            }
            let previous =
                self.enter_empty_scope(self.environments.current, declaration.body.span)?;
            let result = (|| {
                let fields = MethodFields {
                    object,
                    instance,
                    single_field,
                };
                if let Some(flow) = &plan.flow {
                    return self.method_flow(plan, flow, &fields, &mut inputs, bindings);
                }
                for step in &plan.steps {
                    if let Some(value) =
                        self.method_step(step, plan, &fields, &mut inputs, bindings)?
                    {
                        return Ok(value);
                    }
                }
                Ok(Scalar::Null)
            })();
            self.environments.leave_empty(previous);
            let value = result?.value();
            self.annotation(
                &declaration.return_type,
                &value,
                declaration.name_span,
                true,
            )?;
            Ok(value)
        })())
    }

    #[inline(always)]
    fn method_step(
        &mut self,
        step: &Step,
        plan: &Plan,
        fields: &MethodFields<'_>,
        inputs: &mut [Scalar],
        bindings: usize,
    ) -> Result<Option<Scalar>> {
        let object = fields.object;
        let instance = fields.instance;
        let single_field = fields.single_field;
        match step {
            Step::Assign {
                field,
                op,
                value,
                span,
                target,
            } => {
                self.checkpoint(*span)?;
                let old = &inputs[bindings + field];
                let mut value = value.evaluate_scalars(self, inputs)?;
                if *op != AssignmentOp::Assign {
                    let binary = match op {
                        AssignmentOp::Add => BinaryOp::Add,
                        AssignmentOp::Subtract => BinaryOp::Subtract,
                        AssignmentOp::Multiply => BinaryOp::Multiply,
                        AssignmentOp::Divide => BinaryOp::Divide,
                        _ => unreachable!("scalar compound assignment"),
                    };
                    value = self.operation(old.binary(binary, value), *target)?;
                }
                let fixed_scalar = single_field;
                #[cfg(test)]
                let fixed_scalar = fixed_scalar && !self.force_field_snapshot;
                if fixed_scalar {
                    self.set_scalar_method_field(instance, value.value(), *target)?;
                } else {
                    self.set_object_field(object, &plan.fields[*field], value.value(), *target)?;
                }
                inputs[bindings + field] = value;
            }
            Step::Local {
                slot,
                op,
                value,
                span,
                target,
            } => {
                self.checkpoint(*span)?;
                let old = &inputs[*slot];
                let mut value = value.evaluate_scalars(self, inputs)?;
                if *op != AssignmentOp::Assign {
                    let binary = match op {
                        AssignmentOp::Add => BinaryOp::Add,
                        AssignmentOp::Subtract => BinaryOp::Subtract,
                        AssignmentOp::Multiply => BinaryOp::Multiply,
                        AssignmentOp::Divide => BinaryOp::Divide,
                        _ => unreachable!("scalar local compound assignment"),
                    };
                    value = self.operation(old.binary(binary, value), *target)?;
                }
                // Read-only scalar expressions cannot produce handles;
                // no byte quota, capture or callback can observe these
                // fresh bindings before the ordinary scope exit.
                inputs[*slot] = value;
            }
            Step::Return { value, span } => {
                self.checkpoint(*span)?;
                return match value {
                    Some(value) => value.evaluate_scalars(self, inputs),
                    None => Ok(Scalar::Null),
                }
                .map(Some);
            }
        }
        Ok(None)
    }

    fn method_flow(
        &mut self,
        plan: &Plan,
        flow: &[Flow],
        fields: &MethodFields<'_>,
        inputs: &mut [Scalar],
        bindings: usize,
    ) -> Result<Scalar> {
        let mut scopes = [0; DEPTH];
        let mut depth = 0;
        let result = (|| {
            let mut ip = 0;
            while let Some(instruction) = flow.get(ip) {
                ip += 1;
                match instruction {
                    Flow::Action(index) => {
                        if let Some(value) =
                            self.method_step(&plan.steps[*index], plan, fields, inputs, bindings)?
                        {
                            return Ok(value);
                        }
                    }
                    Flow::Tick(span) => self.checkpoint(*span)?,
                    Flow::Test { value, span, next } => {
                        let value = value.evaluate_scalars(self, inputs)?;
                        if !self.operation(value.boolean(), *span)? {
                            ip = *next;
                        }
                    }
                    Flow::Jump(next) => ip = *next,
                    Flow::Enter(span) => {
                        scopes[depth] = self.enter_empty_scope(self.environments.current, *span)?;
                        depth += 1;
                    }
                    Flow::Leave => {
                        depth -= 1;
                        self.environments.leave_empty(scopes[depth]);
                    }
                }
            }
            Ok(Scalar::Null)
        })();
        // Returns, cancellation and every failure must unwind all selected
        // branch blocks before the ordinary body/parameter scopes and annotation.
        while depth > 0 {
            depth -= 1;
            self.environments.leave_empty(scopes[depth]);
        }
        result
    }

    // Entry guards prove a runtime-owned, non-exception, snapshot-free object
    // with one initialized scalar field and no byte quota. Like the setter's
    // fixed-scalar branch, this write cannot change its logical payload size.
    #[inline(always)]
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
