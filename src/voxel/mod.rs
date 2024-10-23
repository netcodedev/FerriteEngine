use ndarray::ArrayBase;

use crate::terrain::ChunkMesh;

pub mod voxel;

pub const CHUNK_SIZE: usize = 128;

pub struct Block {
    pub type_id: u32,
}

pub struct VoxelChunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, ndarray::Dim<[usize; 3]>>,
    pub mesh: Option<ChunkMesh<BlockVertex>>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
#[repr(C)]
pub struct BlockVertex {
    position: (f32, f32, f32),
    normal: (f32, f32, f32),
    texture_coords: (f32, f32),
    block_type: u32,
}
