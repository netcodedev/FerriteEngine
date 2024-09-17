use cgmath::{Matrix, Point3, Vector3};
use gl::types::*;
use crate::camera::{Camera, Projection};
use crate::shader::create_shader;

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
    shader: GLuint,
    vao: GLuint,
    vbo: GLuint,
}

impl LineRenderer {
    pub fn new() -> Self {
        let shader = create_shader(include_str!("shaders/line_vertex.glsl"), include_str!("shaders/line_fragment.glsl"));

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

    pub fn render(&self, camera: &Camera, projection: &Projection, line: &Line) {
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::UseProgram(self.shader);

            let view = camera.calc_matrix();
            let projection = projection.calc_matrix();

            let view_loc = gl::GetUniformLocation(self.shader, "view\0".as_ptr() as *const i8);
            gl::UniformMatrix4fv(view_loc, 1, gl::FALSE, view.as_ptr());

            let projection_loc = gl::GetUniformLocation(self.shader, "projection\0".as_ptr() as *const i8);
            gl::UniformMatrix4fv(projection_loc, 1, gl::FALSE, projection.as_ptr());

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