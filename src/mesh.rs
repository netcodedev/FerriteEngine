use crate::shader::{Shader, VertexArray, VertexBufferData};

pub struct Mesh {
    vertex_array: Option<VertexArray>,
    vertices: Vec<f32>,
    indices: Vec<u32>,
    normals: Vec<f32>,
    texture_coords: Vec<f32>,
    block_type: Vec<u32>,
}

impl Mesh {
    pub fn new(vertices: Vec<f32>, indices: Vec<u32>, normals: Vec<f32>, texture_coords: Vec<f32>, block_type: Vec<u32>) -> Self {
        Mesh {
            vertex_array: None,
            vertices,
            indices,
            normals,
            texture_coords,
            block_type,
        }
    }

    pub fn buffer_data(&mut self) {
        let mut vertex_array = VertexArray::new();
        vertex_array.buffer_data(VertexBufferData {
            vertices: self.vertices.clone(),
            indices: Some(self.indices.clone()),
            normals: Some(self.normals.clone()),
            texture_coords: Some(self.texture_coords.clone()),
            block_type: Some(self.block_type.clone()),
        });
        self.vertex_array = Some(vertex_array);
    }

    pub fn render(&self, shader: &Shader, position: (f32, f32, f32)) {
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::CULL_FACE);
            shader.bind();
            let model = cgmath::Matrix4::from_translation(cgmath::Vector3::new(position.0, position.1, position.2));
            shader.set_uniform_mat4("model", &model);
            shader.set_uniform_1i("texture0", 0);
            shader.set_uniform_1i("texture1", 1);

            if let Some(vertex_array) = &self.vertex_array {
                vertex_array.bind();
                gl::DrawElements(gl::TRIANGLES, vertex_array.get_element_count() as i32, gl::UNSIGNED_INT, std::ptr::null());
                VertexArray::unbind();
            }
            gl::Disable(gl::CULL_FACE);
            gl::Disable(gl::DEPTH_TEST);
        }
    }

    pub fn is_buffered(&self) -> bool {
        if let Some(_) = &self.vertex_array {
            true
        } else {
            false
        }
    }
}
