use crate::{Value, value::Function};
use std::{
    collections::HashMap,
    rc::{Rc, Weak},
};

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy)]
struct CollectionWork {
    arena_slots: usize,
    registry_entries: usize,
    scopes_scanned: usize,
    scopes_marked: usize,
    values_visited: usize,
}

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
    #[cfg(test)]
    last_collection: CollectionWork,
    #[cfg(test)]
    collections: usize,
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
            #[cfg(test)]
            last_collection: CollectionWork::default(),
            #[cfg(test)]
            collections: 0,
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
        #[cfg(test)]
        let mut work = CollectionWork {
            arena_slots: self.scopes.len(),
            registry_entries: self.functions.len(),
            ..Default::default()
        };
        let functions: Vec<_> = self.functions.iter().filter_map(Weak::upgrade).collect();
        let mut internal = HashMap::<*const Function, usize>::new();
        for scope in self.scopes.iter().flatten() {
            #[cfg(test)]
            {
                work.scopes_scanned += 1;
            }
            visit_values(scope, |value| {
                #[cfg(test)]
                {
                    work.values_visited += 1;
                }
                if let Value::Callable(callable) = value {
                    *internal.entry(Rc::as_ptr(&callable.0)).or_default() += 1;
                }
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
            #[cfg(test)]
            {
                work.scopes_marked += 1;
            }
            let scope = self.scope(id);
            if let Some(parent) = scope.parent {
                pending.push(parent);
            }
            visit_values(scope, |value| {
                #[cfg(test)]
                {
                    work.values_visited += 1;
                }
                if let Value::Callable(callable) = value {
                    if let Some(environment) = environments.get(&Rc::as_ptr(&callable.0)) {
                        pending.push(*environment);
                    }
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
        #[cfg(test)]
        {
            self.last_collection = work;
            self.collections += 1;
        }
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
fn visit_values(scope: &Scope, mut visit: impl FnMut(&Value)) {
    let mut values: Vec<_> = scope
        .variables
        .values()
        .chain(scope.functions.values())
        .collect();
    while let Some(value) = values.pop() {
        visit(value);
        match value {
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

    fn execute(runtime: &mut crate::Runtime, body: &str) {
        runtime
            .execute_source(&format!("<?scr {body}"), &mut std::io::sink())
            .unwrap();
    }

    fn fixture(size: usize, nested: bool) -> crate::Runtime {
        let mut runtime = crate::Runtime::new();
        let value = if nested {
            "[\"wrapped\" => [[factory($i)]]]"
        } else {
            "factory($i)"
        };
        execute(
            &mut runtime,
            &format!(
                "function factory($n) {{ $f=function ($depth) {{ if ($depth===0) return $n; return $f($depth-1); }}; return $f; }} \
             $kept=[]; for ($i=0; $i<{size}; $i+=1) {{ $kept[]={value}; }}"
            ),
        );
        runtime
    }

    #[test]
    fn collection_work_scales_linearly_for_live_and_garbage_cycles() {
        for nested in [false, true] {
            let mut previous = None;
            for size in [64, 128, 256, 512] {
                let mut runtime = fixture(size, nested);
                assert_eq!(runtime.collect_garbage(), 0);
                let live = runtime.environments.last_collection;
                assert_eq!(live.scopes_scanned, 1 + 2 * size);
                assert_eq!(live.scopes_marked, live.scopes_scanned);
                assert_eq!(live.registry_entries, 1 + size);
                // Includes arena high-water slots, nested values, and weak registry
                // entries. This bounds work rather than noisy elapsed time.
                let total = live.arena_slots
                    + live.registry_entries
                    + live.scopes_scanned
                    + live.scopes_marked
                    + live.values_visited;
                assert!(total < 40 * size + 32, "{live:?}");
                if let Some(previous) = previous {
                    assert!(total <= 2 * previous, "superlinear work");
                }
                previous = Some(total);
                runtime.set_global("kept", crate::Value::Null).unwrap();
                assert_eq!(runtime.collect_garbage(), 2 * size);
                let garbage = runtime.environments.last_collection;
                assert_eq!(garbage.scopes_scanned, 1 + 2 * size);
                assert_eq!(garbage.scopes_marked, 1);
                assert!(garbage.values_visited < live.values_visited);
                assert_eq!(runtime.environments.functions.len(), 1);
                assert_eq!(runtime.collect_garbage(), 0);
                assert_eq!(runtime.environments.last_collection.scopes_scanned, 1);
                assert_eq!(runtime.environments.last_collection.registry_entries, 1);
            }
        }
    }

    #[test]
    fn shared_capture_aliases_mark_each_environment_only_once() {
        let mut runtime = fixture(1, false);
        let closure = match runtime.get_global("kept").unwrap() {
            crate::Value::List(values) => values[0].clone(),
            _ => unreachable!(),
        };
        runtime
            .set_global("kept", crate::Value::List(vec![closure; 10000]))
            .unwrap();
        assert_eq!(runtime.collect_garbage(), 0);
        let work = runtime.environments.last_collection;
        assert_eq!(work.scopes_marked, 3);
        assert_eq!(work.registry_entries, 2);
        assert!(
            work.values_visited < 20100,
            "shared graph repeatedly expanded: {work:?}"
        );
        runtime.set_global("kept", crate::Value::Null).unwrap();
        assert_eq!(runtime.collect_garbage(), 2);
    }

    #[test]
    fn repeated_churn_bounds_arena_and_registry_and_reuses_slots() {
        let mut runtime = crate::Runtime::new();
        execute(
            &mut runtime,
            "function factory() { $f=function () { return $f; }; return $f; }",
        );
        let mut high_water = 0;
        for batch in 0..20 {
            execute(
                &mut runtime,
                "for ($i=0; $i<1000; $i+=1) { $f=factory(); $f=null; }",
            );
            let arena = &runtime.environments;
            assert_eq!(arena.live, 1);
            assert_eq!(arena.functions.len(), 1);
            assert!(arena.scopes.len() < 600, "unbounded scope storage");
            assert!(arena.scopes.capacity() <= 1024, "unbounded arena capacity");
            assert!(
                arena.functions.capacity() <= 1024,
                "unbounded weak registry capacity"
            );
            assert!(
                arena.free.capacity() <= 1024,
                "unbounded free-list capacity"
            );
            if batch == 0 {
                high_water = arena.scopes.len();
            }
            assert_eq!(arena.scopes.len(), high_water);
        }
    }

    #[test]
    fn scheduling_avoids_scans_at_every_checkpoint_and_adapts_to_live_graphs() {
        let mut runtime = crate::Runtime::new();
        execute(
            &mut runtime,
            "function factory() { return function () {}; }",
        );
        let before = runtime.environments.collections;
        execute(&mut runtime, "$x=0; for ($i=0; $i<10000; $i+=1) { $x+=1; }");
        assert_eq!(
            runtime.environments.collections - before,
            2,
            "ordinary execution should collect only at boundaries"
        );
        let before = runtime.environments.collections;
        execute(
            &mut runtime,
            "$kept=[]; for ($i=0; $i<4096; $i+=1) { $kept[]=factory(); }",
        );
        let collections = runtime.environments.collections - before;
        assert!(
            (3..=10).contains(&collections),
            "unamortized collection scheduling: {collections}"
        );
        assert!(runtime.environments.collection_threshold >= 4096);
        runtime.set_global("kept", crate::Value::Null).unwrap();
        runtime.collect_garbage();
        assert_eq!(runtime.environments.collection_threshold, 256);
    }
}
