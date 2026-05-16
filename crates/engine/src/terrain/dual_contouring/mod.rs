pub mod dual_contouring;

use libnoise::{Fbm, Perlin, Scale};

use crate::terrain::ChunkMesh;

/// Y-coordinate (in local chunk space) below which terrain is considered underwater.
/// Must match the water threshold in the terrain vertex shader.
pub const WATER_LEVEL: f32 = 44.0;

pub struct DualContouringChunk {
    position: (f32, f32, f32),
    cave: Scale<3, Perlin<3>>,
    noise: Fbm<2, Scale<2, Perlin<2>>>,
    chunk_size: usize,
    mesh: Option<ChunkMesh<Vertex>>,
    /// Flat water-surface quad, present only when some terrain in this chunk
    /// falls below WATER_LEVEL.
    water_mesh: Option<ChunkMesh<Vertex>>,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}
