use std::{collections::HashMap, sync::mpsc};

use glfw::MouseButton;

use crate::{
    camera::{Camera, Projection},
    line::Line,
    shader::{DynamicVertexArray, Shader, VertexAttributes},
    texture::Texture,
};

pub const CHUNK_SIZE: usize = 128;
pub const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;

pub mod terrain;

pub struct Terrain<T: Chunk> {
    pub chunks: HashMap<ChunkBounds, T>,
    chunk_receiver: mpsc::Receiver<T>,
    shader: Shader,
    textures: Vec<Texture>,
}

pub trait Chunk {
    fn new(position: (f32, f32, f32), lod: usize) -> Self;
    fn render(&mut self, camera: &Camera, projection: &Projection, shader: &Shader);
    fn get_bounds(&self) -> ChunkBounds;
    fn process_line(&mut self, line: &Line, button: &MouseButton) -> bool;
    fn get_shader_source() -> (String, String);
    fn get_textures() -> Vec<Texture>;
}

pub struct ChunkMesh<T: VertexAttributes> {
    vertex_array: Option<DynamicVertexArray<T>>,
    indices: Option<Vec<u32>>,
    vertices: Vec<T>,
}

#[derive(Eq, PartialEq, Hash, Debug)]
pub struct ChunkBounds {
    pub min: (i32, i32, i32),
    pub max: (i32, i32, i32),
}
