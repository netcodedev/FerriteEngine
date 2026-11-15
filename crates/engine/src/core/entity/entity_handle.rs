use rand::prelude::*;

use super::EntityHandle;

impl EntityHandle {
    pub fn new() -> Self {
        EntityHandle(rand::rng().random::<u64>())
    }

    pub fn from(id: u64) -> Self {
        EntityHandle(id)
    }
}

impl From<EntityHandle> for u64 {
    fn from(handle: EntityHandle) -> u64 {
        handle.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn handles_from_new_are_different() {
        // Random u64 collisions are vanishingly unlikely; ten thousand must be unique.
        let handles: HashSet<u64> = (0..10_000).map(|_| EntityHandle::new().into()).collect();
        assert_eq!(handles.len(), 10_000);
    }

    #[test]
    fn a_handle_roundtrips_through_u64() {
        let h = EntityHandle::new();
        let as_u64: u64 = h.into();
        assert_eq!(EntityHandle::from(as_u64), h);
    }

    #[test]
    fn two_handles_from_the_same_u64_are_equal() {
        assert_eq!(EntityHandle::from(42), EntityHandle::from(42));
    }
}
