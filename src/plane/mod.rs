use crate::shader::Shader;

pub mod plane;

pub struct PlaneRenderer {
    shader: Shader
}

pub struct Plane {
    pub position: (f32, f32, f32),
    pub size: (f32, f32),
    pub color: (f32, f32, f32, f32)
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PlaneVertex {
    pub position: (f32, f32, f32),
    pub color: (f32, f32, f32, f32),
    pub dimensions: (f32, f32)
}