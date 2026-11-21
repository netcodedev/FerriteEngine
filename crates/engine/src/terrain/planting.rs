//! Where scattered models grow, worked out off the main thread: sampling a
//! world's height and chances for every spot of a chunk took several
//! milliseconds per chunk and layer on the frame that planted it.
//!
//! The terrain asks for the placements of a chunk and output (`take`) and
//! picks them up a frame or more later (`take`); the instances and their
//! colliders are made on the main thread from them, as before.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
};

use super::{
    scatter::{
        graph::{evaluate, NodeId, Placement, Rect, ScatterGraph},
        ScatterSpec,
    },
    source::ChunkCoord,
    TerrainSource, CHUNK_SIZE_FLOAT,
};

/// Worker threads (the chunk loaders have their own).
const WORKERS: usize = 2;

type Points = Vec<Placement>;
type Key = (ChunkCoord, u64);

struct Job {
    source: Arc<dyn TerrainSource>,
    generation: u64,
    coord: ChunkCoord,
    graph: Arc<ScatterGraph>,
    output: NodeId,
    salt: u64,
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
                let points = placements_at(&*job.source, job.coord, &job.graph, job.output);
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

    /// The placements of `spec` on `coord`, once worked out; asked for
    /// (once) otherwise.
    pub(super) fn take(&mut self, source: &Arc<dyn TerrainSource>, coord: ChunkCoord, spec: &ScatterSpec) -> Option<Points> {
        let key = (coord, spec.salt);
        if let Some(points) = self.ready.remove(&key) {
            return Some(points);
        }
        if self.pending.insert(key) {
            let job = Job {
                source: Arc::clone(source),
                generation: self.generation,
                coord,
                graph: spec.graph.clone(),
                output: spec.output,
                salt: spec.salt,
            };
            if self.jobs.send(job).is_err() {
                // No worker (it cannot start): work it out here.
                self.pending.remove(&key);
                return Some(placements_at(&**source, coord, &spec.graph, spec.output));
            }
        }
        None
    }

    /// Drop points kept for chunks that are not there any more.
    pub(super) fn retain(&mut self, keep: impl Fn(ChunkCoord) -> bool) {
        self.ready.retain(|(coord, _), _| keep(*coord));
    }
}

/// The placements of `output` of `graph` on the chunk `coord`: the graph
/// worked out on the chunk's column, kept where they stand within the
/// chunk's height.
pub fn placements_at(source: &dyn TerrainSource, coord: ChunkCoord, graph: &ScatterGraph, output: NodeId) -> Vec<Placement> {
    let floor = coord.floor();
    let ceiling = floor + CHUNK_SIZE_FLOAT;
    match evaluate(graph, source, Rect::column(coord.x, coord.z)) {
        Ok(evaluation) => evaluation
            .placements(output)
            .iter()
            .filter(|p| p.position[1] >= floor && p.position[1] < ceiling)
            .copied()
            .collect(),
        Err(e) => {
            log::warn!("terrain scatter: {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{
        chunk::scatter_points_at,
        hills::{Hills, HillsSettings},
        scatter::ScatterLayerData,
        source::TerrainGenerator,
    };

    #[test]
    fn a_layer_as_a_graph_plants_each_chunk_as_before() {
        let source = Hills::create(5, HillsSettings::default());
        let mut layer = ScatterLayerData::file("Trees/Trees.gltf", 40.0);
        layer.tag = "tree".into();
        let salt = 0xfe11;
        let graph = ScatterGraph::from_layer(&layer, salt);
        let output = graph.outputs().next().unwrap().0;
        let attempts = (40.0f32 * 128.0 * 128.0 / 10_000.0).round() as u32;
        let mut planted = 0;
        for (cx, cz) in [(0, 0), (2, 1), (-3, 4)] {
            for cy in source.vertical_range((cx, cz), false) {
                let coord = ChunkCoord::new(cx, cy, cz);
                let now: Vec<(f32, f32, f32, f32)> = placements_at(&*source, coord, &graph, output)
                    .iter()
                    .map(|p| (p.position[0], p.position[1], p.position[2], p.yaw))
                    .collect();
                let before = scatter_points_at(&*source, coord, "tree", salt, attempts);
                assert_eq!(now, before, "chunk {coord:?}");
                planted += now.len();
            }
        }
        assert!(planted > 0);
    }
}
