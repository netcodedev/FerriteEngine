//! Where scattered models grow, worked out off the main thread: sampling a
//! world's height and chances for every spot of a chunk took several
//! milliseconds per chunk and layer on the frame that planted it.
//!
//! The terrain asks for the points of a chunk and layer (`request`) and
//! picks them up a frame or more later (`take`); the instances and their
//! colliders are made on the main thread from them, as before.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
};

use super::{chunk::scatter_points_at, source::ChunkCoord, TerrainSource};

/// Worker threads (the chunk loaders have their own).
const WORKERS: usize = 2;

type Points = Vec<(f32, f32, f32, f32)>;
type Key = (ChunkCoord, u64);

struct Job {
    source: Arc<dyn TerrainSource>,
    generation: u64,
    coord: ChunkCoord,
    tag: String,
    salt: u64,
    attempts: u32,
}

struct Done {
    generation: u64,
    key: Key,
    points: Points,
}

pub(super) struct Planting {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// Asked for and not picked up yet.
    pending: HashSet<Key>,
    ready: HashMap<Key, Points>,
    /// Results of an older world are dropped.
    generation: u64,
}

impl Planting {
    pub(super) fn new() -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        for i in 0..WORKERS {
            let job_rx = Arc::clone(&job_rx);
            let done_tx = done_tx.clone();
            let _ = std::thread::Builder::new().name(format!("planting-{i}")).spawn(move || loop {
                let job = match job_rx.lock() {
                    Ok(rx) => rx.recv(),
                    Err(_) => return,
                };
                let Ok(job) = job else {
                    return;
                };
                let points = scatter_points_at(&*job.source, job.coord, &job.tag, job.salt, job.attempts);
                if done_tx.send(Done { generation: job.generation, key: (job.coord, job.salt), points }).is_err() {
                    return;
                }
            });
        }
        Planting { jobs, done, pending: HashSet::new(), ready: HashMap::new(), generation: 0 }
    }

    /// Forget everything asked for so far (the world changed).
    pub(super) fn reset(&mut self) {
        self.generation += 1;
        self.pending.clear();
        self.ready.clear();
    }

    /// Collect what the workers finished.
    pub(super) fn receive(&mut self) {
        while let Ok(done) = self.done.try_recv() {
            if done.generation == self.generation && self.pending.remove(&done.key) {
                self.ready.insert(done.key, done.points);
            }
        }
    }

    /// The points of the layer `salt` on `coord`, once worked out; asked
    /// for (once) otherwise.
    pub(super) fn take(
        &mut self,
        source: &Arc<dyn TerrainSource>,
        coord: ChunkCoord,
        tag: &str,
        salt: u64,
        attempts: u32,
    ) -> Option<Points> {
        let key = (coord, salt);
        if let Some(points) = self.ready.remove(&key) {
            return Some(points);
        }
        if self.pending.insert(key) {
            let job = Job { source: Arc::clone(source), generation: self.generation, coord, tag: tag.to_owned(), salt, attempts };
            if self.jobs.send(job).is_err() {
                // No worker (it cannot start): work it out here.
                self.pending.remove(&key);
                return Some(scatter_points_at(&**source, coord, tag, salt, attempts));
            }
        }
        None
    }

    /// Drop points kept for chunks that are not there any more.
    pub(super) fn retain(&mut self, keep: impl Fn(ChunkCoord) -> bool) {
        self.ready.retain(|(coord, _), _| keep(*coord));
    }
}
