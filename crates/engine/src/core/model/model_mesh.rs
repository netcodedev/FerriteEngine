use cgmath::Matrix4;

use crate::core::renderer::shader::{DynamicVertexArray, Shader, VertexAttributes};

use super::{Bone, ModelMesh, ModelMeshVertex};

impl ModelMesh {
    pub fn new(
        vertices: Vec<f32>,
        indices: Vec<u32>,
        normals: Vec<f32>,
        texture_coords: Vec<f32>,
        colors: Vec<u32>,
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
        let color = |i: usize| colors.get(i).copied().unwrap_or(super::model_geometry::WHITE);
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
                    color: color(i),
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
                    color: color(i),
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
            let mut model = position;
            if let Some(scale) = scale {
                model = model * cgmath::Matrix4::from_scale(scale);
            }
            shader.set_uniform_mat4("model", &model);
            vertex_array.draw();
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
    fn vertex_formats() -> Vec<crate::core::gfx::VertexFormat> {
        use crate::core::gfx::VertexFormat::*;
        vec![F32x3, F32x3, F32x2, U32x4, F32x4, U32]
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
        let mesh = ModelMesh::new(vertices.clone(), vec![0, 1, 2], Vec::new(), Vec::new(), Vec::new(), Some(root), 0);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.vertices[2].bone_weights, (1.0, 0.0, 0.0, 0.0));
        assert_eq!(mesh.vertices[2].normal, (0.0, 1.0, 0.0));
        let mesh = ModelMesh::new(vertices, vec![0, 1, 2], Vec::new(), Vec::new(), Vec::new(), None, 0);
        assert_eq!(mesh.vertices[1].texture_coords, (0.0, 0.0));
    }
}
