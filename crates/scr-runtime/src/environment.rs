use crate::{
    NativeObject, Value,
    objects::{ScriptClass, ScriptObject},
    value::Function,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
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

#[derive(Clone)]
pub(crate) struct ModuleAlias {
    pub path: PathBuf,
    pub scope: usize,
}

#[derive(Default)]
pub(crate) struct Scope {
    pub parent: Option<usize>,
    pub variables: crate::bindings::Bindings,
    pub functions: HashMap<String, Value>,
    pub classes: HashMap<String, Rc<ScriptClass>>,
    pub aliases: HashMap<String, ModuleAlias>,
    // require_once identity is local to the enclosing isolated module or globals.
    pub loaded_modules: HashSet<PathBuf>,
    pub captured: bool,
    pub function_boundary: bool,
}

// Empty scope storage is a best-effort runtime-local optimization. Bound both
// the number of spares and their aggregate map/set capacity per spare. Names,
// values, module paths, class identities, and capture flags are never retained.
const MAX_SPARE_SCOPES: usize = 64;
const MAX_SPARE_SCOPE_SLOTS: usize = 32;

/// Owned arena: closures refer to scope IDs, so function/environment cycles
/// cannot form Rc cycles. Transient scopes are released and their slots reused.
pub(crate) struct Environments {
    scopes: Vec<Option<Scope>>,
    #[cfg(test)]
    force_hash: bool,
    free: Vec<usize>,
    spare_scopes: Vec<Scope>,
    // Includes suspended callers, which need not be lexical parents of current.
    active: Vec<usize>,
    functions: Vec<Weak<Function>>,
    objects: Vec<Weak<dyn NativeObject>>,
    allocations_since_collection: usize,
    collection_threshold: usize,
    #[cfg(test)]
    last_collection: CollectionWork,
    #[cfg(test)]
    collections: usize,
    // Lexical owners of direct methods awaiting argument evaluation/call entry.
    pub temporary_roots: Vec<usize>,
    pub current: usize,
    pub live: usize,
    // Successful isolated module instances are roots for this runtime's lifetime.
    pub module_cache: HashMap<PathBuf, usize>,
}

impl Default for Environments {
    fn default() -> Self {
        Self {
            scopes: vec![Some(Scope::default())],
            #[cfg(test)]
            force_hash: false,
            free: Vec::new(),
            spare_scopes: Vec::new(),
            active: vec![0],
            functions: Vec::new(),
            objects: Vec::new(),
            allocations_since_collection: 0,
            collection_threshold: 256,
            #[cfg(test)]
            last_collection: CollectionWork::default(),
            #[cfg(test)]
            collections: 0,
            temporary_roots: Vec::new(),
            current: 0,
            live: 1,
            module_cache: HashMap::new(),
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
        let mut scope = self.spare_scopes.pop().unwrap_or_default();
        #[cfg(test)]
        if self.force_hash {
            scope.variables.force_hash();
        }
        scope.parent = Some(parent);
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
            let scope = self.scopes[id].take().expect("live lexical scope");
            Self::recycle_scope(&mut self.spare_scopes, scope);
            self.free.push(id);
        }
    }
    fn recycle_scope(spares: &mut Vec<Scope>, mut scope: Scope) {
        let slots = [
            scope.variables.capacity(),
            scope.functions.capacity(),
            scope.classes.capacity(),
            scope.aliases.capacity(),
            scope.loaded_modules.capacity(),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add);
        if spares.len() >= MAX_SPARE_SCOPES
            || slots > MAX_SPARE_SCOPE_SLOTS
            || spares.try_reserve(1).is_err()
        {
            return;
        }
        // Release all graph edges at the same lifetime boundary as dropping the
        // original scope. Only empty map allocations and their hash seeds survive.
        scope.variables.clear();
        scope.functions.clear();
        scope.classes.clear();
        scope.aliases.clear();
        scope.loaded_modules.clear();
        scope.parent = None;
        scope.captured = false;
        scope.function_boundary = false;
        spares.push(scope);
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

    pub fn track_object(&mut self, object: &Rc<dyn NativeObject>) {
        self.objects.push(Rc::downgrade(object));
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
        let objects: Vec<_> = self.objects.iter().filter_map(Weak::upgrade).collect();
        let mut internal_objects = HashMap::<*const (), usize>::new();
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
                if let Value::NativeObject(object) = value {
                    *internal_objects
                        .entry(Rc::as_ptr(object) as *const ())
                        .or_default() += 1;
                }
                if let Value::Callable(callable) = value {
                    *internal.entry(Rc::as_ptr(&callable.0)).or_default() += 1;
                }
            });
        }
        // Only functions created by this arena occur in this map. Foreign-runtime
        // callables may be stored as values but must never supply our scope IDs.
        for function in &functions {
            if let Function::User {
                receiver: Some(Value::NativeObject(object)),
                ..
            } = function.as_ref()
            {
                *internal_objects
                    .entry(Rc::as_ptr(object) as *const ())
                    .or_default() += 1;
            }
        }
        let mut object_environments = HashMap::new();
        let mut environments = HashMap::new();
        let mut pending = self.active.clone();
        pending.extend(self.temporary_roots.iter().copied());
        pending.extend(self.module_cache.values().copied());
        for object in &objects {
            let instance = object
                .as_any()
                .and_then(|o| o.downcast_ref::<ScriptObject>())
                .expect("script object registry");
            let pointer = Rc::as_ptr(object) as *const ();
            object_environments.insert(pointer, (instance.scope, instance.class.environment));
            if Rc::strong_count(object) > internal_objects.get(&pointer).copied().unwrap_or(0) + 1 {
                pending.extend([instance.scope, instance.class.environment]);
            }
        }
        let mut receivers = HashMap::new();
        for function in &functions {
            if let Function::User {
                receiver: Some(Value::NativeObject(object)),
                ..
            } = function.as_ref()
            {
                if let Some(scopes) = object_environments.get(&(Rc::as_ptr(object) as *const ())) {
                    receivers.insert(Rc::as_ptr(function), *scopes);
                }
            }
        }
        for function in &functions {
            if let Function::User { environment, .. } = function.as_ref() {
                let pointer = Rc::as_ptr(function);
                environments.insert(pointer, *environment);
                // Discount every arena Value edge plus our single temporary upgrade.
                // Remaining owners are evaluator temporaries, host-held Values, or
                // opaque native captures: all must conservatively protect the scope.
                if Rc::strong_count(function) > internal.get(&pointer).copied().unwrap_or(0) + 1 {
                    pending.push(*environment);
                    if let Some((object, class)) = receivers.get(&pointer) {
                        pending.extend([*object, *class]);
                    }
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
            pending.extend(scope.aliases.values().map(|alias| alias.scope));
            pending.extend(scope.classes.values().map(|class| class.environment));
            for class in scope.classes.values() {
                let mut parent = class.parent.get().and_then(Option::as_ref);
                while let Some(base) = parent {
                    pending.push(base.environment);
                    parent = base.parent.get().and_then(Option::as_ref);
                }
            }
            pending.extend(scope.classes.values().flat_map(|class| {
                class
                    .interfaces
                    .get()
                    .into_iter()
                    .flatten()
                    .map(|interface| interface.environment)
            }));
            if let Some(parent) = scope.parent {
                pending.push(parent);
            }
            visit_values(scope, |value| {
                #[cfg(test)]
                {
                    work.values_visited += 1;
                }
                if let Value::NativeObject(object) = value {
                    if let Some((object, class)) =
                        object_environments.get(&(Rc::as_ptr(object) as *const ()))
                    {
                        pending.extend([*object, *class]);
                    }
                }
                if let Value::Callable(callable) = value {
                    if let Some((object, class)) = receivers.get(&Rc::as_ptr(&callable.0)) {
                        pending.extend([*object, *class]);
                    }
                    if let Some(environment) = environments.get(&Rc::as_ptr(&callable.0)) {
                        pending.push(*environment);
                    }
                }
            });
        }
        let mut reclaimed = 0;
        for (id, scope) in self.scopes.iter_mut().enumerate() {
            if !marked[id] && scope.is_some() {
                Self::recycle_scope(
                    &mut self.spare_scopes,
                    scope.take().expect("unmarked live scope"),
                );
                self.free.push(id);
                self.live -= 1;
                reclaimed += 1;
            }
        }
        drop(functions);
        drop(objects);
        self.objects.retain(|object| object.strong_count() > 0);
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

    pub fn find_alias(&self, name: &str) -> Option<&ModuleAlias> {
        let mut id = Some(self.current);
        while let Some(current) = id {
            let scope = self.scope(current);
            if let Some(alias) = scope.aliases.get(name) {
                return Some(alias);
            }
            id = scope.parent;
        }
        None
    }

    pub fn module_root(&self) -> usize {
        let mut id = self.current;
        while let Some(parent) = self.scope(id).parent {
            id = parent;
        }
        id
    }

    pub fn has_alias(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .flatten()
            .any(|scope| scope.aliases.contains_key(name))
    }

    #[cfg(test)]
    pub fn force_hash_bindings(&mut self) {
        self.force_hash = true;
        for scope in self.scopes.iter_mut().flatten() {
            scope.variables.force_hash();
        }
    }

    pub fn resolve_variable(&self, name: &str) -> Option<(usize, usize, &Value)> {
        let mut id = Some(self.current);
        let mut in_function = false;
        while let Some(current) = id {
            if in_function && current == 0 {
                return None;
            }
            let scope = self.scope(current);
            in_function |= scope.function_boundary;
            if let Some((slot, value)) = scope.variables.resolve(name) {
                return Some((current, slot, value));
            }
            id = scope.parent;
        }
        None
    }

    pub fn find(&self, name: &str, function: bool) -> Option<usize> {
        self.lookup(name, function).map(|(scope, _)| scope)
    }

    // Every lookup observes the current lexical stores; no cross-execution hints.
    pub fn lookup(&self, name: &str, function: bool) -> Option<(usize, &Value)> {
        if !function {
            return self
                .resolve_variable(name)
                .map(|(scope, _, value)| (scope, value));
        }
        let mut id = Some(self.current);
        while let Some(current) = id {
            let scope = self.scope(current);
            let value = scope.functions.get(name);
            if let Some(value) = value {
                return Some((current, value));
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

    #[test]
    fn direct_method_calls_avoid_temporary_callable_collection_pressure() {
        let mut measurements = Vec::new();
        for callee in ["$counter->next", "($counter->next)"] {
            let mut runtime = crate::Runtime::new();
            execute(
                &mut runtime,
                "class Counter { $value = 0; function next(): int { $this->value += 1; return $this->value; } } $counter = new Counter();",
            );
            let before = runtime.environments.collections;
            execute(
                &mut runtime,
                &format!(
                    "$total = 0; for ($i = 0; $i < 20000; $i += 1) {{ $total += {callee}(); }}"
                ),
            );
            let collections = runtime.environments.collections - before;
            assert_eq!(
                runtime.get_global("total"),
                Some(&crate::Value::Int(200010000))
            );
            assert_eq!(runtime.environments.live, 2);
            assert!(runtime.environments.functions.is_empty());
            assert!(runtime.environments.temporary_roots.is_empty());
            eprintln!("{callee}: {collections} collections for 20000 calls");
            measurements.push(collections);
        }
        assert_eq!(
            measurements[0], 2,
            "direct calls should collect only at execute boundaries"
        );
        assert!(measurements[1] > measurements[0]);
    }

    #[test]
    fn spare_scope_storage_drops_values_and_all_binding_namespaces() {
        use crate::{NativeFunction, NativeSignature, NativeType, Runtime, Value};
        use std::{path::PathBuf, rc::Rc};
        let mut runtime = Runtime::new();
        execute(&mut runtime, "class Local {}");
        let class = runtime
            .environments
            .scope_mut(0)
            .classes
            .remove("Local")
            .unwrap();
        let class_weak = Rc::downgrade(&class);
        let callable = Value::native_function(
            "local",
            NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
                Ok(Value::Null)
            }),
        );
        let Value::Callable(ref function) = callable else {
            unreachable!()
        };
        let function_weak = Rc::downgrade(&function.0);
        let previous = runtime.environments.enter(0);
        let scope = runtime.environments.scope_mut(runtime.environments.current);
        scope
            .variables
            .insert("old".into(), Value::String("payload".into()));
        scope.functions.insert("local".into(), callable);
        scope.classes.insert("Local".into(), class);
        scope.aliases.insert(
            "Alias".into(),
            super::ModuleAlias {
                path: PathBuf::from("old.scripta"),
                scope: 0,
            },
        );
        scope.loaded_modules.insert(PathBuf::from("old.scripta"));
        scope.function_boundary = true;
        runtime.environments.leave(previous);
        assert_eq!(runtime.environments.spare_scopes.len(), 1);
        assert!(class_weak.upgrade().is_none());
        assert!(function_weak.upgrade().is_none());
        let previous = runtime.environments.enter(0);
        let scope = runtime.environments.scope(runtime.environments.current);
        assert!(scope.variables.is_empty());
        assert!(scope.functions.is_empty());
        assert!(scope.classes.is_empty());
        assert!(scope.aliases.is_empty());
        assert!(scope.loaded_modules.is_empty());
        assert!(!scope.function_boundary);
        assert!(!scope.captured);
        assert_eq!(scope.parent, Some(0));
        runtime.environments.leave(previous);
    }

    #[test]
    fn spare_scope_storage_is_bounded_and_skips_oversized_maps() {
        let mut environments = Environments::default();
        let mut callers = Vec::new();
        for _ in 0..=super::MAX_SPARE_SCOPES {
            callers.push(environments.enter(environments.current));
        }
        for caller in callers.into_iter().rev() {
            environments.leave(caller);
        }
        assert_eq!(environments.live, 1);
        assert_eq!(environments.spare_scopes.len(), super::MAX_SPARE_SCOPES);
        let mut spares = Vec::new();
        let scope = super::Scope {
            variables: std::collections::HashMap::with_capacity(super::MAX_SPARE_SCOPE_SLOTS + 1)
                .into(),
            ..Default::default()
        };
        Environments::recycle_scope(&mut spares, scope);
        assert!(spares.is_empty());
        let scope = super::Scope {
            variables: std::collections::HashMap::with_capacity(1).into(),
            ..Default::default()
        };
        Environments::recycle_scope(&mut spares, scope);
        assert_eq!(spares.len(), 1);
        assert!(spares[0].variables.capacity() <= super::MAX_SPARE_SCOPE_SLOTS);
    }

    #[test]
    fn borrowed_assignment_names_survive_reentrant_global_map_growth_and_prior_effects() {
        use crate::{NativeError, NativeFunction, NativeSignature, NativeType, Runtime, Value};
        for fail in [false, true] {
            let mut runtime = Runtime::new();
            execute(&mut runtime, "$target = 0;");
            runtime
                .register(
                    "grow",
                    NativeFunction::reentrant(
                        NativeSignature::new(vec![], NativeType::Int),
                        move |context, _| {
                            for index in 0..1000 {
                                context
                                    .runtime
                                    .set_global(format!("injected{index}"), Value::Int(index))
                                    .unwrap();
                            }
                            context.runtime.set_global("target", Value::Int(7)).unwrap();
                            if fail {
                                Err(context.error(NativeError::new("ProbeError", "after mutation")))
                            } else {
                                Ok(Value::Int(42))
                            }
                        },
                    ),
                )
                .unwrap();
            let result = runtime.execute_source("$target = grow();", &mut Vec::new());
            if fail {
                assert!(
                    result
                        .unwrap_err()
                        .message
                        .contains("ProbeError: after mutation")
                );
                assert_eq!(runtime.get_global("target"), Some(&Value::Int(7)));
            } else {
                result.unwrap();
                assert_eq!(runtime.get_global("target"), Some(&Value::Int(42)));
            }
            assert_eq!(runtime.get_global("injected999"), Some(&Value::Int(999)));
            execute(&mut runtime, "$target += 1;");
        }
    }
}
