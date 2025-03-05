use rand::Rng;
use std::cmp::Ordering;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd)]
pub struct UIElementHandle(u64);

impl UIElementHandle {
    pub fn new() -> Self {
        Self {
            0: rand::rng().random::<u64>(),
        }
    }
    pub fn from(id: u64) -> Self {
        Self { 0: id }
    }
}

impl Ord for UIElementHandle {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
