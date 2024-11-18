use std::sync::mpsc;

use cgmath::Point3;
use glfw::MouseButton;

use crate::core::{
    mouse_picker::MousePicker,
    renderer::{
        line::Line,
        shader::{DynamicVertexArray, Shader, VertexAttributes},
        texture::Texture,
    },
};

pub const CHUNK_RADIUS: usize = 5;
pub const CHUNK_SIZE: usize = 128;
pub const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;
pub const USE_LOD: bool = false;

pub mod dual_contouring;
pub mod marching_cubes;
mod terrain;
pub mod voxel;

pub struct Terrain<T: Chunk> {
    chunk_receiver: mpsc::Receiver<T>,
    shader: Shader,
    textures: Vec<Texture>,
    mouse_picker: MousePicker,
}

pub trait Chunk {
    fn new(seed: u64, position: (f32, f32, f32), lod: usize) -> Self;
    fn buffer_data(&mut self);
    fn get_bounds(&self) -> ChunkBounds;
    fn process_line(&mut self, line: &Line, button: &MouseButton) -> bool;
    fn get_position(&self) -> Point3<f32>;
    fn get_shader_source() -> (String, String);
    fn get_textures() -> Vec<Texture>;
    fn get_triangle_count(&self) -> usize;
    fn get_vertices(&self) -> Vec<[f32; 3]>;
    fn get_indices(&self) -> Vec<[u32; 3]>;
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
