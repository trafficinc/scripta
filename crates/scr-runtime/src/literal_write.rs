//! Single-index assignments borrow literal keys until the write commits.
use crate::{
    AssignmentOp, BinaryOp, Expr, ExprKind, Literal, Place, Result, Root, Runtime, ScriptOutput,
    Signal, Span, Value, literal_read::ungroup, map_index, ops,
};

impl Runtime {
    #[inline(never)]
    pub(super) fn literal_index_assignment(
        &mut self,
        target: &Expr,
        op: AssignmentOp,
        rhs: &Expr,
        retain: bool,
        output: &mut dyn ScriptOutput,
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_literal_writes {
            return None;
        }
        // Quotas retain ordinary key validation and transactional root copies.
        if self.options.max_value_bytes.is_some() {
            return None;
        }
        let ExprKind::IndexAccess(base, index) = &ungroup(target).kind else {
            return None;
        };
        let ExprKind::Variable(name) = &ungroup(base).kind else {
            return None;
        };
        if name == "this" {
            return None;
        }
        let key = match &ungroup(index).kind {
            ExprKind::Literal(Literal::Int(value)) => ops::IndexKey::Int(*value),
            ExprKind::Literal(Literal::String(text)) => ops::IndexKey::String(text),
            _ => return None,
        };
        #[cfg(test)]
        {
            self.literal_writes_executed += 1;
        }
        Some((|| {
            // Resolve the root before the RHS, without retaining its value.
            let (scope, slot, root) =
                self.environments.resolve_variable(name).ok_or_else(|| {
                    Signal::Error(self.error(format!("undefined variable ${name}"), base.span))
                })?;
            let old = if op == AssignmentOp::Assign {
                None
            } else {
                let hint = match root {
                    Value::Map(entries) if map_index::eligible_size(entries.len()) => self
                        .map_indexes
                        .borrow_mut()
                        .position_key(scope, name, entries, key, true),
                    _ => None,
                };
                Some(
                    if let (Some(position), Value::Map(entries)) = (hint, root) {
                        entries[position].1.clone()
                    } else {
                        self.operation(ops::index_ref_key(root, key), target.span)?
                            .clone()
                    },
                )
            };
            let mut value = self.evaluate(rhs, output)?;
            if let Some(old) = old {
                let binary = match op {
                    AssignmentOp::Add => BinaryOp::Add,
                    AssignmentOp::Subtract => BinaryOp::Subtract,
                    AssignmentOp::Multiply => BinaryOp::Multiply,
                    AssignmentOp::Divide => BinaryOp::Divide,
                    AssignmentOp::Concat => BinaryOp::Concat,
                    AssignmentOp::Assign => unreachable!(),
                };
                value = self.operation(ops::binary(binary, old, value), target.span)?;
            }
            if retain {
                self.write_literal_index(scope, name, slot, key, value.clone(), target.span)?;
                Ok(value)
            } else {
                self.write_literal_index(scope, name, slot, key, value, target.span)?;
                Ok(Value::Null)
            }
        })())
    }

    fn write_literal_index(
        &mut self,
        scope: usize,
        name: &str,
        slot: usize,
        key: ops::IndexKey<'_>,
        replacement: Value,
        span: Span,
    ) -> Result<()> {
        // A reentrant host can change runtime settings. Do not rerun the RHS;
        // reconstruct only the deferred write if a quota became active.
        if self.options.max_value_bytes.is_some() {
            let key = match key {
                ops::IndexKey::Int(value) => Value::Int(value),
                ops::IndexKey::String(text) => Value::String(text.to_owned()),
            };
            return self.write_place(
                &Place {
                    root: Root::Variable { scope, name, slot },
                    path: vec![key],
                    append: false,
                },
                replacement,
                span,
            );
        }
        // Match write_place: validate the replacement before inspecting the
        // current root or detaching its storage. Native payload hooks stay live.
        self.validate_value(&replacement, 1, span)?;
        let hint = match self
            .environments
            .scope(scope)
            .variables
            .get_slot(slot, name)
        {
            Some(Value::Map(entries)) if map_index::eligible_size(entries.len()) => self
                .map_indexes
                .borrow_mut()
                .position_key(scope, name, entries, key, false),
            _ => None,
        };
        let current = self
            .environments
            .scope_mut(scope)
            .variables
            .get_slot_mut(slot, name)
            .unwrap();
        let result = ops::set_index_key(current, key, replacement, hint);
        self.operation(result, span)
    }
}

#[cfg(test)]
mod tests;
