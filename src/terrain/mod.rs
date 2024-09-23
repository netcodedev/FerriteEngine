use std::{collections::HashMap, sync::mpsc};

use ndarray::ArrayBase;

use crate::{mesh::Mesh, shader::Shader, texture::Texture};

pub mod terrain;

pub const CHUNK_SIZE: usize = 128;

pub struct Block {
    pub type_id: u32,
}

pub struct Chunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, ndarray::Dim<[usize; 3]>>,
    pub mesh: Option<Mesh>,
}

#[derive(Eq, PartialEq, Hash, Debug)]
pub struct ChunkBounds {
    pub min: (i32, i32, i32),
    pub max: (i32, i32, i32),
}


pub struct Terrain {
    pub chunks: HashMap<ChunkBounds, Chunk>,
    chunk_receiver: mpsc::Receiver<Chunk>,
    shader: Shader,
    grass_texture: Texture,
    stone_texture: Texture,
}