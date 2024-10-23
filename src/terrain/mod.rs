use glfw::MouseButton;

use crate::{camera::{Camera, Projection}, line::Line, shader::Shader};

pub const CHUNK_SIZE: usize = 128;
pub const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;

pub mod terrain;

pub trait Terrain {
    fn update(&mut self);
    fn render(&mut self, camera: &Camera, projection: &Projection);
    fn process_line(&mut self, line: Option<(Line, MouseButton)>);
}

pub trait Chunk {
    fn render(&mut self, camera: &Camera, projection: &Projection, shader: &Shader);
    fn get_bounds(&self) -> ChunkBounds;
}

#[derive(Eq, PartialEq, Hash, Debug)]
pub struct ChunkBounds {
    pub min: (i32, i32, i32),
    pub max: (i32, i32, i32),
}