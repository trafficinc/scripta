//! One canonical value store: indexed small scopes, hashing for larger scopes.
use crate::Value;
use std::{
    collections::{HashMap, hash_map},
    rc::Rc,
};

pub(crate) const SMALL_BINDINGS: usize = 8;

#[derive(Clone)]
pub(crate) enum Bindings {
    Slots(Vec<(String, Value)>),
    Parameters(Vec<(Rc<String>, Value)>),
    Hashed(HashMap<String, Value>),
}

impl Default for Bindings {
    fn default() -> Self {
        Self::Slots(Vec::new())
    }
}

impl Bindings {
    // usize::MAX denotes hashed lookup. All slot accesses validate both bounds
    // and name, including after RHS callbacks promote or replace the store.
    pub fn resolve(&self, name: &str) -> Option<(usize, &Value)> {
        match self {
            Self::Slots(slots) => slots
                .iter()
                .enumerate()
                .find(|(_, (key, _))| key == name)
                .map(|(index, (_, value))| (index, value)),
            Self::Parameters(slots) => slots
                .iter()
                .enumerate()
                .find(|(_, (key, _))| key.as_str() == name)
                .map(|(index, (_, value))| (index, value)),
            Self::Hashed(values) => values.get(name).map(|value| (usize::MAX, value)),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.resolve(name).map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        match self {
            Self::Slots(slots) => slots
                .iter_mut()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            Self::Parameters(slots) => slots
                .iter_mut()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, value)| value),
            Self::Hashed(values) => values.get_mut(name),
        }
    }

    pub fn get_slot(&self, index: usize, name: &str) -> Option<&Value> {
        if let Self::Slots(slots) = self {
            if let Some((key, value)) = slots.get(index) {
                if key == name {
                    return Some(value);
                }
            }
        }
        if let Self::Parameters(slots) = self {
            if let Some((key, value)) = slots.get(index) {
                if key.as_str() == name {
                    return Some(value);
                }
            }
        }
        self.get(name)
    }

    pub fn get_slot_mut(&mut self, index: usize, name: &str) -> Option<&mut Value> {
        match self {
            Self::Slots(slots) => {
                let index = slots
                    .get(index)
                    .filter(|(key, _)| key == name)
                    .map(|_| index)
                    .or_else(|| slots.iter().position(|(key, _)| key == name))?;
                Some(&mut slots[index].1)
            }
            Self::Parameters(slots) => {
                let index = slots
                    .get(index)
                    .filter(|(key, _)| key.as_str() == name)
                    .map(|_| index)
                    .or_else(|| slots.iter().position(|(key, _)| key.as_str() == name))?;
                Some(&mut slots[index].1)
            }
            Self::Hashed(values) => values.get_mut(name),
        }
    }

    pub fn insert(&mut self, name: String, value: Value) -> Option<Value> {
        match self {
            Self::Parameters(slots) => {
                if let Some((_, previous)) = slots.iter_mut().find(|(key, _)| key.as_str() == name)
                {
                    return Some(std::mem::replace(previous, value));
                }
                // An unexpected local/default binding returns to the ordinary
                // store. Keep order so existing verified slot positions work.
                *self = Self::Slots(
                    std::mem::take(slots)
                        .into_iter()
                        .map(|(key, value)| (key.as_ref().clone(), value))
                        .collect(),
                );
                self.insert(name, value)
            }
            Self::Slots(slots) => {
                if let Some((_, previous)) = slots.iter_mut().find(|(key, _)| *key == name) {
                    return Some(std::mem::replace(previous, value));
                }
                if slots.len() < SMALL_BINDINGS {
                    slots.push((name, value));
                    return None;
                }
                // Promotion moves values, never retains a second copy. There is
                // no demotion on removal/recycling, so large scopes retain their map.
                let mut values: HashMap<_, _> = std::mem::take(slots).into_iter().collect();
                let old = values.insert(name, value);
                *self = Self::Hashed(values);
                old
            }
            Self::Hashed(values) => values.insert(name, value),
        }
    }

    pub fn insert_parameter(&mut self, name: &Rc<String>, value: Value) -> Option<Value> {
        if matches!(self, Self::Slots(slots) if slots.is_empty()) {
            *self = Self::Parameters(Vec::new());
        }
        if let Self::Parameters(slots) = self {
            if let Some((_, previous)) = slots
                .iter_mut()
                .find(|(key, _)| key.as_str() == name.as_str())
            {
                return Some(std::mem::replace(previous, value));
            }
            if slots.len() < SMALL_BINDINGS {
                slots.push((name.clone(), value));
                return None;
            }
        }
        self.insert(name.as_ref().clone(), value)
    }

    #[cfg(test)]
    pub fn remove(&mut self, name: &str) -> Option<Value> {
        match self {
            Self::Slots(slots) => slots
                .iter()
                .position(|(key, _)| key == name)
                .map(|index| slots.swap_remove(index).1),
            Self::Parameters(slots) => slots
                .iter()
                .position(|(key, _)| key.as_str() == name)
                .map(|index| slots.swap_remove(index).1),
            Self::Hashed(values) => values.remove(name),
        }
    }

    pub fn capacity(&self) -> usize {
        match self {
            Self::Slots(slots) => slots.capacity(),
            Self::Parameters(slots) => slots.capacity(),
            Self::Hashed(values) => values.capacity(),
        }
    }

    pub fn clear(&mut self) {
        match self {
            Self::Slots(slots) => slots.clear(),
            Self::Parameters(slots) => slots.clear(),
            Self::Hashed(values) => values.clear(),
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        match self {
            Self::Slots(slots) => slots.len(),
            Self::Parameters(slots) => slots.len(),
            Self::Hashed(values) => values.len(),
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> Iter<'_> {
        match self {
            Self::Slots(slots) => Iter::Slots(slots.iter()),
            Self::Parameters(slots) => Iter::Parameters(slots.iter()),
            Self::Hashed(values) => Iter::Hashed(values.iter()),
        }
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.iter().map(|(_, value)| value)
    }

    pub fn extend(&mut self, entries: impl IntoIterator<Item = (String, Value)>) {
        for (name, value) in entries {
            self.insert(name, value);
        }
    }

    pub fn into_map(self) -> HashMap<String, Value> {
        match self {
            Self::Slots(slots) => slots.into_iter().collect(),
            Self::Parameters(slots) => slots
                .into_iter()
                .map(|(key, value)| (key.as_ref().clone(), value))
                .collect(),
            Self::Hashed(values) => values,
        }
    }

    #[cfg(test)]
    pub fn force_hash(&mut self) {
        if !matches!(self, Self::Hashed(_)) {
            *self = Self::Hashed(std::mem::take(self).into_map());
        }
    }
}

impl From<HashMap<String, Value>> for Bindings {
    fn from(values: HashMap<String, Value>) -> Self {
        Self::Hashed(values)
    }
}

pub(crate) enum Iter<'a> {
    Slots(std::slice::Iter<'a, (String, Value)>),
    Parameters(std::slice::Iter<'a, (Rc<String>, Value)>),
    Hashed(hash_map::Iter<'a, String, Value>),
}
impl<'a> Iterator for Iter<'a> {
    type Item = (&'a String, &'a Value);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Slots(slots) => slots.next().map(|(name, value)| (name, value)),
            Self::Parameters(slots) => slots.next().map(|(name, value)| (name.as_ref(), value)),
            Self::Hashed(values) => values.next(),
        }
    }
}
impl<'a> IntoIterator for &'a Bindings {
    type Item = (&'a String, &'a Value);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions};

    #[test]
    fn shared_parameter_names_preserve_slots_promotion_and_release_values() {
        let names: Vec<_> = (0..SMALL_BINDINGS)
            .map(|i| Rc::new(format!("p{i}")))
            .collect();
        let mut bindings = Bindings::default();
        for (i, name) in names.iter().enumerate() {
            bindings.insert_parameter(name, Value::Int(i as i64));
            assert_eq!(Rc::strong_count(name), 2);
        }
        let (slot, _) = bindings.resolve("p3").unwrap();
        assert_eq!(slot, 3);
        // A regular write to an existing parameter keeps the shared name.
        bindings.insert("p3".into(), Value::Int(30));
        assert_eq!(Rc::strong_count(&names[3]), 2);
        // A new local promotes at the same boundary as ordinary bindings.
        bindings.insert("local".into(), Value::Int(9));
        assert!(matches!(bindings, Bindings::Hashed(_)));
        assert_eq!(bindings.get_slot(slot, "p3"), Some(&Value::Int(30)));
        assert!(names.iter().all(|name| Rc::strong_count(name) == 1));
        *bindings.get_slot_mut(slot, "p3").unwrap() = Value::Int(31);
        assert_eq!(bindings.into_map().get("p3"), Some(&Value::Int(31)));

        let mut bindings = Bindings::default();
        let payload: Rc<[u8]> = Rc::from(&b"payload"[..]);
        let weak = Rc::downgrade(&payload);
        bindings.insert_parameter(&names[0], Value::Bytes(payload));
        bindings.clear();
        assert!(weak.upgrade().is_none());
        assert_eq!(Rc::strong_count(&names[0]), 1);
        bindings.insert_parameter(&names[1], Value::Int(4));
        assert!(bindings.get_slot(0, "p0").is_none());
        assert_eq!(bindings.get_slot(0, "p1"), Some(&Value::Int(4)));
        bindings.force_hash();
        assert_eq!(bindings.get("p1"), Some(&Value::Int(4)));
        assert_eq!(Rc::strong_count(&names[1]), 1);
    }

    fn compare(fast: &mut Runtime, slow: &mut Runtime, source: &str) -> bool {
        crate::test_support::compare_source(fast, slow, source).is_ok()
    }

    fn runtimes(options: RuntimeOptions) -> (Runtime, Runtime) {
        let (fast, mut slow) = crate::test_support::runtimes(options);
        slow.environments.force_hash_bindings();
        (fast, slow)
    }

    #[test]
    fn exact_promotion_boundary_and_stale_indices_preserve_one_value_store() {
        let mut bindings = Bindings::default();
        for i in 0..SMALL_BINDINGS {
            bindings.insert(format!("x{i}"), Value::Int(i as i64));
        }
        assert!(matches!(bindings, Bindings::Slots(_)));
        let (slot, _) = bindings.resolve("x7").unwrap();
        assert_eq!(slot, 7);
        bindings.remove("x0");
        // swap_remove moved x7 into x0's former index; an old index must resolve
        // by the requested name rather than aliasing another live variable.
        assert_eq!(bindings.get_slot(slot, "x7"), Some(&Value::Int(7)));
        *bindings.get_slot_mut(0, "x1").unwrap() = Value::Int(100);
        assert_eq!(bindings.get("x1"), Some(&Value::Int(100)));
        assert!(bindings.get_slot(slot, "missing").is_none());
        assert!(bindings.get_slot_mut(usize::MAX, "missing").is_none());
        bindings.insert("x0".into(), Value::Int(0));
        let callable = Value::native_function(
            "marker",
            NativeFunction::new(NativeSignature::new(vec![], NativeType::Null), |_, _| {
                Ok(Value::Null)
            }),
        );
        let Value::Callable(ref marker) = callable else {
            panic!()
        };
        let weak = std::rc::Rc::downgrade(&marker.0);
        bindings.insert("ninth".into(), callable.clone());
        assert!(matches!(bindings, Bindings::Hashed(_)));
        assert_eq!(std::rc::Rc::strong_count(&marker.0), 2);
        assert_eq!(bindings.get_slot(slot, "x7"), Some(&Value::Int(7)));
        *bindings.get_slot_mut(slot, "x7").unwrap() = Value::Int(77);
        assert_eq!(bindings.get("x7"), Some(&Value::Int(77)));
        drop(callable);
        assert!(weak.upgrade().is_some());
        bindings.clear();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn mixed_operations_match_hash_map_before_and_after_promotion() {
        let mut bindings = Bindings::default();
        let mut reference = HashMap::new();
        let mut random = 116_u64;
        for _ in 0..2000 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = format!("key{}", (random >> 32) % 24);
            let value = Value::list(vec![Value::Int((random >> 8) as i64)]);
            match random % 5 {
                0 => assert_eq!(bindings.remove(&key), reference.remove(&key)),
                1 => {
                    if let Some(old) = bindings.get_mut(&key) {
                        *old = value.clone();
                    }
                    if let Some(old) = reference.get_mut(&key) {
                        *old = value;
                    }
                }
                _ => assert_eq!(
                    bindings.insert(key.clone(), value.clone()),
                    reference.insert(key, value)
                ),
            }
            assert_eq!(bindings.len(), reference.len());
            for (name, value) in &reference {
                assert_eq!(bindings.get(name), Some(value));
            }
            assert_eq!(bindings.clone().into_map(), reference);
        }
    }

    #[test]
    fn functions_captures_shadowing_recursion_and_failed_writes_match_hashing() {
        for source in [
            "function f($x){return ($x%97)*3+1;}$observed=f(99);",
            "function fib($n){if($n<2){return $n;}return fib($n-1)+fib($n-2);}$observed=fib(12);",
            "function f($x){$sum=0;foreach([1,2,3] as $x){$sum+=$x;}$observed=$x;return $sum+$observed;}$observed=f(40);",
            "function make($x){return function($step=1){$x+=$step;return $x;};}$f=make(10);$a=$f();$b=$f(3);",
            "$x=99;function f(){return $x;}f();",
            "$x=[1,2];$copy=$x;$x[0]+=3;$observed=$copy[0];",
            "$x=1;try{$x+=1.0;}catch(TypeError $e){$observed=$x;}",
            "$x=[1];try{$x[3]=2;}catch(IndexError $e){$observed=$x;}",
            "function f($x,$y=$x+1,...$rest){return $x+$y+$rest[0];}$observed=f(1,2,3);",
            "class C{$value=1;function add($x){$this->value+=$x;return $this->value;}}$c=new C();$a=$c->add(2);$b=$c->add(3);",
            "$total=0;for($i=0;$i<10;$i+=1){$total+=$i;}",
        ] {
            let (mut fast, mut slow) = runtimes(RuntimeOptions::default());
            assert_eq!(
                compare(&mut fast, &mut slow, source),
                !source.contains("function f(){return $x;}"),
                "{source}"
            );
        }
    }

    #[test]
    fn promotion_during_reentrant_rhs_preserves_targets_and_failure_semantics() {
        for fail in [false, true] {
            for target in ["$target", "$target[0]"] {
                let (mut fast, mut slow) = runtimes(RuntimeOptions::default());
                for runtime in [&mut fast, &mut slow] {
                    runtime
                        .register(
                            "grow",
                            NativeFunction::reentrant(
                                NativeSignature::new(vec![], NativeType::Int),
                                move |context, _| {
                                    for i in 0..1000 {
                                        context
                                            .runtime
                                            .set_global(format!("added{i}"), Value::Int(i))
                                            .unwrap();
                                    }
                                    if fail {
                                        return Err(context.error(crate::NativeError::new(
                                            "TestError",
                                            "injected failure",
                                        )));
                                    }
                                    Ok(Value::Int(7))
                                },
                            ),
                        )
                        .unwrap();
                }
                let initial = if target == "$target" { "1" } else { "[1]" };
                let source = format!(
                    "$target={initial};try{{{target}+=grow();}}catch(TestError $e){{$caught=true;}}$observed=$target;"
                );
                assert!(compare(&mut fast, &mut slow, &source));
                assert!(matches!(
                    fast.environments.scope(0).variables,
                    Bindings::Hashed(_)
                ));
            }
        }
    }

    #[test]
    fn scope_reuse_host_mutation_and_retained_callbacks_remain_live() {
        let (mut fast, mut slow) = runtimes(RuntimeOptions::default());
        compare(
            &mut fast,
            &mut slow,
            "function make($x){return function(){$x+=1;return $x;};}function churn($x){$y=$x+1;return $y;}$f=make(10);$a=$f();",
        );
        let a = fast.get_global("f").unwrap().clone();
        let b = slow.get_global("f").unwrap().clone();
        // The host-held closures are the only roots after removing the globals.
        fast.environments.scope_mut(0).variables.remove("f");
        slow.environments.scope_mut(0).variables.remove("f");
        for runtime in [&mut fast, &mut slow] {
            runtime.collect_garbage();
            runtime.set_global("injected", Value::Int(9)).unwrap();
        }
        for _ in 0..20 {
            assert_eq!(
                fast.invoke(a.clone(), &[], &mut Vec::new()).unwrap(),
                slow.invoke(b.clone(), &[], &mut Vec::new()).unwrap()
            );
            compare(&mut fast, &mut slow, "$observed=churn($injected);");
            fast.collect_garbage();
            slow.collect_garbage();
        }
        compare(&mut fast, &mut slow, "$missing;");
        fast.set_global("missing", Value::Int(3)).unwrap();
        slow.set_global("missing", Value::Int(3)).unwrap();
        compare(&mut fast, &mut slow, "$observed=$missing;");
    }

    #[test]
    fn resource_limits_and_native_interruption_match_hashing() {
        let source = "function f($x){$a=[$x];$a[]=2;return $a[0]+$a[1];}$observed=f(3);";
        for limit in 0..25 {
            let (mut fast, mut slow) = runtimes(RuntimeOptions {
                instruction_limit: Some(limit),
                ..RuntimeOptions::default()
            });
            compare(&mut fast, &mut slow, source);
        }
        for limit in 1..=6 {
            let (mut fast, mut slow) = runtimes(RuntimeOptions {
                max_scopes: Some(limit),
                ..RuntimeOptions::default()
            });
            compare(&mut fast, &mut slow, source);
        }
        let bytes = std::mem::size_of::<Value>() * 3;
        for limit in [0, bytes - 1, bytes, bytes + 1, 4096] {
            let (mut fast, mut slow) = runtimes(RuntimeOptions {
                max_value_bytes: Some(limit),
                ..RuntimeOptions::default()
            });
            compare(&mut fast, &mut slow, source);
        }
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let mut errors = Vec::new();
        for hashed in [false, true] {
            let flag = Arc::new(AtomicBool::new(false));
            let mut runtime = Runtime::with_options(RuntimeOptions {
                interrupt_flag: Some(flag),
                ..RuntimeOptions::default()
            });
            if hashed {
                runtime.environments.force_hash_bindings();
            }
            runtime
                .register(
                    "trip",
                    NativeFunction::new(
                        NativeSignature::new(vec![], NativeType::Int),
                        |context, _| {
                            context
                                .cancellation
                                .as_ref()
                                .unwrap()
                                .store(true, Ordering::Relaxed);
                            Ok(Value::Int(7))
                        },
                    ),
                )
                .unwrap();
            errors.push(
                runtime
                    .execute_source("$x=1;$x+=trip();", &mut Vec::new())
                    .unwrap_err(),
            );
            assert_eq!(runtime.get_global("x"), Some(&Value::Int(1)));
            assert_eq!(runtime.environments.live, 1);
        }
        assert_eq!(errors[0], errors[1]);
    }
}
