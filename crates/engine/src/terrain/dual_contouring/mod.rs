pub mod dual_contouring;
mod water;

use libnoise::{Fbm, Perlin, Scale};

use crate::terrain::ChunkMesh;

/// Y-coordinate (in local chunk space) below which terrain is considered underwater.
/// Must match the water threshold in the terrain vertex shader.
pub const WATER_LEVEL: f32 = 44.0;

/// Levels of detail of the ground: grids 1, 2, 4 and 8 metres apart.
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
    noise: Fbm<2, Scale<2, Perlin<2>>>,
    /// The ground at each level of detail, finest first (see
    /// `generate_meshes`); empty for chunks above the ground.
    meshes: Vec<ChunkMesh<Vertex>>,
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
