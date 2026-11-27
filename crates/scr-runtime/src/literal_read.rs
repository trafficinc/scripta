//! Literal reads borrow AST keys without constructing an assignment path.
use crate::{Expr, ExprKind, Literal, Result, Runtime, Signal, Span, Value, ops};

pub(super) fn ungroup(mut expression: &Expr) -> &Expr {
    while let ExprKind::Grouped(inner) = &expression.kind {
        expression = inner;
    }
    expression
}

impl Runtime {
    pub(super) fn literal_index_read(
        &mut self,
        base: &Expr,
        index: &Expr,
        span: Span,
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_literal_reads {
            return None;
        }
        // Literal evaluation normally validates the key's logical byte size.
        // Keep that path, including its validation order and span, for quotas.
        if self.options.max_value_bytes.is_some() {
            return None;
        }
        let ExprKind::Variable(name) = &ungroup(base).kind else {
            return None;
        };
        // Assignment-place reads reject this root before evaluating the key.
        if name == "this" {
            return None;
        }
        let key = match &ungroup(index).kind {
            ExprKind::Literal(Literal::Int(value)) => ops::IndexKey::Int(*value),
            ExprKind::Literal(Literal::String(value)) => ops::IndexKey::String(value),
            _ => return None,
        };
        #[cfg(test)]
        {
            self.literal_reads_executed += 1;
        }
        Some((|| {
            let (scope, _, value) = self.environments.resolve_variable(name).ok_or_else(|| {
                Signal::Error(self.error(format!("undefined variable ${name}"), base.span))
            })?;
            if let Value::Map(entries) = value {
                if crate::map_index::eligible_size(entries.len()) {
                    if let Some(position) = self
                        .map_indexes
                        .borrow_mut()
                        .position_key(scope, name, entries, key, true)
                    {
                        return Ok(entries[position].1.clone());
                    }
                }
            }
            self.operation(ops::index_ref_key(value, key), span)
                .cloned()
        })())
    }
}

#[cfg(test)]
mod tests;
