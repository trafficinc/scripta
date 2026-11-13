use crate::Value;
use std::collections::HashMap;

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
    pub current: usize,
    pub live: usize,
}

impl Default for Environments {
    fn default() -> Self {
        Self {
            scopes: vec![Some(Scope::default())],
            free: Vec::new(),
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
        previous
    }
    pub fn leave(&mut self, previous: usize) {
        let id = self.current;
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
