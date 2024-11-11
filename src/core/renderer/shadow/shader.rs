use crate::core::renderer::shader::Shader;

pub struct ShadowShader {
    pub shader: Shader,
}

impl ShadowShader {
    pub fn new() -> Self {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        Self { shader }
    }
}
