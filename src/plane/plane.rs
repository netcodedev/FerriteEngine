use crate::shader::{DynamicVertexArray, Shader, VertexAttributes};

use super::{Plane, PlaneRenderer, PlaneVertex};

impl PlaneRenderer {
    pub fn new() -> Self {
        Self {
            shader: Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"))
        }
    }
    pub fn render(&self, plane: Plane, width: u32, height: u32) {
        // calculate plane vertices
        let vertices = vec![
            PlaneVertex {
                position: (
                    plane.position.0,
                    plane.position.1 + plane.size.1,
                    plane.position.2
                ),
                color: plane.color,
                dimensions: (plane.size.0, plane.size.1, plane.position.0, plane.position.1)
            },
            PlaneVertex {
                position: (
                    plane.position.0 + plane.size.0,
                    plane.position.1 + plane.size.1,
                    plane.position.2
                ),
                color: plane.color,
                dimensions: (plane.size.0, plane.size.1, plane.position.0, plane.position.1)
            },
            PlaneVertex {
                position: (
                    plane.position.0 + plane.size.0,
                    plane.position.1,
                    plane.position.2
                ),
                color: plane.color,
                dimensions: (plane.size.0, plane.size.1, plane.position.0, plane.position.1)
            },
            PlaneVertex {
                position: (
                    plane.position.0,
                    plane.position.1,
                    plane.position.2
                ),
                color: plane.color,
                dimensions: (plane.size.0, plane.size.1, plane.position.0, plane.position.1)
            },
        ];
        let indices: Vec<u32> = vec![
            0, 1, 2,
            2, 3, 0,
        ];
        let mut vertex_array = DynamicVertexArray::<PlaneVertex>::new();
        vertex_array.buffer_data_dyn(&vertices, &Some(indices.clone()));
        vertex_array.bind();
        self.shader.bind();
        let ortho = cgmath::ortho(0.0, width as f32, height as f32, 0.0, -1.0, 100.0);
        self.shader.set_uniform_mat4("projection", &ortho);
        self.shader.set_uniform_1f("borderThickness", 1.0);
        self.shader.set_uniform_4f("borderRadius", 20.0, 40.0, 80.0, 160.0);
        self.shader.set_uniform_4f("borderColor", 1.0, 0.0, 0.0, 1.0);
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::DrawElements(gl::TRIANGLES, indices.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
        }
    }
}

impl VertexAttributes for PlaneVertex {
    fn get_vertex_attributes() -> Vec<(usize, gl::types::GLuint)> {
        vec![
            (3, gl::FLOAT),
            (4, gl::FLOAT),
            (4, gl::FLOAT),
        ]
    }
}