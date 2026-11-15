mod density;
pub mod dual_contouring;
mod water;

use std::sync::Arc;

use crate::terrain::{
    generator::{ChunkCoord, WorldGenerator},
    ChunkMesh,
};

/// Y-coordinate (in local chunk space) below which terrain is considered underwater.
/// Must match the water threshold in the terrain vertex shader.
pub const WATER_LEVEL: f32 = 44.0;

/// Levels of detail of the ground: the density field meshed from samples
/// 1, 2, 4 and 8 metres apart.
const LODS: usize = 4;
/// Up to which distance from the camera (to the nearest point of a chunk,
/// in metres) each level but the last is drawn.
const LOD_DISTANCES: [f32; LODS - 1] = [160.0, 320.0, 560.0];
/// How deep the skirts under a chunk's edges reach: this, plus this per
/// metre between the level's grid points.
const SKIRT_DEPTH: f32 = 4.0;
const SKIRT_DEPTH_PER_STEP: f32 = 2.0;

pub struct DualContouringChunk {
    position: (f32, f32, f32),
    coord: ChunkCoord,
    generator: Arc<WorldGenerator>,
    /// The ground at each level of detail, finest first (see
    /// `generate_meshes`); empty for chunks above the ground.
    meshes: Vec<ChunkMesh<Vertex>>,
    /// Indices of the finest mesh that are ground; its skirt follows.
    ground_indices: usize,
    /// World box around every level's mesh, skirts and all.
    drawn_bounds: crate::terrain::ChunkBounds,
    /// Water surface grid, present only when some terrain in this chunk
    /// comes near WATER_LEVEL. It reaches under the shore (see `water`).
    water_mesh: Option<ChunkMesh<water::WaterVertex>>,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}
