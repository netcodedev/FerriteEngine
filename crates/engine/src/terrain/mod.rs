use std::sync::mpsc;

use cgmath::{Matrix4, Point3};
use glfw::MouseButton;

use crate::core::{
    mouse_picker::MousePicker,
    renderer::{
        line::Line,
        shader::{DynamicVertexArray, Shader, VertexAttributes},
        texture::Texture,
    },
    scene::Scene,
};

pub const CHUNK_RADIUS: usize = 5;
pub const CHUNK_SIZE: usize = 128;
pub const CHUNK_SIZE_FLOAT: f32 = CHUNK_SIZE as f32;
pub const USE_LOD: bool = false;

pub mod dual_contouring;
pub mod marching_cubes;
pub mod materials;
pub mod scatter;
mod terrain;
pub mod voxel;

pub struct Terrain<T: Chunk> {
    seed: u64,
    /// Countdown until the chunks are rebuilt for an edited seed.
    reseed_in: Option<f64>,
    /// Models scattered over the chunks. Optional; see `scatter`.
    scatter: scatter::Scatter,
    /// Editor toggle: draw chunk and scatter colliders as wireframes.
    show_colliders: bool,
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
    /// Textures bound to units 0, 1, ... while the terrain draws. May set
    /// uniforms of `shader` that stay fixed.
    fn get_textures(shader: &Shader) -> Vec<Texture>;
    fn get_triangle_count(&self) -> usize;
    fn get_vertices(&self) -> Vec<[f32; 3]>;
    fn get_indices(&self) -> Vec<[u32; 3]>;

    /// Spots on this chunk where scattered models (trees, rocks, ...) may
    /// stand, as world-space `(x, y, z, yaw_radians)`. `attempts` random
    /// spots are tried and the unsuitable ones dropped; `salt` varies the
    /// pattern so each model gets its own. Must be deterministic for a given
    /// chunk, salt and attempt count. The default is empty — only terrain
    /// backends that support scattering implement this.
    fn scatter_points(&self, _salt: u64, _attempts: u32) -> Vec<(f32, f32, f32, f32)> {
        Vec::new()
    }

    /// Optional transparent/overlay pass rendered after ALL chunks' opaque geometry.
    /// Override this to render water, glass, etc. Default is a no-op.
    fn render_transparent(
        &self,
        _scene: &Scene,
        _view_projection: &Matrix4<f32>,
        _parent_transform: &Matrix4<f32>,
    ) {}
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
