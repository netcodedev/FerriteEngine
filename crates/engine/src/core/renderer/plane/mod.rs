use crate::core::renderer::shader::Shader;

use super::shader::DynamicVertexArray;

pub mod plane;

pub struct PlaneRenderer {
    shader: Shader,
    width: f32,
    height: f32,
}

pub struct Plane {
    position: (f32, f32, f32),
    pub size: (f32, f32),
    color: (f32, f32, f32, f32),
    pub border_thickness: f32,
    pub border_color: (f32, f32, f32, f32),
    pub border_radius: (f32, f32, f32, f32),
    vertex_array: DynamicVertexArray<PlaneVertex>,
}

#[derive(Clone, Copy)]
pub struct PlaneBuilder {
    position: (f32, f32, f32),
    size: (f32, f32),
    color: (f32, f32, f32, f32),
    border_thickness: f32,
    border_color: (f32, f32, f32, f32),
    border_radius: (f32, f32, f32, f32),
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PlaneVertex {
    pub position: (f32, f32, f32),
    pub color: (f32, f32, f32, f32),
    pub dimensions: (f32, f32, f32, f32),
}
