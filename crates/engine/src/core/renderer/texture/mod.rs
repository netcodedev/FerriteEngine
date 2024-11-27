use gl::types::GLuint;

use crate::core::renderer::shader::Shader;

pub mod texture;

pub struct Texture {
    pub id: GLuint,
}

#[allow(dead_code)]
pub struct TextureRenderer {
    shader: Shader,
}
