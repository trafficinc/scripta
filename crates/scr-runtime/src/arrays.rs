use super::*;

impl Runtime {
    fn scalar_comprehension_items(&self, items: &[Value], span: Span) -> Result<bool> {
        for chunk in items.chunks(4096) {
            if self
                .options
                .interrupt_flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                return Err(Signal::Interrupted(
                    self.error("execution interrupted", span),
                ));
            }
            if !chunk.iter().all(crate::compact_call::Expression::scalar) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn push_array_value(
        &mut self,
        values: &mut Vec<Value>,
        value: Value,
        bytes: &mut usize,
        span: Span,
    ) -> Result<()> {
        self.checkpoint(span)?;
        let measured = self.checked(value.measure(1, self.options.max_value_bytes), span)?;
        *bytes = bytes
            .checked_add(measured)
            .ok_or_else(|| Signal::Error(self.error("collection size overflow", span)))?;
        if self
            .options
            .max_value_bytes
            .is_some_and(|limit| *bytes > limit)
        {
            return self.fail("execution value byte limit exceeded", span);
        }
        values
            .try_reserve(1)
            .map_err(|_| Signal::Error(self.error("collection allocation failed", span)))?;
        values.push(value);
        Ok(())
    }

    pub(super) fn expanded_values(
        &mut self,
        expressions: &[Expr],
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Vec<Value>> {
        let mut values = Vec::new();
        let mut bytes = std::mem::size_of::<Value>();
        for expression in expressions {
            if let ExprKind::Spread(inner) = &expression.kind {
                let value = self.evaluate(inner, output)?;
                let Value::List(items) = value else {
                    return Err(self.native_error(
                        NativeError::type_error("spread requires a list"),
                        expression.span,
                    ));
                };
                for value in items {
                    self.push_array_value(&mut values, value, &mut bytes, expression.span)?;
                }
            } else {
                let value = self.evaluate(expression, output)?;
                self.push_array_value(&mut values, value, &mut bytes, expression.span)?;
            }
        }
        if self
            .options
            .max_value_bytes
            .is_some_and(|limit| bytes > limit)
        {
            return self.fail("execution value byte limit exceeded", span);
        }
        Ok(values)
    }

    pub(super) fn evaluate_arguments(
        &mut self,
        expressions: &[Expr],
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Vec<Value>> {
        let mut values = if expressions.is_empty() {
            Vec::new()
        } else {
            self.take_argument_buffer()
        };
        for expression in expressions {
            if let ExprKind::Spread(inner) = &expression.kind {
                let Value::List(items) = self.evaluate(inner, output)? else {
                    return Err(self.native_error(
                        NativeError::type_error("argument spread requires a list"),
                        expression.span,
                    ));
                };
                for value in items {
                    self.checkpoint(span)?;
                    values.try_reserve(1).map_err(|_| {
                        Signal::Error(self.error("argument allocation failed", span))
                    })?;
                    values.push(value);
                }
            } else {
                let value = self.evaluate(expression, output)?;
                values
                    .try_reserve(1)
                    .map_err(|_| Signal::Error(self.error("argument allocation failed", span)))?;
                values.push(value);
            }
        }
        Ok(values)
    }

    pub(super) fn membership(
        &mut self,
        needle: &Value,
        value: &Value,
        span: Span,
    ) -> Result<Value> {
        let found = match value {
            Value::List(items) => {
                let mut found = false;
                for item in items {
                    self.checkpoint(span)?;
                    if item == needle {
                        found = true;
                        break;
                    }
                }
                found
            }
            Value::Map(items) => {
                let key = self.operation(
                    ops::IndexKey::from_value(needle).ok_or(ops::Error::Type(
                        "map membership requires an int or string key",
                    )),
                    span,
                )?;
                let mut found = false;
                for (candidate, _) in items {
                    self.checkpoint(span)?;
                    if key.matches(candidate) {
                        found = true;
                        break;
                    }
                }
                found
            }
            Value::String(value) => {
                let Value::String(needle) = needle else {
                    return self.operation(
                        Err(ops::Error::Type("string membership requires a string")),
                        span,
                    );
                };
                value.contains(needle)
            }
            _ => {
                return self.operation(
                    Err(ops::Error::Type(
                        "membership requires a list, map, or string",
                    )),
                    span,
                );
            }
        };
        Ok(Value::Bool(found))
    }

    pub(super) fn slice(
        &mut self,
        base: &Expr,
        start: Option<&Expr>,
        stop: Option<&Expr>,
        step: Option<&Expr>,
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let value = self.evaluate(base, output)?;
        let Value::List(items) = value else {
            return Err(self.native_error(NativeError::type_error("slicing requires a list"), span));
        };
        let mut integer = |expression: Option<&Expr>| -> Result<Option<i128>> {
            match expression {
                None => Ok(None),
                Some(expression) => match self.evaluate(expression, output)? {
                    Value::Int(value) => Ok(Some(value as i128)),
                    _ => Err(self.native_error(
                        NativeError::type_error("slice bounds and step must be ints"),
                        expression.span,
                    )),
                },
            }
        };
        let start = integer(start)?;
        let stop = integer(stop)?;
        let step = integer(step)?.unwrap_or(1);
        if step == 0 {
            return Err(self.native_error(
                NativeError::new("RangeError", "slice step cannot be zero"),
                span,
            ));
        }
        let len = items.len() as i128;
        let normalize =
            |value: i128, low, high| (if value < 0 { value + len } else { value }).clamp(low, high);
        let (mut at, end) = if step > 0 {
            (
                start.map_or(0, |v| normalize(v, 0, len)),
                stop.map_or(len, |v| normalize(v, 0, len)),
            )
        } else {
            (
                start.map_or(len - 1, |v| normalize(v, -1, len - 1)),
                stop.map_or(-1, |v| normalize(v, -1, len - 1)),
            )
        };
        let mut result = Vec::new();
        let mut bytes = std::mem::size_of::<Value>();
        while if step > 0 { at < end } else { at > end } {
            self.push_array_value(&mut result, items[at as usize].clone(), &mut bytes, span)?;
            at += step;
        }
        let result = Value::list(result);
        self.validate_value(&result, 0, span)?;
        Ok(result)
    }

    pub(super) fn comprehension(
        &mut self,
        value: &Expr,
        binding: &scr_ast::Binding,
        iterable: &Expr,
        condition: Option<&Expr>,
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let items = match self.evaluate(iterable, output)? {
            Value::List(items) => items,
            Value::Map(items) => items.into_iter().map(|(_, value)| value).collect(),
            _ => {
                return Err(self.native_error(
                    NativeError::type_error("comprehension requires a list or map"),
                    iterable.span,
                ));
            }
        };
        #[cfg(not(test))]
        let planned = true;
        #[cfg(test)]
        let planned = !self.disable_comprehension_plans;
        if planned
            && self.options.max_value_bytes.is_none()
            && !items.is_empty()
            && self.scalar_comprehension_items(&items, span)?
        {
            let plans = (|| {
                let value_plan = crate::compact_call::Expression::compile(value, &binding.name)?;
                let condition_plan = match condition {
                    Some(condition) => Some(crate::compact_call::Expression::compile(
                        condition,
                        &binding.name,
                    )?),
                    None => None,
                };
                Some((value_plan, condition_plan))
            })();
            if let Some((value, condition_plan)) = plans {
                #[cfg(test)]
                {
                    self.comprehension_plans_executed += 1;
                }
                let mut values = Vec::new();
                let mut bytes = std::mem::size_of::<Value>();
                for item in items {
                    self.checkpoint(span)?;
                    let previous = self.enter_empty_scope(self.environments.current, span)?;
                    let result = (|| {
                        if let Some(plan) = &condition_plan {
                            let allowed = plan.evaluate(self, &item)?;
                            if !self.operation(ops::boolean(allowed), condition.unwrap().span)? {
                                return Ok(());
                            }
                        }
                        let value = value.evaluate(self, &item)?;
                        self.push_array_value(&mut values, value, &mut bytes, span)
                    })();
                    self.environments.leave_empty(previous);
                    result?;
                }
                let result = Value::list(values);
                self.validate_value(&result, 0, span)?;
                return Ok(result);
            }
        }
        let mut values = Vec::new();
        let mut bytes = std::mem::size_of::<Value>();
        // Share the immutable binding name, not the per-iteration value or
        // scope identity. Captured iterations still retain independent scopes.
        let name = Rc::new(binding.name.clone());
        for item in items {
            self.checkpoint(span)?;
            let previous = self.enter_scope(self.environments.current, span)?;
            let variables = &mut self
                .environments
                .scope_mut(self.environments.current)
                .variables;
            #[cfg(not(test))]
            let shared = true;
            #[cfg(test)]
            let shared = !self.disable_shared_parameter_names;
            if shared {
                variables.insert_parameter(&name, item);
            } else {
                variables.insert(binding.name.clone(), item);
            }
            let result = (|| {
                if let Some(condition) = condition {
                    if !self.condition(condition, output)? {
                        return Ok(());
                    }
                }
                let value = self.evaluate(value, output)?;
                self.push_array_value(&mut values, value, &mut bytes, span)
            })();
            self.environments.leave_parameter(previous);
            result?;
        }
        let value = Value::list(values);
        self.validate_value(&value, 0, span)?;
        Ok(value)
    }

    pub(super) fn destructure(
        &mut self,
        pattern: &[Expr],
        op: AssignmentOp,
        rhs: &Expr,
        retain: bool,
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        if op != AssignmentOp::Assign {
            return self.fail("destructuring requires plain assignment", span);
        }
        let mut names = std::collections::HashSet::new();
        let mut bindings = Vec::new();
        let mut rest = false;
        for (index, expression) in pattern.iter().enumerate() {
            let target = if let ExprKind::Spread(inner) = &expression.kind {
                if index + 1 != pattern.len() {
                    return self.fail("destructuring rest must be last", expression.span);
                }
                rest = true;
                inner.as_ref()
            } else {
                expression
            };
            let ExprKind::Variable(name) = &target.kind else {
                return self.fail("destructuring targets must be variables", target.span);
            };
            if name == "this" || !names.insert(name) {
                return self.fail("invalid or duplicate destructuring binding", target.span);
            }
            bindings.push((
                self.environments
                    .find(name, false)
                    .unwrap_or(self.environments.current),
                name.clone(),
            ));
        }
        let value = self.evaluate(rhs, output)?;
        let Value::List(items) = &value else {
            return Err(self.native_error(
                NativeError::type_error("destructuring requires a list"),
                rhs.span,
            ));
        };
        let fixed = pattern.len() - usize::from(rest);
        if items.len() < fixed || (!rest && items.len() != fixed) {
            return Err(self.native_error(
                NativeError::new("RangeError", "destructuring length mismatch"),
                span,
            ));
        }
        let mut replacements = items[..fixed].to_vec();
        if rest {
            replacements.push(Value::list(items[fixed..].to_vec()));
        }
        for replacement in &replacements {
            self.validate_value(replacement, 0, span)?;
        }
        for ((scope, name), replacement) in bindings.into_iter().zip(replacements) {
            self.map_indexes.get_mut().invalidate(scope, &name);
            self.environments
                .scope_mut(scope)
                .variables
                .insert(name, replacement);
        }
        Ok(if retain { value } else { Value::Null })
    }
}
