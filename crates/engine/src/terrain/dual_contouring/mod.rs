pub mod dual_contouring;

use libnoise::{Fbm, Perlin, Scale};

use crate::terrain::ChunkMesh;

pub struct DualContouringChunk {
    position: (f32, f32, f32),
    cave: Scale<3, Perlin<3>>,
    noise: Fbm<2, Scale<2, Perlin<2>>>,
    chunk_size: usize,
    mesh: Option<ChunkMesh<Vertex>>,
}

#[derive(Clone, Copy)]
#[warn(dead_code)]
#[repr(C)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}
