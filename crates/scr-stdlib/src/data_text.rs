//! Small regex surface and explicit grapheme operations.
use crate::bytes::{budget, grow, payload, range, resource};
use regex::{Regex, RegexBuilder};
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime, Value,
};
use std::{cell::RefCell, collections::VecDeque, mem::size_of, rc::Rc};
use unicode_segmentation::UnicodeSegmentation;

const MAX_PATTERN_BYTES: usize = 65536;
const MAX_CACHE_ENTRIES: usize = 8;
const MAX_CACHE_PATTERN_BYTES: usize = 256 * 1024;
const REGEX_SIZE_LIMIT: usize = 1024 * 1024;

// Oldest entry first. Fixed entry and key-byte limits bound retained backend
// state independently of script value budgets. Regex's size limits are engine
// limits, not exact heap accounting (scratch space and metadata add overhead).
#[derive(Default)]
struct RegexCache {
    entries: VecDeque<Rc<Regex>>,
    pattern_bytes: usize,
}

impl RegexCache {
    fn get_or_compile(&mut self, pattern: &str) -> NativeResult<Rc<Regex>> {
        if pattern.len() > MAX_PATTERN_BYTES {
            return Err(NativeError::new(
                "RegexError",
                "pattern exceeds 65536 bytes",
            ));
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|regex| regex.as_str() == pattern)
        {
            let regex = self.entries.remove(index).expect("cached regex");
            self.entries.push_back(Rc::clone(&regex));
            return Ok(regex);
        }
        // Compile before eviction: invalid patterns neither occupy cache space
        // nor evict successful entries. One miss can temporarily retain one extra
        // bounded compiled regex while the existing cache is still alive.
        let regex = Rc::new(
            RegexBuilder::new(pattern)
                .size_limit(REGEX_SIZE_LIMIT)
                .dfa_size_limit(REGEX_SIZE_LIMIT)
                .build()
                .map_err(|e| NativeError::new("RegexError", e.to_string()))?,
        );
        while self.entries.len() >= MAX_CACHE_ENTRIES
            || self.pattern_bytes + pattern.len() > MAX_CACHE_PATTERN_BYTES
        {
            let oldest = self.entries.pop_front().expect("nonempty bounded cache");
            self.pattern_bytes -= oldest.as_str().len();
        }
        self.pattern_bytes += pattern.len();
        self.entries.push_back(Rc::clone(&regex));
        Ok(regex)
    }
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    register_with_cache(runtime, Rc::new(RefCell::new(RegexCache::default())))
}

fn register_with_cache(runtime: &mut Runtime, cache: Rc<RefCell<RegexCache>>) -> NativeResult<()> {
    use NativeType::{Bool, Int, List, String as Text};
    for (name, returns) in [("isMatch", Bool), ("findAll", List), ("replace", Text)] {
        let mut parameters = vec![Text, Text];
        if name == "replace" {
            parameters.push(Text);
        }
        let cache = Rc::clone(&cache);
        runtime.register(
            format!("Regex::{name}"),
            NativeFunction::new(
                NativeSignature::new(parameters, returns),
                move |context, args| {
                    let Value::String(input) = &args[0] else {
                        unreachable!()
                    };
                    let Value::String(pattern) = &args[1] else {
                        unreachable!()
                    };
                    context.check_cancelled()?;
                    let regex = cache.borrow_mut().get_or_compile(pattern)?;
                    context.check_cancelled()?;
                    if name == "isMatch" {
                        return Ok(Value::Bool(regex.is_match(input)));
                    }
                    let mut size = size_of::<Value>();
                    budget(context, size)?;
                    if name == "findAll" {
                        let mut values = Vec::new();
                        for found in regex.find_iter(input) {
                            context.check_cancelled()?;
                            grow(context, &mut size, size_of::<Value>() + found.len())?;
                            values
                                .try_reserve(1)
                                .map_err(|_| resource("match allocation failed"))?;
                            values.push(Value::String(found.as_str().into()));
                        }
                        return Ok(Value::List(values));
                    }
                    let Value::String(replacement) = &args[2] else {
                        unreachable!()
                    };
                    // Literal replacement deliberately avoids exposing captures in this API.
                    let mut result = String::new();
                    let mut end = 0;
                    for found in regex.find_iter(input) {
                        context.check_cancelled()?;
                        let prefix = &input[end..found.start()];
                        grow(context, &mut size, prefix.len())?;
                        grow(context, &mut size, replacement.len())?;
                        result
                            .try_reserve(prefix.len() + replacement.len())
                            .map_err(|_| resource("replacement allocation failed"))?;
                        result.push_str(prefix);
                        result.push_str(replacement);
                        end = found.end();
                    }
                    grow(context, &mut size, input.len() - end)?;
                    result
                        .try_reserve(input.len() - end)
                        .map_err(|_| resource("replacement allocation failed"))?;
                    result.push_str(&input[end..]);
                    Ok(Value::String(result))
                },
            ),
        )?;
    }
    runtime.register(
        "String::graphemeLength",
        NativeFunction::new(NativeSignature::new(vec![Text], Int), |context, args| {
            let Value::String(input) = &args[0] else {
                unreachable!()
            };
            let mut count = 0i64;
            for _ in input.graphemes(true) {
                context.check_cancelled()?;
                count = count
                    .checked_add(1)
                    .ok_or_else(|| NativeError::new("RangeError", "length exceeds i64"))?;
            }
            Ok(Value::Int(count))
        }),
    )?;
    runtime.register(
        "String::graphemeSubstring",
        NativeFunction::new(
            NativeSignature::new(vec![Text, Int], Text).with_optional(vec![Int]),
            |context, args| {
                let Value::String(input) = &args[0] else {
                    unreachable!()
                };
                let mut count = 0;
                for _ in input.graphemes(true) {
                    context.check_cancelled()?;
                    count += 1;
                }
                let positions = range(args, count)?;
                let mut start = input.len();
                let mut end = input.len();
                for (index, (byte, _)) in input.grapheme_indices(true).enumerate() {
                    context.check_cancelled()?;
                    if index == positions.start {
                        start = byte;
                    }
                    if index == positions.end {
                        end = byte;
                        break;
                    }
                }
                payload(context, end - start)?;
                Ok(Value::String(input[start..end].into()))
            },
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_cache_reuses_promotes_and_evicts_lru() {
        let mut cache = RegexCache::default();
        let first = cache.get_or_compile("first").unwrap();
        for index in 1..MAX_CACHE_ENTRIES {
            cache.get_or_compile(&format!("p{index}")).unwrap();
        }
        assert!(Rc::ptr_eq(&first, &cache.get_or_compile("first").unwrap()));
        let evicted = Rc::downgrade(&cache.entries[0]);
        cache.get_or_compile("new").unwrap();
        assert!(evicted.upgrade().is_none());
        assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
        assert!(Rc::ptr_eq(&first, &cache.get_or_compile("first").unwrap()));
        assert!(cache.get_or_compile("p1").unwrap().is_match("p1"));
        assert!(!cache.get_or_compile("p1").unwrap().is_match("p2"));
    }

    #[test]
    fn regex_cache_bounds_keys_and_preserves_entries_on_errors() {
        let mut cache = RegexCache::default();
        // Comments let the pattern reach the byte ceiling without also hitting
        // the independently enforced compiled-size ceiling.
        let pattern = format!("(?x)#{}", "x".repeat(MAX_PATTERN_BYTES - 5));
        assert_eq!(pattern.len(), MAX_PATTERN_BYTES);
        for index in 0..4 {
            let key = format!("{}{}", &pattern[..pattern.len() - 1], index);
            cache.get_or_compile(&key).unwrap();
        }
        assert_eq!(cache.pattern_bytes, MAX_CACHE_PATTERN_BYTES);
        let oldest = Rc::downgrade(&cache.entries[0]);
        cache.get_or_compile("z").unwrap();
        assert!(oldest.upgrade().is_none());
        assert_eq!(cache.pattern_bytes, 3 * MAX_PATTERN_BYTES + 1);
        let retained = Rc::clone(&cache.entries[0]);
        let before = cache.pattern_bytes;
        for invalid in [
            "[".into(),
            "(?=x)".into(),
            "x".repeat(MAX_PATTERN_BYTES + 1),
            r"\p{L}{10000}".into(),
        ] {
            let error = cache.get_or_compile(&invalid).unwrap_err();
            assert_eq!(error.kind, "RegexError");
            assert_eq!(cache.pattern_bytes, before);
            assert!(Rc::ptr_eq(&retained, &cache.entries[0]));
        }
    }

    #[test]
    fn regex_cache_is_shared_by_apis_isolated_and_dropped_with_runtime() {
        let cache = Rc::new(RefCell::new(RegexCache::default()));
        let weak = Rc::downgrade(&cache);
        let mut runtime = Runtime::new();
        register_with_cache(&mut runtime, Rc::clone(&cache)).unwrap();
        for source in [
            "Regex::isMatch('é', '.');",
            "Regex::findAll('é', '.');",
            "Regex::replace('é', '.', '$1');",
        ] {
            runtime.execute_source(source, &mut Vec::new()).unwrap();
            assert_eq!(cache.borrow().entries.len(), 1);
        }
        let compiled = Rc::downgrade(&cache.borrow().entries[0]);
        let other = Rc::new(RefCell::new(RegexCache::default()));
        let mut other_runtime = Runtime::new();
        register_with_cache(&mut other_runtime, Rc::clone(&other)).unwrap();
        assert!(other.borrow().entries.is_empty());
        other_runtime
            .execute_source("Regex::isMatch('é', '.');", &mut Vec::new())
            .unwrap();
        assert!(!Rc::ptr_eq(
            &cache.borrow().entries[0],
            &other.borrow().entries[0]
        ));
        drop(cache);
        drop(runtime);
        assert!(weak.upgrade().is_none());
        assert!(compiled.upgrade().is_none());
    }
}
