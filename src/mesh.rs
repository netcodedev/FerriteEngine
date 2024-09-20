use crate::shader::{Shader, VertexArray, VertexBufferData};

pub struct Mesh {
    vertex_array: Option<VertexArray>,
    vertices: Vec<f32>,
    indices: Option<Vec<u32>>,
    normals: Option<Vec<f32>>,
    texture_coords: Option<Vec<f32>>,
    block_type: Option<Vec<u32>>,
}

impl Mesh {
    pub fn new(vertices: Vec<f32>, indices: Option<Vec<u32>>, normals: Option<Vec<f32>>, texture_coords: Option<Vec<f32>>, block_type: Option<Vec<u32>>) -> Self {
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
            indices: if let Some(indices) = &self.indices { Some(indices.clone()) } else { None },
            normals: if let Some(normals) = &self.normals { Some(normals.clone()) } else { None },
            texture_coords: if let Some(texture_coords) = &self.texture_coords { Some(texture_coords.clone()) } else { None },
            block_type: if let Some(block_type) = &self.block_type { Some(block_type.clone()) } else { None },
        });
        self.vertex_array = Some(vertex_array);
    }

    pub fn render(&self, shader: &Shader, position: (f32, f32, f32), scale: Option<f32>) {
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::CULL_FACE);
            shader.bind();
            let mut model = cgmath::Matrix4::from_translation(cgmath::Vector3::new(position.0, position.1, position.2));
            if let Some(scale) = scale {
                model = model * cgmath::Matrix4::from_scale(scale);
            }
            shader.set_uniform_mat4("model", &model);
            shader.set_uniform_1i("texture0", 0);
            shader.set_uniform_1i("texture1", 1);

            if let Some(vertex_array) = &self.vertex_array {
                vertex_array.bind();
                if let Some(_) = &self.indices {
                    gl::DrawElements(gl::TRIANGLES, vertex_array.get_element_count() as i32, gl::UNSIGNED_INT, std::ptr::null());
                } else {
                    gl::DrawArrays(gl::TRIANGLES, 0, vertex_array.get_element_count() as i32);
                }
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
