//! The one element that owns keyboard focus.
//!
//! Widgets keep their own `is_focused` flag, but only the owner recorded here
//! may act on it: a widget that finds it lost ownership drops its focus. A
//! press handled by `UI` releases ownership before it is dispatched, so the
//! widget under the cursor can take it again and every other one blurs, even
//! if the press never reached it (consumed earlier, scrolled out of view).

use std::sync::atomic::{AtomicU64, Ordering};

use super::element_handle::UIElementHandle;

/// No handle uses this id: auto ids count up from 2^32.
const NONE: u64 = u64::MAX;

static OWNER: AtomicU64 = AtomicU64::new(NONE);

/// Make `handle` the focus owner.
pub fn take(handle: &UIElementHandle) {
    OWNER.store(handle.id(), Ordering::Relaxed);
}

/// Give up focus, if `handle` owns it.
pub fn release(handle: &UIElementHandle) {
    let _ = OWNER.compare_exchange(handle.id(), NONE, Ordering::Relaxed, Ordering::Relaxed);
}

/// Whether `handle` owns focus.
pub fn owns(handle: &UIElementHandle) -> bool {
    OWNER.load(Ordering::Relaxed) == handle.id()
}

/// The current focus owner.
pub fn owner() -> Option<UIElementHandle> {
    match OWNER.load(Ordering::Relaxed) {
        NONE => None,
        id => Some(UIElementHandle::from(id)),
    }
}

/// Nobody owns focus any more; focused widgets blur on their next update or
/// event.
pub fn clear() {
    OWNER.store(NONE, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_moves_and_is_released_only_by_the_owner() {
        let a = UIElementHandle::new();
        let b = UIElementHandle::new();
        take(&a);
        assert!(owns(&a));
        take(&b);
        assert!(!owns(&a) && owns(&b));
        // A stale owner releasing must not take focus away from the new one.
        release(&a);
        assert_eq!(owner(), Some(b));
        release(&b);
        assert_eq!(owner(), None);
        take(&a);
        clear();
        assert!(!owns(&a));
    }
}
