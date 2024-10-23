pub mod dual_contouring;

use libnoise::{Perlin, Scale};

use crate::terrain::ChunkMesh;

const CHUNK_SIZE: usize = 128;
const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;
const ISO_VALUE: f32 = 0.3;

pub struct DualContouringChunk {
    position: (f32, f32, f32),
    cave: Scale<3, Perlin<3>>,
    noises: [Scale<2, Perlin<2>>; 3],
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
