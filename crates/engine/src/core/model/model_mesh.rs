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
        material_index: usize,
    ) -> ModelMesh {
        let mut mesh_vertices = Vec::<ModelMeshVertex>::new();
        let vertex_count = vertices.len() / 3;
        if root_bone.is_none() {
            // Unskinned mesh (e.g. a static prop FBX with no skeleton). The vertex
            // shader still multiplies by `boneTransforms[boneIDs[0]] * weights[0]`,
            // so emit a single full-weight bind to bone slot 0 and rely on the
            // renderer to upload an identity matrix there. Without this, `vertices`
            // would stay empty while `indices` still references vertex IDs, and
            // `glDrawElements` would read past the end of the VBO and crash.
            for i in 0..vertex_count {
                mesh_vertices.push(ModelMeshVertex {
                    position: (vertices[i * 3], vertices[i * 3 + 1], vertices[i * 3 + 2]),
                    normal: (normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]),
                    texture_coords: if texture_coords.len() >= (i + 1) * 2 {
                        (texture_coords[i * 2], texture_coords[i * 2 + 1])
                    } else {
                        (0.0, 0.0)
                    },
                    bone_ids: (0, 0, 0, 0),
                    bone_weights: (1.0, 0.0, 0.0, 0.0),
                });
            }
        }
        if let Some(root_bone) = &root_bone {
            let bone_weights = ModelMesh::get_bone_weights(root_bone.clone());
            for i in 0..vertex_count {
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
            material_index,
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
        // A vertex holds 4 influences: keep the strongest, summing to 1.
        for weights in &mut bone_weights {
            if weights.len() > 4 {
                weights.sort_by(|a, b| b.1.total_cmp(&a.1));
                weights.truncate(4);
                let sum: f32 = weights.iter().map(|w| w.1).sum();
                if sum > 0.0 {
                    weights.iter_mut().for_each(|w| w.1 /= sum);
                }
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

#[cfg(test)]
mod tests {
    use cgmath::{Matrix4, SquareMatrix, Vector3, Zero};

    use super::*;

    fn bone(id: usize, weight: f32, children: Option<Vec<Bone>>) -> Bone {
        Bone {
            id,
            name: format!("b{id}"),
            offset_matrix: Matrix4::identity(),
            weights: vec![(0, weight)],
            children,
            current_transform: Matrix4::identity(),
            last_translation: Vector3::zero(),
        }
    }

    #[test]
    fn a_vertex_keeps_its_four_strongest_influences_normalized() {
        // Six influences on vertex 0, the weakest first in tree order.
        let leaves = vec![
            bone(1, 0.05, None),
            bone(2, 0.3, None),
            bone(3, 0.2, None),
            bone(4, 0.1, None),
            bone(5, 0.25, None),
        ];
        let root = bone(0, 0.1, Some(leaves));
        let weights = &ModelMesh::get_bone_weights(root)[0];
        let ids: Vec<usize> = weights.iter().map(|w| w.0).collect();
        assert_eq!(ids, vec![2, 5, 3, 0]);
        let sum: f32 = weights.iter().map(|w| w.1).sum();
        assert!((sum - 1.0).abs() < 1e-5, "{sum}");
        assert!((weights[0].1 - 0.3 / 0.85).abs() < 1e-5);
    }

    #[test]
    fn up_to_four_influences_are_kept_as_they_are() {
        let root = bone(0, 0.5, Some(vec![bone(1, 0.25, None)]));
        let weights = &ModelMesh::get_bone_weights(root)[0];
        assert_eq!(weights, &vec![(0, 0.5), (1, 0.25)]);
    }
}
