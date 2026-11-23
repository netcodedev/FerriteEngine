//! Holders counted where they are made and dropped, whoever owns them
//! (models live in the engine's components and in scripts' own): a
//! `Counter` per kind, a `Tally` in each holder.

use std::sync::atomic::{AtomicI64, Ordering::Relaxed};

use super::Report;

/// How many of a kind live, and their bytes.
pub struct Counter {
    pub group: &'static str,
    pub name: &'static str,
    count: AtomicI64,
    bytes: AtomicI64,
}

impl Counter {
    pub const fn new(group: &'static str, name: &'static str) -> Self {
        Self { group, name, count: AtomicI64::new(0), bytes: AtomicI64::new(0) }
    }

    pub fn report(&self, report: &mut Report) {
        report.add(self.group, self.name, self.count.load(Relaxed).max(0) as usize, self.bytes.load(Relaxed).max(0) as usize);
    }
}

/// One holder of a `Counter`'s kind: counted while it lives, with the
/// bytes last `set`.
pub struct Tally {
    counter: &'static Counter,
    bytes: usize,
}

impl Tally {
    pub fn new(counter: &'static Counter) -> Self {
        counter.count.fetch_add(1, Relaxed);
        Self { counter, bytes: 0 }
    }

    pub fn set(&mut self, bytes: usize) {
        self.counter.bytes.fetch_add(bytes as i64 - self.bytes as i64, Relaxed);
        self.bytes = bytes;
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        self.counter.count.fetch_sub(1, Relaxed);
        self.counter.bytes.fetch_sub(self.bytes as i64, Relaxed);
    }
}

/// Models' CPU copies of their geometry: assimp's scene and the meshes'
/// vertices and indices, kept after the upload.
pub static MODELS: Counter = Counter::new("models", "CPU geometry (assimp scene + meshes)");

/// Every counter, for the snapshot.
pub(super) const ALL: &[&Counter] = &[&MODELS];

#[cfg(test)]
mod tests {
    use super::*;

    static TEST: Counter = Counter::new("test", "things");

    #[test]
    fn counts_living_holders() {
        let mut a = Tally::new(&TEST);
        a.set(100);
        {
            let mut b = Tally::new(&TEST);
            b.set(50);
            a.set(80);
        }
        let mut r = Report::default();
        TEST.report(&mut r);
        assert_eq!((r.items()[0].count, r.items()[0].bytes), (Some(1), 80));
    }
}
