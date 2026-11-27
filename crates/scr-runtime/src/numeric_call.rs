//! Bounded, declaration-owned plans for numeric leaf returns and comparisons.
use crate::{Result, Runtime, Value, float_loop::Number, value::UserDeclaration};
use scr_ast::{BinaryOp, Expr, ExprKind, Literal, StatementKind, TypeAnnotation, UnaryOp};
use scr_diagnostics::Span;

pub(super) const MAX_PARAMETERS: usize = 8;
const MAX_NODES: usize = 64;
const MAX_DEPTH: usize = 16;
const MAX_CAPTURE_NAME_BYTES: usize = 4096;

#[derive(Clone, Copy)]
enum Annotation {
    None,
    Int,
    Float,
    Bool,
    General,
}

impl Annotation {
    fn compile(annotation: &Option<TypeAnnotation>) -> Self {
        match annotation.as_ref().map(|a| a.name.as_str()) {
            None => Self::None,
            Some("int") => Self::Int,
            Some("float") => Self::Float,
            Some("bool") => Self::Bool,
            _ => Self::General,
        }
    }

    #[inline]
    fn check(
        self,
        runtime: &Runtime,
        annotation: &Option<TypeAnnotation>,
        value: &Value,
        span: Span,
        returning: bool,
    ) -> Result<()> {
        #[cfg(test)]
        if runtime.disable_numeric_annotations {
            return runtime.annotation(annotation, value, span, returning);
        }
        // declare_type rejects these exact primitive names, including host-built
        // AST declarations. They cannot name a shadowing script class. Other
        // names/qualified types and all mismatches retain normal resolution and
        // diagnostics; no successful value-type check is omitted.
        match (self, value) {
            (Self::None, _)
            | (Self::Int, Value::Int(_))
            | (Self::Float, Value::Float(_))
            | (Self::Bool, Value::Bool(_)) => Ok(()),
            _ => runtime.annotation(annotation, value, span, returning),
        }
    }
}

enum Numeric {
    Literal(Number),
    Parameter(usize),
    Unary(UnaryOp, Box<Numeric>, Span),
    Binary(Box<Numeric>, BinaryOp, Box<Numeric>, Span),
}

enum Leaf {
    Numeric(Numeric),
    Comparison {
        left: Numeric,
        op: BinaryOp,
        right: Numeric,
        span: Span,
    },
}

pub(super) struct Plan {
    expression: Leaf,
    integer: bool,
    parameter_count: u8,
    checkpoint: Span,
    parameters: [Annotation; MAX_PARAMETERS],
    returning: Annotation,
    scope_independent: bool,
}

struct Inputs<'a, 'b> {
    declaration: &'a UserDeclaration,
    captures: Option<&'b mut Vec<String>>,
}

impl Inputs<'_, '_> {
    fn resolve(&mut self, name: &str) -> Option<usize> {
        let parameters = &self.declaration.parameters;
        if let Some(index) = parameters.iter().position(|p| p.binding.name == name) {
            return Some(index);
        }
        let captures = self.captures.as_deref_mut()?;
        if name == "this" {
            return None;
        }
        let index = if let Some(index) = captures.iter().position(|capture| capture == name) {
            index
        } else {
            if parameters.len() + captures.len() >= MAX_PARAMETERS {
                return None;
            }
            let bytes = captures.iter().try_fold(name.len(), |bytes, capture| {
                bytes.checked_add(capture.len())
            })?;
            if bytes > MAX_CAPTURE_NAME_BYTES {
                return None;
            }
            captures.push(name.into());
            captures.len() - 1
        };
        Some(parameters.len() + index)
    }
}

impl Plan {
    pub(super) fn compile(declaration: &UserDeclaration) -> Option<Box<Self>> {
        Self::compile_inputs(declaration, None)
    }

    pub(super) fn compile_captured(
        declaration: &UserDeclaration,
    ) -> Option<(Box<Self>, Box<[String]>)> {
        let mut captures = Vec::new();
        let plan = Self::compile_inputs(declaration, Some(&mut captures))?;
        if captures.is_empty() || !plan.scope_independent {
            return None;
        }
        Some((plan, captures.into_boxed_slice()))
    }

    fn compile_inputs(
        declaration: &UserDeclaration,
        captures: Option<&mut Vec<String>>,
    ) -> Option<Box<Self>> {
        if declaration.parameters.len() > MAX_PARAMETERS
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
        let [statement] = declaration.body.statements.as_slice() else {
            return None;
        };
        let StatementKind::Return(Some(expression)) = &statement.kind else {
            return None;
        };
        let mut inputs = Inputs {
            declaration,
            captures,
        };
        let expression = Self::leaf(&mut inputs, expression, &mut 0, 0)?;
        let input_count = declaration.parameters.len()
            + inputs
                .captures
                .as_ref()
                .map_or(0, |captures| captures.len());
        let mut parameters = [Annotation::None; MAX_PARAMETERS];
        for (slot, parameter) in parameters.iter_mut().zip(&declaration.parameters) {
            *slot = Annotation::compile(&parameter.annotation);
        }
        // Caller-loop plans require an integer return, not just integer inputs.
        let integer = matches!(&expression, Leaf::Numeric(value) if value.integer_only());
        let returning = Annotation::compile(&declaration.return_type);
        let scope_independent = parameters
            .iter()
            .all(|kind| !matches!(kind, Annotation::General))
            && !matches!(returning, Annotation::General);
        Some(Box::new(Self {
            expression,
            integer,
            parameter_count: input_count as u8,
            checkpoint: statement.span,
            parameters,
            returning,
            scope_independent,
        }))
    }

    fn leaf(
        inputs: &mut Inputs<'_, '_>,
        expr: &Expr,
        nodes: &mut usize,
        depth: usize,
    ) -> Option<Leaf> {
        match &expr.kind {
            ExprKind::Grouped(inner) => {
                *nodes += 1;
                if *nodes > MAX_NODES || depth > MAX_DEPTH {
                    return None;
                }
                Self::leaf(inputs, inner, nodes, depth + 1)
            }
            ExprKind::Binary(
                left,
                op @ (BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual),
                right,
            ) => {
                *nodes += 1;
                if *nodes > MAX_NODES || depth > MAX_DEPTH {
                    return None;
                }
                Some(Leaf::Comparison {
                    left: Self::expression(inputs, left, nodes, depth + 1)?,
                    op: *op,
                    right: Self::expression(inputs, right, nodes, depth + 1)?,
                    span: expr.span,
                })
            }
            _ => Some(Leaf::Numeric(Self::expression(inputs, expr, nodes, depth)?)),
        }
    }

    fn expression(
        inputs: &mut Inputs<'_, '_>,
        expr: &Expr,
        nodes: &mut usize,
        depth: usize,
    ) -> Option<Numeric> {
        *nodes += 1;
        if *nodes > MAX_NODES || depth > MAX_DEPTH {
            return None;
        }
        Some(match &expr.kind {
            ExprKind::Literal(Literal::Int(v)) => Numeric::Literal(Number::Int(*v)),
            ExprKind::Literal(Literal::Float(v)) if v.is_finite() => {
                Numeric::Literal(Number::Float(*v))
            }
            ExprKind::Variable(name) => Numeric::Parameter(inputs.resolve(name)?),
            ExprKind::Grouped(inner) => {
                return Self::expression(inputs, inner, nodes, depth + 1);
            }
            ExprKind::Unary(op @ (UnaryOp::Positive | UnaryOp::Negative), inner) => Numeric::Unary(
                *op,
                Box::new(Self::expression(inputs, inner, nodes, depth + 1)?),
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
            ) => Numeric::Binary(
                Box::new(Self::expression(inputs, left, nodes, depth + 1)?),
                *op,
                Box::new(Self::expression(inputs, right, nodes, depth + 1)?),
                expr.span,
            ),
            _ => return None,
        })
    }
}

impl Numeric {
    fn evaluate_integer(
        &self,
        arguments: &[Number; MAX_PARAMETERS],
        runtime: &Runtime,
    ) -> Result<i64> {
        match self {
            Self::Literal(Number::Int(value)) => Ok(*value),
            Self::Parameter(index) => {
                let Number::Int(value) = arguments[*index] else {
                    unreachable!("proven integer arguments");
                };
                Ok(value)
            }
            Self::Unary(op, inner, span) => {
                let value = inner.evaluate_integer(arguments, runtime)?;
                runtime.operation(
                    match op {
                        UnaryOp::Positive => Ok(value),
                        UnaryOp::Negative => value
                            .checked_neg()
                            .ok_or(crate::ops::Error::Arithmetic("integer arithmetic overflow")),
                        _ => unreachable!("verified numeric unary operator"),
                    },
                    *span,
                )
            }
            Self::Binary(left, op, right, span) => {
                let left = left.evaluate_integer(arguments, runtime)?;
                let right = right.evaluate_integer(arguments, runtime)?;
                runtime.operation(crate::ops::integer(*op, left, right), *span)
            }
            Self::Literal(Number::Float(_)) => unreachable!("proven integer expression"),
        }
    }

    fn integer_only(&self) -> bool {
        match self {
            Self::Literal(Number::Int(_)) | Self::Parameter(_) => true,
            Self::Literal(Number::Float(_)) => false,
            Self::Unary(_, value, _) => value.integer_only(),
            Self::Binary(left, _, right, _) => left.integer_only() && right.integer_only(),
        }
    }
    fn evaluate(&self, arguments: &[Number; MAX_PARAMETERS], runtime: &Runtime) -> Result<Number> {
        match self {
            Self::Literal(v) => Ok(*v),
            Self::Parameter(index) => Ok(arguments[*index]),
            Self::Unary(op, inner, span) => {
                let value = inner.evaluate(arguments, runtime)?;
                runtime.operation(value.unary(*op), *span)
            }
            Self::Binary(left, op, right, span) => {
                let left = left.evaluate(arguments, runtime)?;
                let right = right.evaluate(arguments, runtime)?;
                runtime.operation(left.binary(*op, right), *span)
            }
        }
    }
}

impl Plan {
    #[inline(never)]
    fn evaluate(
        &self,
        arguments: &[Number; MAX_PARAMETERS],
        runtime: &mut Runtime,
    ) -> Result<Value> {
        let expression = match &self.expression {
            Leaf::Numeric(expression) => expression,
            Leaf::Comparison {
                left,
                op,
                right,
                span,
            } => {
                #[cfg(test)]
                {
                    runtime.numeric_predicates_executed += 1;
                }
                // Evaluate operands left-to-right and use ordinary comparison
                // semantics, including strict equality and mixed-type errors.
                let left = left.evaluate(arguments, runtime)?.into_value();
                let right = right.evaluate(arguments, runtime)?.into_value();
                return runtime.operation(crate::ops::binary(*op, left, right), *span);
            }
        };
        let integer = self.integer
            && arguments[..usize::from(self.parameter_count)]
                .iter()
                .all(|value| matches!(value, Number::Int(_)));
        #[cfg(test)]
        let integer = integer && !runtime.disable_integer_numeric_calls;
        if integer {
            #[cfg(test)]
            {
                runtime.integer_numeric_calls_executed += 1;
            }
            expression
                .evaluate_integer(arguments, runtime)
                .map(Value::Int)
        } else {
            expression
                .evaluate(arguments, runtime)
                .map(Number::into_value)
        }
    }

    pub(super) fn integer_only(&self) -> bool {
        self.integer
    }
}

impl Runtime {
    // Called only after ordinary owner/arity/frame/source setup and the cached
    // numeric-argument guard. Numeric plans cannot read lexical bindings or
    // capture scopes. Primitive annotations are independent of lexical classes;
    // configured scope/value quotas and every cold/general case stay ordinary.
    pub(super) fn numeric_call_without_scopes(
        &mut self,
        declaration: &UserDeclaration,
        values: &[Value],
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_numeric_setup_elision {
            return None;
        }
        let plan = declaration.numeric_call.get()?.as_deref()?;
        if !plan.scope_independent {
            return None;
        }
        let result = plan.invoke::<false>(self, declaration, values)?;
        #[cfg(test)]
        {
            self.numeric_parameter_scopes_reused += 1;
            self.numeric_setup_elisions += 1;
        }
        Some(result)
    }

    #[inline]
    pub(super) fn can_reuse_numeric_parameter_scope(
        &self,
        declaration: &UserDeclaration,
        values: &[Value],
    ) -> bool {
        #[cfg(test)]
        if self.disable_numeric_calls || self.disable_numeric_parameter_scopes {
            return false;
        }
        // Cold calls and negative cache entries stay on the existing path.
        // Do not admit or compile a declaration before its parameter-scope
        // limit check. All arguments, including unused ones, must be scalars.
        matches!(declaration.numeric_call.get(), Some(Some(_)))
            && values.len() == declaration.parameters.len()
            && values.iter().all(|value| match value {
                Value::Int(_) => true,
                Value::Float(value) => value.is_finite(),
                _ => false,
            })
    }
    // Call ownership, arity, frame, source and parameter scope are already set
    // up by the ordinary call path. Plans never hold runtime Values or scopes.
    pub(super) fn numeric_call(
        &mut self,
        declaration: &UserDeclaration,
        values: &[Value],
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_numeric_calls {
            return None;
        }
        let cached = match declaration.numeric_call.get() {
            Some(cached) => cached,
            None => {
                // A function called once should pay no compilation cost. Warm
                // declarations cache both supported plans and rejected shapes.
                let first = !declaration.numeric_seen.replace(true);
                #[cfg(not(test))]
                let force = false;
                #[cfg(test)]
                let force = self.force_numeric_calls;
                if first && !force {
                    return None;
                }
                declaration
                    .numeric_call
                    .get_or_init(|| Plan::compile(declaration))
            }
        };
        let plan = cached.as_deref()?;
        plan.invoke::<true>(self, declaration, values)
    }
}

impl Plan {
    #[inline]
    fn invoke<const SCOPED: bool>(
        &self,
        runtime: &mut Runtime,
        declaration: &UserDeclaration,
        values: &[Value],
    ) -> Option<Result<Value>> {
        debug_assert_eq!(values.len(), declaration.parameters.len());
        let mut arguments = [Number::Int(0); MAX_PARAMETERS];
        for (slot, value) in arguments.iter_mut().zip(values) {
            *slot = Number::from_value(value)?;
        }
        #[cfg(test)]
        {
            runtime.numeric_calls_executed += 1;
        }
        Some((|| {
            // Keep value checks, general resolution and their original order. Only
            // numeric arguments are eligible, so omitting their lexical copies
            // cannot change ownership counts or collector edges.
            for (kind, (parameter, value)) in self
                .parameters
                .iter()
                .zip(declaration.parameters.iter().zip(values))
            {
                kind.check(runtime, &parameter.annotation, value, parameter.span, false)?;
            }
            let previous = if SCOPED {
                Some(
                    runtime
                        .enter_empty_scope(runtime.environments.current, declaration.body.span)?,
                )
            } else {
                None
            };
            let result = (|| {
                runtime.checkpoint(self.checkpoint)?;
                self.evaluate(&arguments, runtime)
            })();
            if let Some(previous) = previous {
                runtime.environments.leave_empty(previous);
            }
            let value = result?;
            self.returning.check(
                runtime,
                &declaration.return_type,
                &value,
                declaration.name_span,
                true,
            )?;
            Ok(value)
        })())
    }
}

impl Plan {
    pub(super) fn invoke_captured_inputs(
        &self,
        runtime: &mut Runtime,
        declaration: &UserDeclaration,
        values: &[Value],
        arguments: &[Number; MAX_PARAMETERS],
    ) -> Result<Value> {
        for (kind, (parameter, value)) in self
            .parameters
            .iter()
            .zip(declaration.parameters.iter().zip(values))
        {
            kind.check(runtime, &parameter.annotation, value, parameter.span, false)?;
        }
        runtime.checkpoint(self.checkpoint)?;
        let value = self.evaluate(arguments, runtime)?;
        self.returning.check(
            runtime,
            &declaration.return_type,
            &value,
            declaration.name_span,
            true,
        )?;
        Ok(value)
    }
}

#[cfg(test)]
mod annotation_tests;
#[cfg(test)]
mod integer_tests;
#[cfg(test)]
mod parameter_scope_tests;
#[cfg(test)]
mod predicate_tests;
#[cfg(test)]
mod setup_tests;
#[cfg(test)]
mod tests;
