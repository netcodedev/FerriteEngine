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
        // Vertices only lines or points use have no normal; files without
        // UVs have none either.
        let normal = |i: usize| match normals.get(i * 3..i * 3 + 3) {
            Some(&[x, y, z]) => (x, y, z),
            _ => (0.0, 1.0, 0.0),
        };
        let uv = |i: usize| match texture_coords.get(i * 2..i * 2 + 2) {
            Some(&[u, v]) => (u, v),
            _ => (0.0, 0.0),
        };
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
                    normal: normal(i),
                    texture_coords: uv(i),
                    bone_ids: (0, 0, 0, 0),
                    bone_weights: (1.0, 0.0, 0.0, 0.0),
                });
            }
        }
        if let Some(root_bone) = &root_bone {
            let bone_weights = ModelMesh::get_bone_weights(root_bone.clone());
            for i in 0..vertex_count {
                let (bone_ids, bone_weights) = vertex_weights(bone_weights.get(i));
                mesh_vertices.push(ModelMeshVertex {
                    position: (vertices[i * 3], vertices[i * 3 + 1], vertices[i * 3 + 2]),
                    normal: normal(i),
                    texture_coords: uv(i),
                    bone_ids,
                    bone_weights,
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

/// A skinned vertex's first four bones and weights, from its `weights`
/// (bone id, weight). A vertex no bone of the skeleton weighs (a partly
/// skinned mesh) follows bone 0 fully instead of collapsing to the origin.
fn vertex_weights(weights: Option<&Vec<(usize, f32)>>) -> ((u32, u32, u32, u32), (f32, f32, f32, f32)) {
    let weights = match weights {
        Some(weights) if !weights.is_empty() => weights.as_slice(),
        _ => return ((0, 0, 0, 0), (1.0, 0.0, 0.0, 0.0)),
    };
    let id = |n: usize| weights.get(n).map_or(0, |w| w.0 as u32);
    let weight = |n: usize| weights.get(n).map_or(0.0, |w| w.1);
    ((id(0), id(1), id(2), id(3)), (weight(0), weight(1), weight(2), weight(3)))
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
    use super::*;

    #[test]
    fn unweighted_vertices_follow_the_first_bone() {
        assert_eq!(vertex_weights(None), ((0, 0, 0, 0), (1.0, 0.0, 0.0, 0.0)));
        assert_eq!(vertex_weights(Some(&Vec::new())), ((0, 0, 0, 0), (1.0, 0.0, 0.0, 0.0)));
        let two = vec![(3, 0.75), (5, 0.25)];
        assert_eq!(vertex_weights(Some(&two)), ((3, 5, 0, 0), (0.75, 0.25, 0.0, 0.0)));
    }

    #[test]
    fn missing_normals_and_uvs_do_not_panic() {
        // A skinned triangle whose third vertex no bone weighs, without
        // normals or UVs.
        let root = Bone {
            id: 0,
            name: "root".into(),
            current_transform: Matrix4::from_scale(1.0),
            offset_matrix: Matrix4::from_scale(1.0),
            weights: vec![(0, 1.0), (1, 1.0)],
            children: None,
            last_translation: cgmath::Vector3::new(0.0, 0.0, 0.0),
        };
        let vertices = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mesh = ModelMesh::new(vertices.clone(), vec![0, 1, 2], Vec::new(), Vec::new(), Some(root), 0);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.vertices[2].bone_weights, (1.0, 0.0, 0.0, 0.0));
        assert_eq!(mesh.vertices[2].normal, (0.0, 1.0, 0.0));
        let mesh = ModelMesh::new(vertices, vec![0, 1, 2], Vec::new(), Vec::new(), None, 0);
        assert_eq!(mesh.vertices[1].texture_coords, (0.0, 0.0));
    }
}
