//! The horizon: the ground beyond the loaded chunks, out to many
//! kilometres, as one height field mesh. It is made from the source's
//! samples (`TerrainSource::horizon_sample`) on a few nested grids, each twice as coarse as the one inside
//! it (like a geometry clipmap), around a centre that follows the camera
//! in steps, on a thread of its own.
//!
//! Where chunks are loaded the horizon sinks under them (in the vertex
//! shaders, from the camera's live position), so the two never fight. The
//! water beyond the chunks (the sea, lakes) is a mesh of its own, drawn
//! like the chunks' water (see `build`), over the ground under it.

use std::sync::{
    mpsc::{self, Receiver},
    Arc,
};

use super::{mesher::TerrainVertex, source::TerrainSource, water::WaterVertex, ChunkMesh};

/// Metres between the samples of the finest grid (a multiple of
/// `MACRO_STEP`, so samples fall on the large shape's grid).
pub const FINEST_STEP: f64 = 32.0;
/// Cells from the centre to the edge of each grid.
const HALF_CELLS: i32 = 64;
/// Grids, each twice as coarse and twice as wide as the one inside it.
pub const LEVELS: usize = 4;
/// The centre moves in steps of this many metres (twice the coarsest
/// grid's step, so every grid stays on its own lattice).
pub const CENTRE_STEP: f64 = FINEST_STEP * (1 << LEVELS) as f64;
/// Typical width of the sea and of lakes for the far water's waves (see
/// `WaterVertex::body_size`).
const SEA_SIZE: f32 = 1000.0;
const LAKE_SIZE: f32 = 300.0;
/// Ground up to this far above the water still gets far water over it, so
/// its edge lies under the shore.
const SHORE_MARGIN: f32 = 2.0;

/// What `build` makes: the ground, and the water over it.
pub struct Built {
    pub ground: (Vec<HorizonVertex>, Vec<u32>),
    pub water: (Vec<WaterVertex>, Vec<u32>),
}

/// The horizon's vertices: the chunks', so the terrain's programs draw it.
pub type HorizonVertex = TerrainVertex;

/// Distance in metres from the centre to the horizon's edge.
pub fn reach() -> f64 {
    FINEST_STEP * HALF_CELLS as f64 * (1 << (LEVELS - 1)) as f64
}

/// The horizon around `centre` (world `(x, z)`, a multiple of
/// `CENTRE_STEP`): the ground's vertices in world metres and triangles
/// facing up, and the water's (the sea and lakes, flat at their levels)
/// over the cells where the ground comes near them.
pub fn build(source: &dyn TerrainSource, centre: [f64; 2]) -> Built {
    let sea = source.sea_level();
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut water_vertices = Vec::new();
    let mut water_indices = Vec::new();
    for level in 0..LEVELS {
        let step = FINEST_STEP * (1 << level) as f64;
        let n = 2 * HALF_CELLS + 1;
        // Heights one sample beyond the grid, for the normals.
        let side = n + 2;
        let at = |i: i32| (i - 1 - HALF_CELLS) as f64 * step;
        let mut samples = Vec::with_capacity((side * side) as usize);
        for j in 0..side {
            for i in 0..side {
                let sample = source.horizon_sample(centre[0] + at(i), centre[1] + at(j));
                samples.push((sample.height, sample.surface, sample.water));
            }
        }
        // The ground (under the water too: the water is drawn over it).
        let height = |i: i32, j: i32| samples[((j + 1) * side + i + 1) as usize].0;
        // On the edge a coarser grid meets, every other vertex lies on the
        // coarse grid's edge between its neighbours: no cracks.
        let outer = level + 1 < LEVELS;
        let edge_height = |i: i32, j: i32| {
            let on_edge_x = i == 0 || i == n - 1;
            let on_edge_z = j == 0 || j == n - 1;
            if outer && on_edge_z && i % 2 == 1 {
                0.5 * (height(i - 1, j) + height(i + 1, j))
            } else if outer && on_edge_x && j % 2 == 1 {
                0.5 * (height(i, j - 1) + height(i, j + 1))
            } else {
                height(i, j)
            }
        };
        let first = vertices.len() as u32;
        for j in 0..n {
            for i in 0..n {
                let (_, surface, _) = samples[((j + 1) * side + i + 1) as usize];
                let y = edge_height(i, j);
                let dx = height(i + 1, j) - height(i - 1, j);
                let dz = height(i, j + 1) - height(i, j - 1);
                let normal = [-dx, 2.0 * step as f32, -dz];
                let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                vertices.push(HorizonVertex {
                    position: [(centre[0] + at(i + 1)) as f32, y, (centre[1] + at(j + 1)) as f32],
                    normal: normal.map(|c| c / length),
                    color: surface.color,
                    data: surface.data,
                });
            }
        }
        // The inner grid covers the middle of every grid but the finest.
        let hole = if level == 0 { 0 } else { HALF_CELLS / 2 };
        let covered = |i: i32, j: i32| {
            let (ci, cj) = (i - HALF_CELLS, j - HALF_CELLS);
            level > 0 && ci >= -hole && ci < hole && cj >= -hole && cj < hole
        };
        // The water over this grid: cells with a corner near or under it,
        // flat at its level (a dry corner takes its wet neighbours' level).
        let sample = |i: i32, j: i32| samples[((j + 1) * side + i + 1) as usize];
        let wet = |i: i32, j: i32| {
            let (h, _, water) = sample(i, j);
            h < water + SHORE_MARGIN
        };
        let surface = |i: i32, j: i32| {
            let (_, _, own) = sample(i, j);
            if wet(i, j) {
                return own;
            }
            let mut best: Option<f32> = None;
            for dj in -1..=1 {
                for di in -1..=1 {
                    if wet(i + di, j + dj) {
                        let l = sample(i + di, j + dj).2;
                        best = Some(best.map_or(l, |b: f32| b.max(l)));
                    }
                }
            }
            best.unwrap_or(own)
        };
        let mut index_of = vec![u32::MAX; (n * n) as usize];
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                if covered(i, j) || !(0..2).any(|dj| (0..2).any(|di| wet(i + di, j + dj))) {
                    continue;
                }
                let mut corner = |i: i32, j: i32| {
                    let slot = &mut index_of[(j * n + i) as usize];
                    if *slot == u32::MAX {
                        let (h, _, _) = sample(i, j);
                        let level = surface(i, j);
                        *slot = water_vertices.len() as u32;
                        water_vertices.push(WaterVertex {
                            position: [(centre[0] + at(i + 1)) as f32, level, (centre[1] + at(j + 1)) as f32],
                            depth: level - h,
                            body_size: if sea.is_some_and(|sea| level <= sea + 0.01) { SEA_SIZE } else { LAKE_SIZE },
                            flow: [0.0; 2],
                        });
                    }
                    *slot
                };
                let (a, b, c, d) = (corner(i, j), corner(i + 1, j), corner(i, j + 1), corner(i + 1, j + 1));
                water_indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                if covered(i, j) {
                    continue;
                }
                let v = |i: i32, j: i32| first + (j * n + i) as u32;
                let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i, j + 1), v(i + 1, j + 1));
                // Up-facing, as the chunks' triangles.
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    Built {
        ground: (vertices, indices),
        water: (water_vertices, water_indices),
    }
}

/// The horizon the terrain draws, made again on a thread of its own when
/// the camera has moved a step.
pub struct Horizon {
    mesh: Option<ChunkMesh<HorizonVertex>>,
    /// The water beyond the chunks.
    water: Option<ChunkMesh<WaterVertex>>,
    /// Centre of the mesh drawn, and of the one being made.
    centre: Option<[f64; 2]>,
    pending: Option<([f64; 2], Receiver<Built>)>,
}

impl Horizon {
    pub fn new() -> Self {
        Self {
            mesh: None,
            water: None,
            centre: None,
            pending: None,
        }
    }

    /// Forget the mesh (the world changed).
    pub fn invalidate(&mut self) {
        self.mesh = None;
        self.water = None;
        self.centre = None;
        self.pending = None;
    }

    /// Follow the camera at world `(x, z)`.
    pub fn update(&mut self, source: &Arc<dyn TerrainSource>, x: f64, z: f64) {
        if let Some((centre, rx)) = &self.pending {
            if let Ok(built) = rx.try_recv() {
                let (vertices, indices) = built.ground;
                let mut mesh = ChunkMesh::new(vertices, Some(indices));
                mesh.buffer_and_release();
                self.mesh = Some(mesh);
                let (vertices, indices) = built.water;
                self.water = (!indices.is_empty()).then(|| {
                    let mut water = ChunkMesh::new(vertices, Some(indices));
                    water.buffer_and_release();
                    water
                });
                self.centre = Some(*centre);
                self.pending = None;
            }
        }
        let snap = |v: f64| (v / CENTRE_STEP).round() * CENTRE_STEP;
        let wanted = [snap(x), snap(z)];
        let current = self.pending.as_ref().map(|(c, _)| *c).or(self.centre);
        if current != Some(wanted) {
            let (tx, rx) = mpsc::channel();
            let source = source.clone();
            std::thread::spawn(move || {
                let _ = tx.send(build(&*source, wanted));
            });
            self.pending = Some((wanted, rx));
        }
    }

    pub fn mesh(&self) -> Option<&ChunkMesh<HorizonVertex>> {
        self.mesh.as_ref()
    }

    /// The water beyond the chunks, if there is any in reach.
    pub fn water(&self) -> Option<&ChunkMesh<WaterVertex>> {
        self.water.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grids_meet_without_cracks_and_face_up() {
        use crate::terrain::{hills::{Hills, HillsSettings}, source::TerrainGenerator};
        let source = Hills::create(1, HillsSettings::default());
        let (vertices, indices) = build(&*source, [-2048.0, -6144.0]).ground;
        assert_eq!(vertices.len(), LEVELS * (2 * HALF_CELLS as usize + 1).pow(2));
        for t in indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| vertices[i as usize].position);
            let e1 = [b[0] - a[0], b[2] - a[2]];
            let e2 = [c[0] - a[0], c[2] - a[2]];
            assert!(e1[1] * e2[0] - e1[0] * e2[1] > 0.0, "triangle facing down");
        }
        // Where a grid meets the coarser one around it, its edge vertices
        // lie on the coarser grid's edges: a vertex halfway between two of
        // the coarse grid has the mean of their heights.
        let n = 2 * HALF_CELLS as usize + 1;
        let fine = &vertices[..n * n];
        let coarse = &vertices[n * n..2 * n * n];
        let find = |grid: &[HorizonVertex], x: f32, z: f32| {
            grid.iter().find(|v| v.position[0] == x && v.position[2] == z).map(|v| v.position[1])
        };
        for i in (0..n - 2).step_by(2) {
            let (a, mid, b) = (fine[i].position, fine[i + 1].position, fine[i + 2].position);
            let (ca, cb) = (find(coarse, a[0], a[2]).unwrap(), find(coarse, b[0], b[2]).unwrap());
            assert!((mid[1] - 0.5 * (ca + cb)).abs() < 1e-3, "crack at {mid:?}");
        }
        assert!(reach() >= 16000.0);
    }
}
