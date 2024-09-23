use gl::types::GLuint;
use rusttype::{gpu_cache::Cache, Font};

use crate::shader::Shader;

pub mod text;

pub struct TextRenderer {
    font: Font<'static>,
    cache: Cache<'static>,
    shader: Shader,
    texture_buffer: Texture,
    width: u32,
    height: u32,
}

struct Texture {
    id: GLuint,
}