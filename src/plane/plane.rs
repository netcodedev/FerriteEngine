use crate::shader::{DynamicVertexArray, Shader, VertexAttributes};

use super::{Plane, PlaneBuilder, PlaneRenderer, PlaneVertex};

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
        self.shader.set_uniform_1f("borderThickness", plane.border_thickness);
        self.shader.set_uniform_4f("borderRadius", plane.border_radius.0, plane.border_radius.1, plane.border_radius.2, plane.border_radius.3);
        self.shader.set_uniform_4f("borderColor", plane.border_color.0, plane.border_color.1, plane.border_color.2, plane.border_color.3);
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::DrawElements(gl::TRIANGLES, indices.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
        }
    }
}

impl PlaneBuilder {
    pub fn new() -> Self {
        Self {
            position: (0.0, 0.0, 0.0),
            size: (0.0, 0.0),
            color: (0.0, 0.0, 0.0, 0.0),
            border_thickness: 0.0,
            border_color: (0.0, 0.0, 0.0, 1.0),
            border_radius: (0.0, 0.0, 0.0, 0.0)
        }
    }
    pub fn position(mut self, position: (f32, f32, f32)) -> Self {
        self.position = position;
        self
    }
    pub fn size(mut self, size: (f32, f32)) -> Self {
        self.size = size;
        self
    }
    pub fn color(mut self, color: (f32, f32, f32, f32)) -> Self {
        self.color = color;
        self
    }
    pub fn border_thickness(mut self, border_thickness: f32) -> Self {
        self.border_thickness = border_thickness;
        self
    }
    pub fn border_color(mut self, border_color: (f32, f32, f32, f32)) -> Self {
        self.border_color = border_color;
        self
    }
    pub fn border_radius(mut self, border_radius: (f32, f32, f32, f32)) -> Self {
        self.border_radius = (border_radius.2, border_radius.1, border_radius.3, border_radius.0);
        self
    }
    pub fn border_radius_uniform(mut self, border_radius: f32) -> Self {
        self.border_radius = (border_radius, border_radius, border_radius, border_radius);
        self
    }
    pub fn build(self) -> Plane {
        Plane {
            position: self.position,
            size: self.size,
            color: self.color,
            border_thickness: self.border_thickness,
            border_color: self.border_color,
            border_radius: self.border_radius
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