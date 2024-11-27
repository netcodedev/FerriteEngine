use cgmath::Matrix4;

use crate::core::renderer::shader::{DynamicVertexArray, Shader, VertexAttributes};

use super::{Bone, ModelMesh, ModelMeshVertex};

impl ModelMesh {
    pub fn new(
        vertices: Vec<f32>,
        indices: Vec<u32>,
        normals: Vec<f32>,
        texture_coords: Vec<f32>,
        root_bone: Option<Bone>,
    ) -> ModelMesh {
        let mut mesh_vertices = Vec::<ModelMeshVertex>::new();
        if let Some(root_bone) = &root_bone {
            let bone_weights = ModelMesh::get_bone_weights(root_bone.clone());
            for i in 0..vertices.len() / 3 {
                let weights = &bone_weights[i];
                mesh_vertices.push(ModelMeshVertex {
                    position: (vertices[i * 3], vertices[i * 3 + 1], vertices[i * 3 + 2]),
                    normal: (normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]),
                    texture_coords: (texture_coords[i * 2], texture_coords[i * 2 + 1]),
                    bone_ids: (
                        if weights.len() >= 1 {
                            weights[0].0 as u32
                        } else {
                            0
                        },
                        if weights.len() >= 2 {
                            weights[1].0 as u32
                        } else {
                            0
                        },
                        if weights.len() >= 3 {
                            weights[2].0 as u32
                        } else {
                            0
                        },
                        if weights.len() >= 4 {
                            weights[3].0 as u32
                        } else {
                            0
                        },
                    ),
                    bone_weights: (
                        if weights.len() >= 1 {
                            weights[0].1
                        } else {
                            0.0
                        },
                        if weights.len() >= 2 {
                            weights[1].1
                        } else {
                            0.0
                        },
                        if weights.len() >= 3 {
                            weights[2].1
                        } else {
                            0.0
                        },
                        if weights.len() >= 4 {
                            weights[3].1
                        } else {
                            0.0
                        },
                    ),
                });
            }
        }
        ModelMesh {
            root_bone,
            indices,
            vertices: mesh_vertices,
            vertex_array: None,
        }
    }

    pub fn render(&self, shader: &Shader, position: Matrix4<f32>, scale: Option<f32>) {
        if let Some(vertex_array) = &self.vertex_array {
            unsafe {
                gl::Enable(gl::DEPTH_TEST);
                gl::Enable(gl::CULL_FACE);
            }
            vertex_array.bind();
            let mut model = position;
            if let Some(scale) = scale {
                model = model * cgmath::Matrix4::from_scale(scale);
            }
            shader.set_uniform_mat4("model", &model);
            unsafe {
                gl::DrawElements(
                    gl::TRIANGLES,
                    self.indices.len() as i32,
                    gl::UNSIGNED_INT,
                    std::ptr::null(),
                );
                DynamicVertexArray::<ModelMeshVertex>::unbind();
                gl::Disable(gl::DEPTH_TEST);
                gl::Disable(gl::CULL_FACE);
            }
        }
    }

    pub fn is_buffered(&self) -> bool {
        self.vertex_array.is_some()
    }

    pub fn buffer_data(&mut self) {
        let mut vertex_array = DynamicVertexArray::<ModelMeshVertex>::new();
        vertex_array.buffer_data(&self.vertices, &Some(self.indices.clone()));
        self.vertex_array = Some(vertex_array);
    }

    fn get_bone_weights(root_bone: Bone) -> Vec<Vec<(usize, f32)>> {
        let bones = root_bone.get_as_vec();
        let mut bone_weights: Vec<Vec<(usize, f32)>> = Vec::new();
        for bone in bones {
            for weight in &bone.weights {
                if weight.1 == 0.0 {
                    continue;
                }
                if bone_weights.len() <= weight.0 as usize {
                    bone_weights.resize(weight.0 as usize + 1, Vec::new());
                }
                bone_weights[weight.0 as usize].push((bone.id, weight.1));
            }
        }
        bone_weights
    }
}

impl VertexAttributes for ModelMeshVertex {
    fn get_vertex_attributes() -> Vec<(usize, gl::types::GLuint)> {
        vec![
            (3, gl::FLOAT),
            (3, gl::FLOAT),
            (2, gl::FLOAT),
            (4, gl::UNSIGNED_INT),
            (4, gl::FLOAT),
        ]
    }
}
