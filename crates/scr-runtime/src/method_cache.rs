//! Class-owned call-site hints, containing no receiver or declaration owners.
use scr_diagnostics::Span;
use std::cell::{Cell, RefCell};

const SLOTS: usize = 8;
const MAX_MISSES: u8 = 32;

#[derive(Clone, Copy)]
pub(crate) struct Hint {
    pub span: Span,
    pub depth: usize,
    pub method: usize,
}

#[derive(Default)]
pub(crate) struct MethodCache {
    pub field_reads: crate::field_read::FieldRead,
    entries: RefCell<Vec<Option<Hint>>>,
    misses: Cell<u8>,
    disabled: Cell<bool>,
    #[cfg(test)]
    pub hits: Cell<usize>,
}

impl MethodCache {
    pub fn with_fields(fields: usize, ordinary: bool) -> Self {
        Self {
            field_reads: crate::field_read::FieldRead::new(fields, ordinary),
            ..Self::default()
        }
    }

    pub fn enabled(&self, span: Span) -> bool {
        // Host-built trees can deliberately reuse empty/default spans. They
        // retain ordinary lookup instead of repeatedly replacing one entry.
        !self.disabled.get() && span.start < span.end
    }

    fn slot(span: Span) -> usize {
        (span.start ^ (span.start >> 3) ^ span.end) & (SLOTS - 1)
    }

    pub fn hint(&self, span: Span) -> Option<Hint> {
        self.entries
            .borrow()
            .get(Self::slot(span))
            .copied()
            .flatten()
            .filter(|hint| hint.span == span)
    }

    // Called only after checking the actual immutable class declaration's name
    // and bounds. A span is a locator, never proof of method identity.
    pub fn hit(&self) {
        self.misses.set(self.misses.get().saturating_sub(1));
        #[cfg(test)]
        self.hits.set(self.hits.get() + 1);
    }

    pub fn remember(&self, hint: Hint) {
        // Hits repay misses one at a time, so an occasional hit cannot keep a
        // mostly colliding workload on the speculative path indefinitely.
        let misses = self.misses.get().saturating_add(1);
        self.misses.set(misses);
        if misses >= MAX_MISSES {
            self.disabled.set(true);
            self.entries.borrow_mut().clear();
            return;
        }
        let mut entries = self.entries.borrow_mut();
        if entries.is_empty() {
            if entries.try_reserve_exact(SLOTS).is_err() {
                self.disabled.set(true);
                return;
            }
            entries.resize(SLOTS, None);
        }
        entries[Self::slot(hint.span)] = Some(hint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_sites_collisions_and_exhaustion_fall_back_with_bounded_storage() {
        let cache = MethodCache::default();
        assert!(!cache.enabled(Span::default()));
        let span = Span::new(8, 16);
        assert!(cache.hint(span).is_none());
        cache.remember(Hint {
            span,
            depth: 0,
            method: 3,
        });
        assert_eq!(cache.hint(span).unwrap().method, 3);
        cache.hit();
        for index in 0..usize::from(MAX_MISSES) {
            cache.remember(Hint {
                span: Span::new(index + 1, index + 2),
                depth: 0,
                method: index,
            });
            assert!(cache.entries.borrow().len() <= SLOTS);
        }
        assert!(!cache.enabled(span));
        assert!(cache.entries.borrow().is_empty());
    }
}
