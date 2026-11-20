use cgmath::{Point3, Vector3};

use crate::core::{gfx, renderer::shader::Shader};

pub mod line;

#[derive(Clone, Debug)]
pub struct Line {
    pub position: Point3<f32>,
    pub direction: Vector3<f32>,
    pub length: f32,
}

pub struct LineRenderer {
    shader: Shader,
    /// The ends of the lines drawn last.
    vertices: gfx::Buffer,
}
