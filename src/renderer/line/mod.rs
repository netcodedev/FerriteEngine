use cgmath::{Point3, Vector3};
use gl::types::GLuint;

use crate::shader::Shader;

pub mod line;

#[derive(Clone)]
pub struct Line {
    pub position: Point3<f32>,
    pub direction: Vector3<f32>,
    pub length: f32,
}

pub struct LineRenderer {
    shader: Shader,
    vao: GLuint,
    vbo: GLuint,
}
