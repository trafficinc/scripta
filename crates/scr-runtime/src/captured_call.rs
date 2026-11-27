//! Numeric leaf callbacks read live captures into bounded invocation-local slots.
use crate::{
    Function, MAX_RECURSION_LIMIT, Result, Runtime, Span, Value,
    float_loop::Number,
    numeric_call,
    value::{SpecialCall, UserDeclaration},
};

pub(super) struct Plan {
    numeric: Box<numeric_call::Plan>,
    names: Box<[String]>,
}

impl Plan {
    fn compile(declaration: &UserDeclaration) -> Option<Box<Self>> {
        let (numeric, names) = numeric_call::Plan::compile_captured(declaration)?;
        Some(Box::new(Self { numeric, names }))
    }
}

impl Runtime {
    // Keep the captured entry out of ordinary numeric call setup. Failed guards
    // return before user work, leaving the existing entry to report owner, arity,
    // depth and unsupported-value errors in their original order.
    #[inline(never)]
    pub(super) fn captured_call(
        &mut self,
        function: &Function,
        values: &[Value],
        span: Span,
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_captured_calls {
            return None;
        }
        let Function::User {
            declaration,
            environment,
            receiver: None,
            owner,
            source,
        } = function
        else {
            return None;
        };
        if !std::rc::Rc::ptr_eq(owner, &self.identity)
            || self.stack.len().saturating_sub(1)
                >= self.options.recursion_limit.min(MAX_RECURSION_LIMIT)
            || self.options.max_scopes.is_some()
            || self.options.max_value_bytes.is_some()
            || matches!(declaration.numeric_call.get(), Some(Some(_)))
            || values.len() != declaration.parameters.len()
        {
            return None;
        }
        // Leave first calls on ordinary evaluation. Cache both accepted and
        // unsupported shapes; plans retain names/operators, never capture Values
        // or scope IDs, so sibling closures and arena reuse cannot stale them.
        let cached = match declaration.special_call.get() {
            Some(cached) => cached,
            None => {
                if !declaration.numeric_seen.get() {
                    return None;
                }
                declaration
                    .special_call
                    .get_or_init(|| Plan::compile(declaration).map(SpecialCall::Captured))
            }
        };
        let Some(SpecialCall::Captured(plan)) = cached else {
            return None;
        };
        let mut inputs = [Number::Int(0); numeric_call::MAX_PARAMETERS];
        for (slot, value) in inputs.iter_mut().zip(values) {
            *slot = Number::from_value(value)?;
        }
        for (slot, name) in inputs[values.len()..].iter_mut().zip(plan.names.iter()) {
            *slot = Number::from_value(self.environments.capture_value(*environment, name)?)?;
        }
        #[cfg(test)]
        {
            self.captured_calls_executed += 1;
        }
        // No operation between the guards and the return expression can invoke
        // script code or mutate a binding. Preserve parameter annotations, the
        // return-statement checkpoint, arithmetic errors and return annotations.
        let frame = crate::error::ActiveFrame::call(function, span, self.source.clone());
        #[cfg(test)]
        let frame = if self.disable_call_reuse {
            frame.snapshot().into()
        } else {
            frame
        };
        self.stack.push(frame);
        let caller_source = self.source.clone();
        self.source = source.clone();
        let result = plan
            .numeric
            .invoke_captured_inputs(self, declaration, values, &inputs);
        self.source = caller_source;
        self.stack.pop();
        Some(result)
    }
}

#[cfg(test)]
mod tests;
