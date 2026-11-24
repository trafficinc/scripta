//! Bounded class-local training for stable reads of late declaration fields.
use std::cell::Cell;

const INDEX_BITS: u32 = 24;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
const WARM_SHIFT: u32 = INDEX_BITS;
const CHURN_SHIFT: u32 = 28;
const WARM: u32 = 4;
const MAX_CHURN: u32 = 4;
const DISABLED: u32 = u32::MAX;
pub(crate) const MIN_FIELDS: usize = 16;
const MIN_SLOT: usize = 8;

pub(crate) struct FieldRead {
    state: Cell<u32>,
    #[cfg(test)]
    pub hits: Cell<usize>,
}

impl Default for FieldRead {
    fn default() -> Self {
        Self::new(0, false)
    }
}

impl FieldRead {
    pub fn new(fields: usize, ordinary: bool) -> Self {
        Self {
            state: Cell::new(if ordinary && fields >= MIN_FIELDS {
                0
            } else {
                DISABLED
            }),
            #[cfg(test)]
            hits: Cell::new(0),
        }
    }

    pub fn enabled(&self) -> bool {
        self.state.get() != DISABLED
    }

    pub fn slot(&self) -> Option<usize> {
        let state = self.state.get();
        if state == DISABLED || (state >> WARM_SHIFT) & 15 < WARM {
            return None;
        }
        (state & INDEX_MASK)
            .checked_sub(1)
            .map(|slot| slot as usize)
    }

    // Called once per actual field read, never by the classification lookup.
    pub fn observe(&self, slot: usize) {
        let state = self.state.get();
        if state == DISABLED || slot < MIN_SLOT {
            return;
        }
        let Some(index) = u32::try_from(slot)
            .ok()
            .and_then(|v| v.checked_add(1))
            .filter(|v| *v <= INDEX_MASK)
        else {
            self.state.set(DISABLED);
            return;
        };
        let old_index = state & INDEX_MASK;
        let count = (state >> WARM_SHIFT) & 15;
        let churn = state >> CHURN_SHIFT;
        let (count, churn) = if old_index == index {
            (
                (count + 1).min(WARM),
                churn.saturating_sub(u32::from(count >= WARM)),
            )
        } else {
            (1, churn + u32::from(old_index != 0))
        };
        if churn >= MAX_CHURN {
            self.state.set(DISABLED);
            return;
        }
        let next = index | (count << WARM_SHIFT) | (churn << CHURN_SHIFT);
        if next != state {
            self.state.set(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_repeated_late_reads_and_disables_churning_layouts() {
        let stable = FieldRead::new(64, true);
        assert!(stable.slot().is_none());
        for _ in 0..3 {
            stable.observe(63);
            assert!(stable.slot().is_none());
        }
        stable.observe(63);
        assert_eq!(stable.slot(), Some(63));
        stable.observe(0);
        assert_eq!(stable.slot(), Some(63));
        let mixed = FieldRead::new(64, true);
        for slot in [62, 63, 62, 63, 62] {
            mixed.observe(slot);
        }
        assert!(!mixed.enabled());
        assert!(mixed.slot().is_none());
        assert!(!FieldRead::new(8, true).enabled());
        assert!(!FieldRead::new(64, false).enabled());
        stable.observe(usize::MAX);
        assert!(!stable.enabled());
    }
}
