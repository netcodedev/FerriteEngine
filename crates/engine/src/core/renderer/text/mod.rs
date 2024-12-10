use gl::types::GLuint;
use rusttype::{gpu_cache::Cache, PositionedGlyph};

use crate::core::renderer::shader::Shader;

use super::shader::DynamicVertexArray;

pub mod text;

pub struct Font {
    font: rusttype::Font<'static>,
}

pub enum Fonts {
    RobotoMono,
}

pub struct TextRenderer {
    cache: Cache<'static>,
    shader: Shader,
    texture_buffer: Texture,
    pub width: u32,
    height: u32,
}

pub struct Text {
    pub content: String,
    font: Fonts,
    size: f32,
    pub glyphs: Vec<PositionedGlyph<'static>>,
    dirty: bool,
    x: i32,
    y: i32,
    z: i32,
    pub mesh: TextMesh,
    pub max_x: i32,
    pub max_y: i32,
}

pub struct TextMesh {
    pub vertex_array: DynamicVertexArray<TextVertex>,
    vertices: Vec<TextVertex>,
}

#[derive(Clone)]
#[repr(C)]
pub struct TextVertex {
    position: (f32, f32, f32),
    texture_coords: (f32, f32),
}

struct Texture {
    id: GLuint,
}
