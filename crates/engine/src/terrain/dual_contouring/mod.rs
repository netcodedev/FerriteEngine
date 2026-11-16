mod density;
pub mod dual_contouring;
pub mod water;

use std::sync::Arc;

use crate::terrain::{
    generator::{ChunkCoord, WorldGenerator},
    ChunkMesh,
};

/// Height of the water's surface, in world metres: the sea level. Must
/// match `WATER_LINE` in the terrain's fragment shader.
pub const WATER_LEVEL: f32 = crate::terrain::generator::SEA_LEVEL;

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
    /// Whether it has its caves and overhangs (it is made without them
    /// far away).
    caves: bool,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    /// Ground colour of the biomes here (display space).
    color: [f32; 3],
    /// Temperature (°C, at this height) and humidity (0..1).
    climate: [f32; 2],
    /// How much of the ground is forest floor, dry grass, jungle floor and
    /// moss (see `BiomeBlend`), 0..1 each.
    cover: [f32; 4],
    /// How much of the ground is road, settlement, gravel (see
    /// `Column::paint`) and mud (swamps), 0..1 each.
    paint: [f32; 4],
    /// How far it lies under the surface (caves, under overhangs), 0..1.
    underground: f32,
}
