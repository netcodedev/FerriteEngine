use rand::Rng;

use super::EntityHandle;

impl EntityHandle {
    pub fn new() -> Self {
        EntityHandle(rand::thread_rng().gen())
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
