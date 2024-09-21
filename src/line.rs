use cgmath::{Point3, Vector3};
use gl::types::*;
use crate::camera::{Camera, Projection};
use crate::shader::Shader;

#[derive(Clone)]
pub struct Line {
    pub position: Point3<f32>,
    pub direction: Vector3<f32>,
    pub length: f32,
}

impl Line {
    pub fn new(position: Point3<f32>, direction: Vector3<f32>, length: f32) -> Self {
        Self {
            position,
            direction,
            length
        }
    }
}

pub struct LineRenderer {
    shader: Shader,
    vao: GLuint,
    vbo: GLuint,
}

impl LineRenderer {
    pub fn new() -> Self {
        let shader = Shader::new(include_str!("shaders/line_vertex.glsl"), include_str!("shaders/line_fragment.glsl"));

        let mut vao = 0;
        let mut vbo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);

            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);

            gl::VertexAttribPointer(0, 3, gl::FLOAT, gl::FALSE, 3 * std::mem::size_of::<GLfloat>() as GLsizei, std::ptr::null());
            gl::EnableVertexAttribArray(0);

            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }

        Self {
            shader,
            vao,
            vbo,
        }
    }

    pub fn render(&self, camera: &Camera, projection: &Projection, line: &Line, color: Vector3<f32>, always_on_top: bool) {
        unsafe {
            if always_on_top {
                gl::Disable(gl::DEPTH_TEST);
            } else {
                gl::Enable(gl::DEPTH_TEST);
            }
            self.shader.bind();

            let view = camera.calc_matrix();
            let projection = projection.calc_matrix();

            self.shader.set_uniform_mat4("view", &view);
            self.shader.set_uniform_mat4("projection", &projection);
            self.shader.set_uniform_3fv("color", &color);

            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);

            let end = line.position + line.direction * line.length;
            let lines = vec![
                line.position.x, line.position.y, line.position.z,
                end.x, end.y, end.z,
            ];

            gl::BufferData(gl::ARRAY_BUFFER, (lines.len() * std::mem::size_of::<GLfloat>()) as GLsizeiptr, lines.as_ptr() as *const _, gl::STATIC_DRAW);
            gl::DrawArrays(gl::LINES, 0, (lines.len() / 3) as i32);

            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
            gl::Disable(gl::DEPTH_TEST);
        }
    }
}