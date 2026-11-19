use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UIElementHandle(u64);

// Auto-generated handles live above this value; `UIElementHandle::from(small)`
// stays reserved for explicit, user-assigned ids. Keeping auto ids monotonic
// means `BTreeMap<UIElementHandle, _>` iterates in insertion order.
const AUTO_HANDLE_BASE: u64 = 1 << 32;
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(AUTO_HANDLE_BASE);

impl UIElementHandle {
    pub fn new() -> Self {
        Self(NEXT_HANDLE.fetch_add(1, AtomicOrdering::Relaxed))
    }
    pub fn from(id: u64) -> Self {
        Self(id)
    }
    pub(crate) fn id(&self) -> u64 {
        self.0
    }
}

impl Ord for UIElementHandle {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for UIElementHandle {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
