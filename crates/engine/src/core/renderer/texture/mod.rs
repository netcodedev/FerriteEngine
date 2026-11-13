use gl::types::{GLenum, GLuint};

use crate::core::renderer::shader::Shader;

pub mod texture;

pub struct Texture {
    pub id: GLuint,
    /// `TEXTURE_2D`, or `TEXTURE_2D_ARRAY` once loaded as an array.
    target: GLenum,
}

pub struct TextureRenderer {
    shader: Shader,
}
