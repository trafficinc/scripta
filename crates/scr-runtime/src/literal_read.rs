//! Literal reads borrow AST keys without constructing an assignment path.
use crate::{Expr, ExprKind, Literal, Result, Runtime, Signal, Span, Value, ops};

pub(super) fn ungroup(mut expression: &Expr) -> &Expr {
    while let ExprKind::Grouped(inner) = &expression.kind {
        expression = inner;
    }
    expression
}

fn literal_key(expression: &Expr) -> Option<ops::IndexKey<'_>> {
    match &ungroup(expression).kind {
        ExprKind::Literal(Literal::Int(value)) => Some(ops::IndexKey::Int(*value)),
        ExprKind::Literal(Literal::String(value)) => Some(ops::IndexKey::String(value)),
        _ => None,
    }
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
            return self.nested_literal_index_read(base, index, span);
        };
        // Assignment-place reads reject this root before evaluating the key.
        if name == "this" {
            return None;
        }
        let key = literal_key(index)?;
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

    fn nested_literal_index_read(
        &mut self,
        base: &Expr,
        index: &Expr,
        span: Span,
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        if self.disable_nested_literal_reads {
            return None;
        }
        let ExprKind::IndexAccess(root, first_index) = &ungroup(base).kind else {
            return None;
        };
        let ExprKind::Variable(name) = &ungroup(root).kind else {
            return None;
        };
        if name == "this" {
            return None;
        }
        let first = literal_key(first_index)?;
        let second = literal_key(index)?;
        #[cfg(test)]
        {
            self.nested_literal_reads_executed += 1;
        }
        Some((|| {
            let (scope, _, value) = self.environments.resolve_variable(name).ok_or_else(|| {
                Signal::Error(self.error(format!("undefined variable ${name}"), root.span))
            })?;
            // Ordinary place construction checks this prefix before evaluating
            // the second key. Literal keys have no checkpoints and cannot fail
            // without a byte quota (excluded by literal_index_read).
            let intermediate = if let Value::Map(entries) = value {
                let position = crate::map_index::eligible_size(entries.len())
                    .then(|| {
                        self.map_indexes
                            .borrow_mut()
                            .position_key(scope, name, entries, first, true)
                    })
                    .flatten();
                if let Some(position) = position {
                    &entries[position].1
                } else {
                    self.operation(ops::index_ref_key(value, first), base.span)?
                }
            } else {
                self.operation(ops::index_ref_key(value, first), base.span)?
            };
            self.operation(ops::index_ref_key(intermediate, second), span)
                .cloned()
        })())
    }
}

#[cfg(test)]
mod nested_tests;
#[cfg(test)]
mod tests;
