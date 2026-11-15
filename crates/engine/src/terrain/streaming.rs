//! Which columns of chunks are loaded: those around an anchor (the camera),
//! made on worker threads, nearest first, and let go again once the anchor
//! has moved well away from them.
//!
//! The unit is a column (every chunk stacked over one chunk square): the
//! worker that makes it knows from the column's heights which chunks of
//! the stack hold ground, so nothing on the main thread has to.
//!
//! Nothing here knows about GL or chunks: the work a column takes is a
//! function handed in, and its result is passed back as it is.

use std::{
    collections::{BinaryHeap, HashSet},
    cmp::Reverse,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use super::CHUNK_SIZE;

/// A column of chunks, by its chunk square `(x, z)`.
pub type ColumnKey = (i32, i32);

/// How far around the anchor columns load: up to `load_radius` metres
/// from the anchor to the nearest point of their square, and stay loaded
/// until `unload_radius`.
#[derive(Clone, Copy, Debug)]
pub struct StreamRadius {
    pub load: f64,
    pub unload: f64,
}

impl StreamRadius {
    pub const DEFAULT: Self = Self {
        load: 720.0,
        unload: 720.0 + CHUNK_SIZE as f64,
    };
}

/// Distance in metres from `(x, z)` to the nearest point of `column`'s
/// square.
pub fn column_distance(column: ColumnKey, x: f64, z: f64) -> f64 {
    let size = CHUNK_SIZE as f64;
    let gap = |p: f64, c: i32| {
        let min = c as f64 * size;
        (min - p).max(p - (min + size)).max(0.0)
    };
    gap(x, column.0).hypot(gap(z, column.1))
}

struct Queue {
    /// Columns waiting for a worker, nearest first (distance in
    /// centimetres, so it orders as an integer).
    pending: BinaryHeap<Reverse<(u64, ColumnKey)>>,
    /// Columns the main thread still wants: a worker skips a queued column
    /// that has dropped out of it.
    wanted: HashSet<ColumnKey>,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

/// Loads columns around an anchor on worker threads; see the module.
pub struct Streamer<R> {
    shared: Arc<Shared>,
    results: Receiver<(ColumnKey, R)>,
    radius: StreamRadius,
    /// Columns handed out and not back yet (queued or being made).
    requested: HashSet<ColumnKey>,
    /// Columns whose result was taken.
    loaded: HashSet<ColumnKey>,
    /// The column the anchor was in at the last update, and where it was.
    anchor: Option<(ColumnKey, [f64; 2])>,
}

impl<R: Send + 'static> Streamer<R> {
    /// Start `workers` threads that make columns with `make`.
    pub fn new(
        workers: usize,
        radius: StreamRadius,
        make: impl Fn(ColumnKey) -> R + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                pending: BinaryHeap::new(),
                wanted: HashSet::new(),
                stop: false,
            }),
            ready: Condvar::new(),
        });
        let (tx, results) = mpsc::channel();
        let make = Arc::new(make);
        for _ in 0..workers.max(1) {
            let (shared, tx, make) = (shared.clone(), tx.clone(), make.clone());
            thread::spawn(move || Self::work(&shared, &tx, &*make));
        }
        Self {
            shared,
            results,
            radius,
            requested: HashSet::new(),
            loaded: HashSet::new(),
            anchor: None,
        }
    }

    /// Worker threads to use on this machine: all cores but the main
    /// thread's, at least two.
    pub fn default_workers() -> usize {
        thread::available_parallelism()
            .map(|n| n.get().saturating_sub(1))
            .unwrap_or(2)
            .max(2)
    }

    fn work(shared: &Shared, tx: &Sender<(ColumnKey, R)>, make: &(dyn Fn(ColumnKey) -> R + Sync)) {
        loop {
            let column = {
                let mut queue = shared.queue.lock().unwrap();
                loop {
                    if queue.stop {
                        return;
                    }
                    match queue.pending.pop() {
                        Some(Reverse((_, column))) if queue.wanted.contains(&column) => break column,
                        // No longer wanted: the main thread has forgotten it.
                        Some(_) => continue,
                        None => queue = shared.ready.wait(queue).unwrap(),
                    }
                }
            };
            if tx.send((column, make(column))).is_err() {
                return;
            }
        }
    }

    /// Follow the anchor to world `(x, z)`: columns now in reach are
    /// queued (nearest first), and those out of reach are dropped. Returns
    /// the loaded columns to let go of; they count as unloaded from now on.
    pub fn update(&mut self, x: f64, z: f64) -> Vec<ColumnKey> {
        let here = ((x / CHUNK_SIZE as f64).floor() as i32, (z / CHUNK_SIZE as f64).floor() as i32);
        // Within a column nothing changes but the order, which is decided
        // when the anchor enters it: no need to redo it every frame.
        if self.anchor.is_some_and(|(column, _)| column == here) {
            return Vec::new();
        }
        self.anchor = Some((here, [x, z]));
        let reach = (self.radius.unload / CHUNK_SIZE as f64).ceil() as i32 + 1;
        let mut wanted = HashSet::new();
        let mut pending = BinaryHeap::new();
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let column = (here.0 + dx, here.1 + dz);
                let distance = column_distance(column, x, z);
                if distance <= self.radius.load {
                    wanted.insert(column);
                    if !self.loaded.contains(&column) && !self.requested.contains(&column) {
                        pending.push(Reverse(((distance * 100.0) as u64, column)));
                    }
                }
            }
        }
        // Keep what is loaded or on its way while it is within the unload
        // radius (so stepping back and forth over an edge reloads
        // nothing); still wanted, a queued column keeps its place too.
        let mut unload = Vec::new();
        for &column in self.loaded.iter() {
            if column_distance(column, x, z) > self.radius.unload {
                unload.push(column);
            }
        }
        for column in &unload {
            self.loaded.remove(column);
        }
        let mut queue = self.shared.queue.lock().unwrap();
        // Requested columns between the radii stay wanted.
        for &column in &self.requested {
            if column_distance(column, x, z) <= self.radius.unload {
                wanted.insert(column);
            }
        }
        // Requested but not wanted any more: a worker will skip it, or its
        // result is dropped when it comes.
        self.requested.retain(|column| wanted.contains(column));
        // Queued columns are ordered afresh from where the anchor is now.
        let requeue: Vec<_> = queue
            .pending
            .drain()
            .map(|Reverse((_, column))| column)
            .filter(|column| wanted.contains(column))
            .collect();
        for column in requeue {
            pending.push(Reverse(((column_distance(column, x, z) * 100.0) as u64, column)));
        }
        for Reverse((_, column)) in pending.iter() {
            self.requested.insert(*column);
        }
        queue.pending = pending;
        queue.wanted = wanted;
        drop(queue);
        self.shared.ready.notify_all();
        unload.sort_unstable();
        unload
    }

    /// Columns made since the last call that are still wanted, taken for
    /// up to `budget` (at least one, if any is ready).
    pub fn receive(&mut self, budget: Duration) -> Vec<(ColumnKey, R)> {
        let start = Instant::now();
        let mut out = Vec::new();
        while let Ok((column, result)) = self.results.try_recv() {
            if !self.requested.remove(&column) {
                // Dropped while it was being made.
                continue;
            }
            self.loaded.insert(column);
            out.push((column, result));
            if start.elapsed() >= budget {
                break;
            }
        }
        out
    }

    /// Whether `column` is loaded (its result was taken).
    pub fn is_loaded(&self, column: ColumnKey) -> bool {
        self.loaded.contains(&column)
    }

    /// Columns queued or being made.
    pub fn in_flight(&self) -> usize {
        self.requested.len()
    }

    pub fn loaded_count(&self) -> usize {
        self.loaded.len()
    }
}

impl<R> Drop for Streamer<R> {
    fn drop(&mut self) {
        // The workers finish the column at hand and stop; what they make
        // after this goes nowhere.
        self.shared.queue.lock().unwrap().stop = true;
        self.shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for<R: Send + 'static>(streamer: &mut Streamer<R>, count: usize) -> Vec<(ColumnKey, R)> {
        let start = Instant::now();
        let mut got = Vec::new();
        while got.len() < count && start.elapsed() < Duration::from_secs(10) {
            got.extend(streamer.receive(Duration::from_secs(1)));
            thread::sleep(Duration::from_millis(1));
        }
        got
    }

    fn within(radius: f64, x: f64, z: f64) -> usize {
        let reach = (radius / CHUNK_SIZE as f64).ceil() as i32 + 2;
        let mut n = 0;
        for cz in -reach..=reach {
            for cx in -reach..=reach {
                let column = (cx + (x / 128.0).floor() as i32, cz + (z / 128.0).floor() as i32);
                if column_distance(column, x, z) <= radius {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn the_distance_to_a_column_is_to_its_nearest_point() {
        assert_eq!(column_distance((0, 0), 64.0, 64.0), 0.0);
        assert_eq!(column_distance((1, 0), 100.0, 64.0), 28.0);
        assert_eq!(column_distance((-1, -1), 3.0, 4.0), 5.0);
    }

    #[test]
    fn columns_around_the_anchor_load_once_each() {
        let radius = StreamRadius { load: 300.0, unload: 428.0 };
        let mut streamer = Streamer::new(3, radius, |column| column);
        assert!(streamer.update(10.0, 10.0).is_empty());
        let expected = within(300.0, 10.0, 10.0);
        let got = wait_for(&mut streamer, expected);
        assert_eq!(got.len(), expected);
        let unique: HashSet<_> = got.iter().map(|(c, _)| *c).collect();
        assert_eq!(unique.len(), expected);
        assert!(got.iter().all(|(c, r)| c == r && column_distance(*c, 10.0, 10.0) <= 300.0));
        // Nothing more comes for the same place.
        streamer.update(20.0, 30.0);
        thread::sleep(Duration::from_millis(50));
        assert!(streamer.receive(Duration::from_secs(1)).is_empty());
        assert_eq!(streamer.in_flight(), 0);
    }

    #[test]
    fn the_nearest_columns_come_first() {
        let radius = StreamRadius { load: 500.0, unload: 600.0 };
        // One worker, so the order is the queue's.
        let mut streamer = Streamer::new(1, radius, |column| column);
        streamer.update(-500.0, 900.0);
        let got = wait_for(&mut streamer, within(500.0, -500.0, 900.0));
        let distances: Vec<f64> = got.iter().map(|(c, _)| column_distance(*c, -500.0, 900.0)).collect();
        assert!(distances.windows(2).all(|w| w[0] <= w[1] + 1e-9), "{distances:?}");
    }

    #[test]
    fn columns_unload_past_the_unload_radius_and_not_before() {
        let radius = StreamRadius { load: 200.0, unload: 400.0 };
        let mut streamer = Streamer::new(2, radius, |column| column);
        streamer.update(0.0, 0.0);
        wait_for(&mut streamer, within(200.0, 0.0, 0.0));
        // A step past the load radius: everything stays.
        assert!(streamer.update(150.0, 0.0).is_empty());
        let start = Instant::now();
        while streamer.in_flight() > 0 && start.elapsed() < Duration::from_secs(10) {
            streamer.receive(Duration::from_secs(1));
            thread::sleep(Duration::from_millis(1));
        }
        // Far away: all columns behind are let go, and only those.
        let unloaded = streamer.update(5000.0, 0.0);
        assert!(!unloaded.is_empty());
        assert!(unloaded.iter().all(|c| column_distance(*c, 5000.0, 0.0) > 400.0));
        assert!(unloaded.iter().all(|c| !streamer.is_loaded(*c)));
        let got = wait_for(&mut streamer, within(200.0, 5000.0, 0.0));
        assert!(got.iter().all(|(c, _)| column_distance(*c, 5000.0, 0.0) <= 200.0));
        assert_eq!(streamer.loaded_count(), within(200.0, 5000.0, 0.0));
    }

    #[test]
    fn columns_left_behind_while_queued_are_not_delivered() {
        let radius = StreamRadius { load: 600.0, unload: 700.0 };
        // Slow workers, so most columns are still queued when the anchor
        // jumps away.
        let mut streamer = Streamer::new(1, radius, |column| {
            thread::sleep(Duration::from_millis(2));
            column
        });
        streamer.update(0.0, 0.0);
        streamer.update(100_000.0, 0.0);
        let got = wait_for(&mut streamer, within(600.0, 100_000.0, 0.0));
        assert!(got.iter().all(|(c, _)| column_distance(*c, 100_000.0, 0.0) <= 600.0));
    }
}
