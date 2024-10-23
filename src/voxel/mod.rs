use std::{collections::HashMap, sync::mpsc};

use ndarray::ArrayBase;

use crate::{shader::{DynamicVertexArray, Shader}, terrain::ChunkBounds, texture::Texture};

pub mod voxel;

pub const CHUNK_SIZE: usize = 128;

pub struct Block {
    pub type_id: u32,
}

pub struct VoxelChunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, ndarray::Dim<[usize; 3]>>,
    pub mesh: Option<ChunkMesh>,
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

pub struct ChunkMesh {
    vertex_array: Option<DynamicVertexArray<BlockVertex>>,
    indices: Option<Vec<u32>>,
    vertices: Vec<BlockVertex>,
}


pub struct VoxelTerrain {
    pub chunks: HashMap<ChunkBounds, VoxelChunk>,
    chunk_receiver: mpsc::Receiver<VoxelChunk>,
    shader: Shader,
    grass_texture: Texture,
    stone_texture: Texture,
}