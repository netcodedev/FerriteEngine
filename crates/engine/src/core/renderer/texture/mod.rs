use gl::types::GLuint;

use crate::core::renderer::shader::Shader;

pub mod texture;

pub struct Texture {
    pub id: GLuint,
}

pub struct TextureRenderer {
    shader: Shader,
}
