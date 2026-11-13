use crate::{Value, value::Function};
use std::{
    collections::HashMap,
    rc::{Rc, Weak},
};

#[derive(Default)]
pub(crate) struct Scope {
    pub parent: Option<usize>,
    pub variables: HashMap<String, Value>,
    pub functions: HashMap<String, Value>,
    pub captured: bool,
    pub function_boundary: bool,
}

/// Owned arena: closures refer to scope IDs, so function/environment cycles
/// cannot form Rc cycles. Transient scopes are released and their slots reused.
pub(crate) struct Environments {
    scopes: Vec<Option<Scope>>,
    free: Vec<usize>,
    // Includes suspended callers, which need not be lexical parents of current.
    active: Vec<usize>,
    functions: Vec<Weak<Function>>,
    allocations_since_collection: usize,
    collection_threshold: usize,
    pub current: usize,
    pub live: usize,
}

impl Default for Environments {
    fn default() -> Self {
        Self {
            scopes: vec![Some(Scope::default())],
            free: Vec::new(),
            active: vec![0],
            functions: Vec::new(),
            allocations_since_collection: 0,
            collection_threshold: 256,
            current: 0,
            live: 1,
        }
    }
}

impl Environments {
    pub fn scope(&self, id: usize) -> &Scope {
        self.scopes[id].as_ref().expect("live lexical scope")
    }
    pub fn scope_mut(&mut self, id: usize) -> &mut Scope {
        self.scopes[id].as_mut().expect("live lexical scope")
    }
    pub fn enter(&mut self, parent: usize) -> usize {
        self.live += 1;
        let scope = Scope {
            parent: Some(parent),
            ..Scope::default()
        };
        let id = if let Some(id) = self.free.pop() {
            self.scopes[id] = Some(scope);
            id
        } else {
            self.scopes.push(Some(scope));
            self.scopes.len() - 1
        };
        let previous = self.current;
        self.current = id;
        self.active.push(id);
        previous
    }
    pub fn leave(&mut self, previous: usize) {
        let id = self.current;
        let active = self.active.pop();
        debug_assert_eq!(active, Some(id));
        self.current = previous;
        if !self.scope(id).captured {
            self.live -= 1;
            self.scopes[id] = None;
            self.free.push(id);
        }
    }
    pub fn capture(&mut self) {
        let mut id = Some(self.current);
        while let Some(current) = id {
            let scope = self.scope_mut(current);
            scope.captured = true;
            id = scope.parent;
        }
    }
    pub fn track_function(&mut self, function: &Rc<Function>) {
        self.functions.push(Rc::downgrade(function));
        self.allocations_since_collection = self.allocations_since_collection.saturating_add(1);
    }

    pub fn collect_if_needed(&mut self) {
        if self.allocations_since_collection >= self.collection_threshold {
            self.collect();
        }
    }

    /// Trace lexical environments, preserving callable references outside the arena.
    /// The registry contains weak references only and does not keep functions alive.
    pub fn collect(&mut self) -> usize {
        let functions: Vec<_> = self.functions.iter().filter_map(Weak::upgrade).collect();
        let mut internal = HashMap::<*const Function, usize>::new();
        for scope in self.scopes.iter().flatten() {
            visit_callables(scope, |callable| {
                *internal.entry(Rc::as_ptr(callable)).or_default() += 1;
            });
        }
        // Only functions created by this arena occur in this map. Foreign-runtime
        // callables may be stored as values but must never supply our scope IDs.
        let mut environments = HashMap::new();
        let mut pending = self.active.clone();
        for function in &functions {
            if let Function::User { environment, .. } = function.as_ref() {
                let pointer = Rc::as_ptr(function);
                environments.insert(pointer, *environment);
                // Discount every arena Value edge plus our single temporary upgrade.
                // Remaining owners are evaluator temporaries, host-held Values, or
                // opaque native captures: all must conservatively protect the scope.
                if Rc::strong_count(function) > internal.get(&pointer).copied().unwrap_or(0) + 1 {
                    pending.push(*environment);
                }
            }
        }
        let mut marked = vec![false; self.scopes.len()];
        while let Some(id) = pending.pop() {
            if marked[id] {
                continue;
            }
            marked[id] = true;
            let scope = self.scope(id);
            if let Some(parent) = scope.parent {
                pending.push(parent);
            }
            visit_callables(scope, |callable| {
                if let Some(environment) = environments.get(&Rc::as_ptr(callable)) {
                    pending.push(*environment);
                }
            });
        }
        let mut reclaimed = 0;
        for (id, scope) in self.scopes.iter_mut().enumerate() {
            if !marked[id] && scope.is_some() {
                *scope = None;
                self.free.push(id);
                self.live -= 1;
                reclaimed += 1;
            }
        }
        drop(functions);
        self.functions
            .retain(|function| function.strong_count() > 0);
        self.allocations_since_collection = 0;
        // Amortize scans when the program intentionally retains many closures.
        self.collection_threshold = 256.max(self.live.saturating_add(self.functions.len()));
        reclaimed
    }

    pub fn find(&self, name: &str, function: bool) -> Option<usize> {
        let mut id = Some(self.current);
        let mut in_function = false;
        while let Some(current) = id {
            if !function && in_function && current == 0 {
                return None;
            }
            let scope = self.scope(current);
            in_function |= scope.function_boundary;
            if if function {
                scope.functions.contains_key(name)
            } else {
                scope.variables.contains_key(name)
            } {
                return Some(current);
            }
            id = scope.parent;
        }
        None
    }
}

// No user property access or native callbacks during tracing. Opaque native-held
// callables are counted as external roots instead. Traverse owned collections
// iteratively so collection itself does not add recursive stack depth.
fn visit_callables(scope: &Scope, mut visit: impl FnMut(&Rc<Function>)) {
    let mut values: Vec<_> = scope
        .variables
        .values()
        .chain(scope.functions.values())
        .collect();
    while let Some(value) = values.pop() {
        match value {
            Value::Callable(callable) => visit(&callable.0),
            Value::List(items) => values.extend(items),
            Value::Map(items) => values.extend(items.iter().map(|(_, value)| value)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Environments;

    #[test]
    fn transient_scope_slots_are_reused_across_many_batches() {
        let mut environments = Environments::default();
        for _ in 0..100 {
            for _ in 0..100 {
                let outer = environments.enter(0);
                let inner = environments.enter(environments.current);
                environments.leave(inner);
                environments.leave(outer);
            }
            assert_eq!(environments.current, 0);
            assert_eq!(environments.live, 1);
            assert_eq!(environments.scopes.len(), 3);
            assert_eq!(environments.free.len(), 2);
        }
    }
}
