//! Closed scalar call graphs execute without recursively entering the evaluator.
//! Graph bindings are resolved anew at entry, before any instructions execute.
use super::*;
use crate::{Callable, MAX_RECURSION_LIMIT, Signal, error::ActiveFrame, value::Function};
use std::{rc::Rc, sync::Arc};

struct Node {
    declaration: Rc<UserDeclaration>,
    source: Option<Arc<crate::SourceFile>>,
    targets: Vec<usize>,
}
impl Node {
    fn from_function(function: &Function, identity: &Rc<()>) -> Option<Self> {
        let Function::User {
            declaration,
            environment: 0,
            receiver: None,
            owner,
            source,
        } = function
        else {
            return None;
        };
        if !Rc::ptr_eq(owner, identity) {
            return None;
        }
        declaration
            .compact_call
            .get_or_init(|| Plan::compile(declaration))
            .as_ref()?;
        // Existing specialized recursive plans retain precedence, including
        // their first cold invocation and calls through another scalar body.
        if !declaration
            .compact_call
            .get()
            .unwrap()
            .as_ref()
            .unwrap()
            .calls
            .is_empty()
            && declaration
                .prepared_call
                .get_or_init(|| crate::prepared_call::Plan::compile(declaration))
                .is_some()
        {
            return None;
        }
        Some(Self {
            declaration: declaration.clone(),
            source: source.clone(),
            targets: Vec::new(),
        })
    }
    fn plan(&self) -> &Plan {
        self.declaration
            .compact_call
            .get()
            .unwrap()
            .as_deref()
            .unwrap()
    }
    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.declaration, &other.declaration)
            && match (&self.source, &other.source) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}
struct Frame {
    node: usize,
    ip: usize,
    base: usize,
    scopes: usize,
}
struct Scope {
    previous: usize,
    parameter: bool,
}

#[derive(Default)]
pub(crate) struct Stack {
    values: Vec<Scalar>,
    frames: Vec<Frame>,
    scopes: Vec<Scope>,
}

#[cfg(test)]
impl Stack {
    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty() && self.frames.is_empty() && self.scopes.is_empty()
    }
}
fn unwind(runtime: &mut Runtime, scopes: &mut Vec<Scope>, depth: usize) {
    while scopes.len() > depth {
        let scope = scopes.pop().unwrap();
        if scope.parameter {
            runtime
                .environments
                .scope_mut(runtime.environments.current)
                .function_boundary = false;
            runtime.environments.leave_parameter(scope.previous);
        } else {
            runtime.environments.leave_block(scope.previous);
        }
    }
}

fn repeats(nodes: &[Node]) -> bool {
    if nodes.iter().any(|node| node.plan().repeats) {
        return true;
    }
    // A bounded topological pass identifies mutual/self recursion without a
    // recursive graph walk or additional allocations.
    let mut incoming = [0usize; MAX_RECURSION_LIMIT];
    for node in nodes {
        for &target in &node.targets {
            incoming[target] += 1;
        }
    }
    let mut queue = [0usize; MAX_RECURSION_LIMIT];
    let mut end = 0;
    for (index, &count) in incoming[..nodes.len()].iter().enumerate() {
        if count == 0 {
            queue[end] = index;
            end += 1;
        }
    }
    let mut start = 0;
    while start < end {
        let index = queue[start];
        start += 1;
        for &target in &nodes[index].targets {
            incoming[target] -= 1;
            if incoming[target] == 0 {
                queue[end] = target;
                end += 1;
            }
        }
    }
    end < nodes.len()
}

impl Runtime {
    fn compact_graph(&self, function: &Function) -> Option<Vec<Node>> {
        let mut nodes = Vec::new();
        nodes.try_reserve(1).ok()?;
        nodes.push(Node::from_function(function, &self.identity)?);
        let mut index = 0;
        while index < nodes.len() {
            let count = nodes[index].plan().calls.len();
            nodes[index].targets.try_reserve_exact(count).ok()?;
            for site in 0..count {
                let name = &nodes[index].plan().calls[site].name;
                // Every admitted body is noncapturing and contains no native
                // callbacks, declarations or loads that could rebind this name.
                let Value::Callable(Callable(callee)) =
                    self.environments.scope(0).functions.get(name)?
                else {
                    return None;
                };
                let target = Node::from_function(callee, &self.identity)?;
                let target = if let Some(found) = nodes.iter().position(|n| n.same(&target)) {
                    found
                } else {
                    if nodes.len() == MAX_RECURSION_LIMIT {
                        return None;
                    }
                    nodes.try_reserve(1).ok()?;
                    nodes.push(target);
                    nodes.len() - 1
                };
                nodes[index].targets.push(target);
            }
            index += 1;
        }
        Some(nodes)
    }

    pub(super) fn compact_frames(
        &mut self,
        function: &Function,
        arguments: &[Value],
    ) -> Option<Result<Value>> {
        #[cfg(test)]
        let force = self.force_compact_frames;
        #[cfg(not(test))]
        let force = false;
        // Rejection never executes a prefix or consumes an instruction budget.
        let nodes = match self.compact_graph(function) {
            Some(nodes) => nodes,
            None => {
                // Unsupported graphs also stay off the warm speculative path.
                // The ordinary evaluator still resolves every current binding.
                if !force {
                    if let Function::User { declaration, .. } = function {
                        if let Some(Some(plan)) = declaration.compact_call.get() {
                            plan.frames_disabled.set(true);
                        }
                    }
                }
                return None;
            }
        };
        if !force && !repeats(&nodes) {
            // This is only an optimization opt-out. Future function rebinding
            // remains correct on the ordinary evaluator, even if a later graph
            // could have benefited from VM frames.
            nodes[0].plan().frames_disabled.set(true);
            return None;
        }
        #[cfg(test)]
        {
            self.compact_calls_executed += 1;
        }
        Some(self.run_compact_frames(&nodes, arguments))
    }

    #[inline(never)]
    fn run_compact_frames(&mut self, nodes: &[Node], arguments: &[Value]) -> Result<Value> {
        let initial_depth = self.stack.len();
        let initial_source = self.source.clone();
        let mut storage = self.compact_stack.take().unwrap_or_default();
        let values = &mut storage.values;
        let frames = &mut storage.frames;
        let scopes = &mut storage.scopes;
        // Compiled scalar bodies cannot capture locals or reenter arbitrary
        // code. Their environments are unobservable unless scopes are budgeted.
        let lexical_scopes = self.options.max_scopes.is_some();
        let result = (|| {
            let depth = self
                .options
                .recursion_limit
                .min(MAX_RECURSION_LIMIT)
                .saturating_sub(initial_depth.saturating_sub(1))
                + 1;
            values.try_reserve((LOCALS + STACK) * depth).map_err(|_| {
                Signal::Error(self.error(
                    "compact value stack allocation failed",
                    nodes[0].declaration.name_span,
                ))
            })?;
            frames.try_reserve(depth).map_err(|_| {
                Signal::Error(self.error(
                    "compact call stack allocation failed",
                    nodes[0].declaration.name_span,
                ))
            })?;
            if lexical_scopes {
                scopes.try_reserve((DEPTH + 3) * depth).map_err(|_| {
                    Signal::Error(self.error(
                        "compact scope stack allocation failed",
                        nodes[0].declaration.name_span,
                    ))
                })?;
            }
            self.stack.try_reserve(depth - 1).map_err(|_| {
                Signal::Error(self.error(
                    "compact trace stack allocation failed",
                    nodes[0].declaration.name_span,
                ))
            })?;
            values.resize(LOCALS, Scalar::Null);
            for (local, value) in values.iter_mut().zip(arguments) {
                *local = Scalar::from_value(value).unwrap();
            }
            for (parameter, value) in nodes[0].declaration.parameters.iter().zip(arguments) {
                self.annotation(&parameter.annotation, value, parameter.span, false)?;
            }
            frames.push(Frame {
                node: 0,
                ip: 0,
                base: 0,
                scopes: 0,
            });
            loop {
                let frame = frames.last_mut().unwrap();
                let node = &nodes[frame.node];
                let plan = node.plan();
                let Instruction { code, operand } = plan.code[frame.ip];
                let span = plan.spans[frame.ip];
                frame.ip += 1;
                let operand = operand as usize;
                let base = frame.base;
                match code {
                    Code::Tick => self.checkpoint(span)?,
                    Code::Enter if lexical_scopes => {
                        let previous = self.enter_empty_scope(self.environments.current, span)?;
                        scopes.push(Scope {
                            previous,
                            parameter: false,
                        });
                    }
                    Code::Leave if lexical_scopes => {
                        let depth = scopes.len() - 1;
                        unwind(self, scopes, depth);
                    }
                    Code::Enter | Code::Leave => {}
                    Code::Constant => values.push(plan.constants[operand]),
                    Code::Local => values.push(values[base + operand]),
                    Code::Store => values[base + operand] = *values.last().unwrap(),
                    Code::Drop => {
                        values.pop().unwrap();
                    }
                    Code::Unary(op) => {
                        let value = values.pop().unwrap();
                        values.push(
                            Scalar::from_value(
                                &self.operation(ops::unary(op, value.value()), span)?,
                            )
                            .unwrap(),
                        );
                    }
                    Code::Binary(op) => {
                        let right = values.pop().unwrap();
                        let left = values.pop().unwrap();
                        values.push(
                            Scalar::from_value(
                                &self.operation(
                                    ops::binary(op, left.value(), right.value()),
                                    span,
                                )?,
                            )
                            .unwrap(),
                        );
                    }
                    Code::Boolean => {
                        let value = values.pop().unwrap();
                        values.push(Scalar::Bool(
                            self.operation(ops::boolean(value.value()), span)?,
                        ));
                    }
                    Code::Jump => frame.ip = operand,
                    Code::False => {
                        let value = values.pop().unwrap();
                        if !self.operation(ops::boolean(value.value()), span)? {
                            frame.ip = operand;
                        }
                    }
                    Code::Short(stop) => {
                        let value =
                            self.operation(ops::boolean(values.last().unwrap().value()), span)?;
                        if value == stop {
                            *values.last_mut().unwrap() = Scalar::Bool(value);
                            frame.ip = operand;
                        } else {
                            values.pop();
                        }
                    }
                    Code::Call => {
                        let target = &nodes[node.targets[operand]];
                        let count = plan.calls[operand].arguments;
                        // Match ordinary calls: evaluate every argument first,
                        // then check depth, create the trace frame, check arity,
                        // enter the parameter scope and validate annotations.
                        if self.stack.len().saturating_sub(1)
                            >= self.options.recursion_limit.min(MAX_RECURSION_LIMIT)
                        {
                            return self.fail("maximum call depth exceeded", span);
                        }
                        self.stack.push(ActiveFrame {
                            name: crate::error::FrameName::User(target.declaration.clone()),
                            span,
                            source: self.source.clone(),
                        });
                        if count != target.declaration.parameters.len() {
                            return Err(Signal::TypeError(self.error(
                                format!(
                                    "TypeError: {}() expects {} arguments, got {}",
                                    target.declaration.name,
                                    target.declaration.parameters.len(),
                                    count
                                ),
                                span,
                            )));
                        }
                        self.source = target.source.clone();
                        let scope_base = scopes.len();
                        if lexical_scopes {
                            let previous = self.enter_scope(0, span)?;
                            self.environments
                                .scope_mut(self.environments.current)
                                .function_boundary = true;
                            scopes.push(Scope {
                                previous,
                                parameter: true,
                            });
                        }
                        let argument_base = values.len() - count;
                        for (parameter, value) in target
                            .declaration
                            .parameters
                            .iter()
                            .zip(&values[argument_base..])
                        {
                            self.annotation(
                                &parameter.annotation,
                                &value.value(),
                                parameter.span,
                                false,
                            )?;
                        }
                        let target_index = node.targets[operand];
                        values.resize(argument_base + LOCALS, Scalar::Null);
                        frames.push(Frame {
                            node: target_index,
                            ip: 0,
                            base: argument_base,
                            scopes: scope_base,
                        });
                        #[cfg(test)]
                        {
                            self.compact_frame_calls_executed += 1;
                        }
                    }
                    Code::Return => {
                        let value = values.pop().unwrap();
                        let frame = frames.last().unwrap();
                        let nested = frames.len() > 1;
                        if lexical_scopes {
                            unwind(self, scopes, frame.scopes + usize::from(nested));
                        }
                        self.annotation(
                            &node.declaration.return_type,
                            &value.value(),
                            node.declaration.name_span,
                            true,
                        )?;
                        if lexical_scopes {
                            unwind(self, scopes, frame.scopes);
                        }
                        values.truncate(frame.base);
                        frames.pop();
                        if let Some(caller) = frames.last() {
                            self.stack.pop();
                            self.source = nodes[caller.node].source.clone();
                            values.push(value);
                        } else {
                            return Ok(value.value());
                        }
                    }
                }
            }
        })();
        unwind(self, scopes, 0);
        self.stack.truncate(initial_depth);
        self.source = initial_source;
        values.clear();
        frames.clear();
        self.compact_stack = Some(storage);
        result
    }
}

#[cfg(test)]
mod flat_stack_tests {
    use super::*;

    #[test]
    fn mutual_recursion_and_locals_reuse_slots_without_recursive_environments() {
        let mut runtime = Runtime::new();
        let source = "function even($n){$copy=$n;if($copy===0){return true;}return odd($copy-1);}function odd($n){$copy=$n;if($copy===0){return false;}return even($copy-1);}even(2);";
        runtime.execute_source(source, &mut Vec::new()).unwrap();
        let values = runtime.compact_stack.as_ref().unwrap().values.as_ptr();
        let capacity = runtime.compact_stack.as_ref().unwrap().values.capacity();
        let frames = runtime.compact_stack.as_ref().unwrap().frames.capacity();
        let trace = runtime.stack.capacity();
        let entries = runtime.environments.scope_entries;
        runtime
            .execute_source("$answer=even(60);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("answer"), Some(&Value::Bool(true)));
        assert_eq!(runtime.environments.scope_entries - entries, 1);
        assert_eq!(
            runtime.compact_stack.as_ref().unwrap().values.as_ptr(),
            values
        );
        assert_eq!(
            runtime.compact_stack.as_ref().unwrap().values.capacity(),
            capacity
        );
        assert_eq!(
            runtime.compact_stack.as_ref().unwrap().frames.capacity(),
            frames
        );
        assert_eq!(runtime.stack.capacity(), trace);
        assert!(runtime.compact_stack.as_ref().unwrap().is_empty());
        // A later budgeted entry still uses lexical accounting and unwinds
        // completely before switching back to unbudgeted flat execution.
        runtime.options.max_scopes = Some(5);
        assert!(
            runtime
                .execute_source("$answer=even(60);", &mut Vec::new())
                .is_err()
        );
        assert!(runtime.compact_stack.as_ref().unwrap().is_empty());
        assert_eq!(runtime.environments.live, 1);
        assert_eq!(runtime.get_global("answer"), Some(&Value::Bool(true)));
        runtime.options.max_scopes = None;
        runtime
            .execute_source("$answer=odd(60);", &mut Vec::new())
            .unwrap();
        assert_eq!(runtime.get_global("answer"), Some(&Value::Bool(false)));
        assert!(runtime.compact_stack.as_ref().unwrap().is_empty());
    }
}
