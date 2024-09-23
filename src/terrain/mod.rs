use std::{collections::HashMap, sync::mpsc};

use ndarray::ArrayBase;

use crate::{shader::{DynamicVertexArray, Shader}, texture::Texture};

pub mod terrain;

pub const CHUNK_SIZE: usize = 128;

pub struct Block {
    pub type_id: u32,
}

pub struct Chunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, ndarray::Dim<[usize; 3]>>,
    pub mesh: Option<ChunkMesh>,
}

#[derive(Eq, PartialEq, Hash, Debug)]
pub struct ChunkBounds {
    pub min: (i32, i32, i32),
    pub max: (i32, i32, i32),
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


pub struct Terrain {
    pub chunks: HashMap<ChunkBounds, Chunk>,
    chunk_receiver: mpsc::Receiver<Chunk>,
    shader: Shader,
    grass_texture: Texture,
    stone_texture: Texture,
}