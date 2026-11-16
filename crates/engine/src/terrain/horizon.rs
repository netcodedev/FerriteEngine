//! The horizon: the ground beyond the loaded chunks, out to many
//! kilometres, as one height field mesh. It is made from the generator's
//! samples on a few nested grids, each twice as coarse as the one inside
//! it (like a geometry clipmap), around a centre that follows the camera
//! in steps, on a thread of its own.
//!
//! Where chunks are loaded the horizon sinks under them (in the vertex
//! shaders, from the camera's live position), so the two never fight; the
//! sea beyond the chunks is drawn as a flat surface of its own colour.

use std::sync::{
    mpsc::{self, Receiver},
    Arc,
};

use gl::types::GLuint;

use super::{
    generator::{WorldGenerator, SEA_LEVEL},
    ChunkMesh,
};
use crate::core::renderer::shader::VertexAttributes;

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
/// Humidity a horizon vertex carries to say it is open sea (the shaders
/// colour it as water).
pub const SEA_MARK: f32 = 2.0;

/// The same layout as the chunks' vertices, so the terrain's programs draw
/// it.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct HorizonVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    climate: [f32; 2],
}

impl VertexAttributes for HorizonVertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (3, gl::FLOAT), (3, gl::FLOAT), (2, gl::FLOAT)]
    }
}

/// Distance in metres from the centre to the horizon's edge.
pub fn reach() -> f64 {
    FINEST_STEP * HALF_CELLS as f64 * (1 << (LEVELS - 1)) as f64
}

/// The horizon around `centre` (world `(x, z)`, a multiple of
/// `CENTRE_STEP`): vertices in world metres and triangles facing up.
pub fn build(generator: &WorldGenerator, centre: [f64; 2]) -> (Vec<HorizonVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for level in 0..LEVELS {
        let step = FINEST_STEP * (1 << level) as f64;
        let n = 2 * HALF_CELLS + 1;
        // Heights one sample beyond the grid, for the normals.
        let side = n + 2;
        let at = |i: i32| (i - 1 - HALF_CELLS) as f64 * step;
        let mut samples = Vec::with_capacity((side * side) as usize);
        for j in 0..side {
            for i in 0..side {
                samples.push(generator.grid_sample(centre[0] + at(i), centre[1] + at(j)));
            }
        }
        let height = |i: i32, j: i32| samples[((j + 1) * side + i + 1) as usize].0.max(SEA_LEVEL);
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
                let (h, biome) = samples[((j + 1) * side + i + 1) as usize];
                let y = edge_height(i, j);
                let dx = height(i + 1, j) - height(i - 1, j);
                let dz = height(i, j + 1) - height(i, j - 1);
                let normal = [-dx, 2.0 * step as f32, -dz];
                let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
                let sea = h < SEA_LEVEL;
                vertices.push(HorizonVertex {
                    position: [(centre[0] + at(i + 1)) as f32, y, (centre[1] + at(j + 1)) as f32],
                    normal: if sea { [0.0, 1.0, 0.0] } else { normal.map(|c| c / length) },
                    color: biome.color,
                    climate: [biome.temperature, if sea { SEA_MARK } else { biome.humidity }],
                });
            }
        }
        // The inner grid covers the middle of every grid but the finest.
        let hole = if level == 0 { 0 } else { HALF_CELLS / 2 };
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let (ci, cj) = (i - HALF_CELLS, j - HALF_CELLS);
                if level > 0 && ci >= -hole && ci < hole && cj >= -hole && cj < hole {
                    continue;
                }
                let v = |i: i32, j: i32| first + (j * n + i) as u32;
                let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i, j + 1), v(i + 1, j + 1));
                // Up-facing, as the chunks' triangles.
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    (vertices, indices)
}

/// The horizon the terrain draws, made again on a thread of its own when
/// the camera has moved a step.
pub struct Horizon {
    mesh: Option<ChunkMesh<HorizonVertex>>,
    /// Centre of the mesh drawn, and of the one being made.
    centre: Option<[f64; 2]>,
    pending: Option<([f64; 2], Receiver<(Vec<HorizonVertex>, Vec<u32>)>)>,
}

impl Horizon {
    pub fn new() -> Self {
        Self {
            mesh: None,
            centre: None,
            pending: None,
        }
    }

    /// Forget the mesh (the world changed).
    pub fn invalidate(&mut self) {
        self.mesh = None;
        self.centre = None;
        self.pending = None;
    }

    /// Follow the camera at world `(x, z)`.
    pub fn update(&mut self, generator: &Arc<WorldGenerator>, x: f64, z: f64) {
        if let Some((centre, rx)) = &self.pending {
            if let Ok((vertices, indices)) = rx.try_recv() {
                let mut mesh = ChunkMesh::new(vertices, Some(indices));
                mesh.buffer_and_release();
                self.mesh = Some(mesh);
                self.centre = Some(*centre);
                self.pending = None;
            }
        }
        let snap = |v: f64| (v / CENTRE_STEP).round() * CENTRE_STEP;
        let wanted = [snap(x), snap(z)];
        let current = self.pending.as_ref().map(|(c, _)| *c).or(self.centre);
        if current != Some(wanted) {
            let (tx, rx) = mpsc::channel();
            let generator = generator.clone();
            std::thread::spawn(move || {
                let _ = tx.send(build(&generator, wanted));
            });
            self.pending = Some((wanted, rx));
        }
    }

    pub fn mesh(&self) -> Option<&ChunkMesh<HorizonVertex>> {
        self.mesh.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grids_meet_without_cracks_and_face_up() {
        let generator = WorldGenerator::new(1);
        let (vertices, indices) = build(&generator, [-2048.0, -6144.0]);
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

    #[test]
    fn the_horizon_samples_the_ground_the_chunks_have() {
        let generator = WorldGenerator::new(3);
        for (x, z) in [(0.0, 0.0), (-4096.0, 1024.0), (9000.0 - 8.0, -3200.0)] {
            let (h, _) = generator.grid_sample(x, z);
            assert_eq!(h, generator.surface_height(x, z), "({x}, {z})");
        }
    }
}
