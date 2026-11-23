//! The RAM map: what holds how much memory now, as a snapshot that two of
//! can be compared (`diff`) to see what grows. See `docs/memory-map.md`.
//!
//! A snapshot has four groups of rows:
//! - `process`: what the system counts (resident memory by kind of
//!   mapping); the truth, but without names.
//! - `heap`: Rust's allocations alive (`heap::Counting`, the global
//!   allocator) and what glibc's malloc keeps besides; live bytes that stay
//!   flat while the resident memory grows point to malloc keeping freed
//!   memory, not to a leak.
//! - `gpu`: the Vulkan allocator's allocations by name, and its blocks.
//! - the engine's own holders (`Scene::memory_report`): chunks, caches,
//!   ray tracing structures, by count and estimated bytes; and the heap no
//!   holder explains.
//!
//! Snapshots are saved as RON in the log folder (`memory/`), each with a
//! text file comparing it with the one before.

pub mod heap;
mod process;
pub mod tally;

pub use tally::{Counter, Tally};

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::{diagnostics, gfx, scene::Scene};

/// Snapshots kept (the newest; older ones and their comparisons deleted).
pub const KEEP: usize = 40;

/// Rows of the process group.
pub const PROCESS: &str = "process";
/// Rows of the heap group.
pub const HEAP: &str = "heap";
/// Rows of the GPU group.
pub const GPU: &str = "gpu";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Item {
    /// `process`, `heap`, `gpu` or the engine part (`terrain`, `rays`, ...).
    pub group: String,
    pub name: String,
    /// How many of it, where that means something (chunks, blocks).
    #[serde(default)]
    pub count: Option<u64>,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// UTC, `2026-11-23 14:05:09`.
    pub taken: String,
    /// Seconds since 1970, for the time between two.
    pub unix: u64,
    #[serde(default)]
    pub label: String,
    pub items: Vec<Item>,
}

/// What the engine's holders report into (`Scene::memory_report`).
#[derive(Default)]
pub struct Report {
    items: Vec<Item>,
}

impl Report {
    /// `count` things of `group`'s `name`, `bytes` in all (an estimate:
    /// capacities times element sizes, not the allocator's overhead).
    pub fn add(&mut self, group: &str, name: &str, count: usize, bytes: usize) {
        match self.items.iter_mut().find(|i| i.group == group && i.name == name) {
            Some(item) => {
                item.count = Some(item.count.unwrap_or(0) + count as u64);
                item.bytes += bytes as u64;
            }
            None => self.items.push(Item { group: group.to_owned(), name: name.to_owned(), count: Some(count as u64), bytes: bytes as u64 }),
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

/// Bytes a `Vec`'s storage takes.
pub fn vec_bytes<T>(v: &Vec<T>) -> usize {
    v.capacity() * std::mem::size_of::<T>()
}

impl Snapshot {
    /// Take one now, of `scene` (on the render thread: the GPU's rows come
    /// from its device).
    pub fn take(scene: Option<&Scene>, label: &str) -> Self {
        let now = SystemTime::now();
        let mut items = Vec::new();
        let mut row = |group: &str, name: &str, count: Option<u64>, bytes: u64| {
            items.push(Item { group: group.to_owned(), name: name.to_owned(), count, bytes })
        };

        for (name, bytes) in process::totals() {
            row(PROCESS, name, None, bytes);
        }
        for (name, bytes, mappings) in process::mappings() {
            row(PROCESS, &format!("mapped: {name}"), Some(mappings), bytes);
        }

        let (live, blocks) = heap::live();
        row(HEAP, "rust allocations alive", Some(blocks), live);
        if let Some(m) = heap::malloc_info() {
            row(HEAP, "malloc in use (all of C and Rust)", None, m.in_use);
            row(HEAP, "malloc in own blocks (mmap)", Some(m.mapped_blocks), m.mapped);
            row(HEAP, "malloc freed but kept", None, m.free_kept);
        }

        let gpu = gfx::device().memory_items();
        for (name, count, bytes) in gpu {
            row(GPU, &name, Some(count), bytes);
        }
        if let Some((used, total)) = gfx::device().video_memory() {
            row(GPU, "driver: video memory used", None, used);
            row(GPU, "driver: video memory total", None, total);
        }

        let mut report = Report::default();
        if let Some(scene) = scene {
            scene.memory_report(&mut report);
        }
        for counter in tally::ALL {
            counter.report(&mut report);
        }
        let explained: u64 = report.items.iter().map(|i| i.bytes).sum();
        items.extend(report.items);
        items.push(Item {
            group: HEAP.to_owned(),
            name: "rust allocations no holder reports".to_owned(),
            count: None,
            bytes: live.saturating_sub(explained),
        });

        Self {
            taken: diagnostics::readable_stamp(now),
            unix: now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            label: label.trim().to_owned(),
            items,
        }
    }

    /// Bytes of a row, 0 where it is missing.
    pub fn bytes(&self, group: &str, name: &str) -> u64 {
        self.items.iter().find(|i| i.group == group && i.name == name).map_or(0, |i| i.bytes)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Where snapshots are saved: `memory/` in the log folder.
pub fn folder() -> Option<PathBuf> {
    diagnostics::folder().map(|d| d.join("memory"))
}

/// The snapshots saved, oldest first.
pub fn saved() -> Vec<PathBuf> {
    let Some(dir) = folder() else { return Vec::new() };
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .collect();
    files.sort();
    files
}

/// Save `snapshot`, with a comparison with the one saved before it (a
/// `.txt` of the same name). Returns the snapshot's path and the
/// comparison, if there was one before.
pub fn save(snapshot: &Snapshot) -> Result<(PathBuf, Option<Diff>), String> {
    let dir = folder().ok_or("no log folder (diagnostics not installed)")?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let previous = saved().last().and_then(|p| Snapshot::load(p).ok());
    let stamp = snapshot.taken.replace([' ', ':'], "-");
    let label: String = snapshot
        .label
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .take(40)
        .collect();
    let name = if label.is_empty() { stamp } else { format!("{stamp}_{label}") };
    let path = dir.join(format!("{name}.ron"));
    let text = ron::ser::to_string_pretty(snapshot, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    let diff = previous.map(|before| Diff::between(&before, snapshot));
    if let Some(diff) = &diff {
        let _ = fs::write(path.with_extension("txt"), diff.text(usize::MAX));
    }
    prune();
    Ok((path, diff))
}

fn prune() {
    let files = saved();
    for old in files.iter().take(files.len().saturating_sub(KEEP)) {
        let _ = fs::remove_file(old);
        let _ = fs::remove_file(old.with_extension("txt"));
    }
}

/// One row of a comparison.
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub group: String,
    pub name: String,
    pub before: u64,
    pub after: u64,
    pub count_before: Option<u64>,
    pub count_after: Option<u64>,
}

impl Change {
    pub fn delta(&self) -> i64 {
        self.after as i64 - self.before as i64
    }
}

/// Two snapshots compared, rows by how much they changed.
#[derive(Clone, Debug, Serialize)]
pub struct Diff {
    pub before: String,
    pub after: String,
    pub seconds: u64,
    pub changes: Vec<Change>,
}

impl Diff {
    pub fn between(before: &Snapshot, after: &Snapshot) -> Self {
        let mut changes: Vec<Change> = Vec::new();
        for item in &after.items {
            let old = before.items.iter().find(|i| i.group == item.group && i.name == item.name);
            changes.push(Change {
                group: item.group.clone(),
                name: item.name.clone(),
                before: old.map_or(0, |o| o.bytes),
                after: item.bytes,
                count_before: old.and_then(|o| o.count),
                count_after: item.count,
            });
        }
        for old in &before.items {
            if !after.items.iter().any(|i| i.group == old.group && i.name == old.name) {
                changes.push(Change {
                    group: old.group.clone(),
                    name: old.name.clone(),
                    before: old.bytes,
                    after: 0,
                    count_before: old.count,
                    count_after: None,
                });
            }
        }
        changes.sort_by_key(|c| std::cmp::Reverse(c.delta().unsigned_abs()));
        let name = |s: &Snapshot| if s.label.is_empty() { s.taken.clone() } else { format!("{} ({})", s.taken, s.label) };
        Self { before: name(before), after: name(after), seconds: after.unix.saturating_sub(before.unix), changes }
    }

    /// The rows that grew most, without the process's totals (which every
    /// growth shows up in): what to show first.
    pub fn top_growth(&self, n: usize) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| c.group != PROCESS && c.delta() > 0).take(n)
    }

    /// As a table, the `limit` rows that changed most (`usize::MAX`: all).
    pub fn text(&self, limit: usize) -> String {
        let resident = self
            .changes
            .iter()
            .find(|c| c.group == PROCESS && c.name == "resident (RSS)")
            .map(|c| format!("resident {} -> {} ({})\n", size(c.before as i64), size(c.after as i64), signed(c.delta())))
            .unwrap_or_default();
        let mut text = format!(
            "Memory from {} to {} ({} min)\n{resident}\n{:<10} {:<52} {:>10} {:>10} {:>10} {:>14}\n",
            self.before,
            self.after,
            self.seconds / 60,
            "group",
            "what",
            "before",
            "after",
            "change",
            "count",
        );
        for c in self.changes.iter().take(limit) {
            let count = match (c.count_before, c.count_after) {
                (Some(a), Some(b)) if a != b => format!("{a} -> {b}"),
                (_, Some(b)) => b.to_string(),
                (Some(a), None) => format!("{a} -> 0"),
                _ => String::new(),
            };
            text += &format!(
                "{:<10} {:<52} {:>10} {:>10} {:>10} {:>14}\n",
                c.group,
                shorten(&c.name, 52),
                size(c.before as i64),
                size(c.after as i64),
                signed(c.delta()),
                count,
            );
        }
        text
    }
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        s.chars().take(max - 1).chain(std::iter::once('…')).collect()
    }
}

/// `1.2 GB`, `340 MB`, `12 KB`.
pub fn size(bytes: i64) -> String {
    let b = bytes.unsigned_abs() as f64;
    let sign = if bytes < 0 { "-" } else { "" };
    if b >= 1e9 {
        format!("{sign}{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{sign}{:.0} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{sign}{:.0} KB", b / 1e3)
    } else {
        format!("{sign}{b} B")
    }
}

/// `size` with a plus for growth.
pub fn signed(bytes: i64) -> String {
    if bytes > 0 {
        format!("+{}", size(bytes))
    } else {
        size(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(unix: u64, items: &[(&str, &str, u64)]) -> Snapshot {
        Snapshot {
            taken: format!("t{unix}"),
            unix,
            label: String::new(),
            items: items
                .iter()
                .map(|(g, n, b)| Item { group: g.to_string(), name: n.to_string(), count: Some(1), bytes: *b })
                .collect(),
        }
    }

    #[test]
    fn diff_orders_by_change_and_keeps_vanished_rows() {
        let a = snap(0, &[("terrain", "chunks", 100), ("gpu", "texture", 50), ("rays", "records", 10)]);
        let b = snap(600, &[("terrain", "chunks", 400), ("gpu", "texture", 40)]);
        let d = Diff::between(&a, &b);
        assert_eq!(d.seconds, 600);
        let order: Vec<(&str, i64)> = d.changes.iter().map(|c| (c.name.as_str(), c.delta())).collect();
        assert_eq!(order, [("chunks", 300), ("texture", -10), ("records", -10)]);
        assert_eq!(d.top_growth(5).count(), 1);
        assert!(d.text(10).contains("+300 B"));
    }

    #[test]
    fn report_sums_rows_of_one_name() {
        let mut r = Report::default();
        r.add("terrain", "chunks", 2, 100);
        r.add("terrain", "chunks", 3, 50);
        assert_eq!(r.items(), [Item { group: "terrain".into(), name: "chunks".into(), count: Some(5), bytes: 150 }]);
    }

    #[test]
    fn snapshot_round_trips_as_ron() {
        let s = snap(5, &[("heap", "rust allocations alive", 7)]);
        let text = ron::ser::to_string_pretty(&s, ron::ser::PrettyConfig::default()).unwrap();
        let back: Snapshot = ron::from_str(&text).unwrap();
        assert_eq!(back.items, s.items);
    }
}
