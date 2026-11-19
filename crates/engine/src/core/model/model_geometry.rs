//! A model's geometry: submeshes, variants, vertices and indices.

use super::Model;

impl Model {
    /// Per-submesh geometry (positions, normals, UVs, indices) for callers
    /// that want to build their own GPU buffers, e.g. an instanced renderer.
    /// Positions are already pre-multiplied by `self.scale`.
    pub fn extract_submeshes(&self) -> Vec<crate::core::model::SubmeshData> {
        use crate::core::model::SubmeshData;
        self.model
            .meshes
            .iter()
            .map(|mesh| {
                let positions: Vec<[f32; 3]> = mesh
                    .vertices
                    .iter()
                    .map(|v| [v.x * self.scale, v.y * self.scale, v.z * self.scale])
                    .collect();
                let normals: Vec<[f32; 3]> = if mesh.normals.len() == mesh.vertices.len() {
                    mesh.normals.iter().map(|v| [v.x, v.y, v.z]).collect()
                } else {
                    vec![[0.0, 1.0, 0.0]; positions.len()]
                };
                let uvs: Vec<[f32; 2]> = mesh
                    .texture_coords
                    .first()
                    .and_then(|c| c.as_ref())
                    .map(|c| c.iter().map(|v| [v.x, v.y]).collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
                let indices: Vec<u32> = mesh
                    .faces
                    .iter()
                    .filter(|f| f.0.len() == 3)
                    .flat_map(|f| [f.0[0], f.0[1], f.0[2]])
                    .collect();
                SubmeshData {
                    positions,
                    normals,
                    uvs,
                    indices,
                    material_index: mesh.material_index as usize,
                }
            })
            .collect()
    }

    /// Walks the scene-graph root's direct children and returns each child
    /// node grouped with the submeshes referenced by its subtree, paired with
    /// the node's name. Useful for LOD-by-node setups where each top-level
    /// node represents one detail tier.
    pub fn extract_lod_submeshes(&self) -> Vec<(String, Vec<crate::core::model::SubmeshData>)> {
        let mut out = Vec::new();
        if let Some(root) = &self.model.root {
            for child in root.children.borrow().iter() {
                let mut group = Vec::new();
                self.collect_node_submeshes(child, &mut group);
                if !group.is_empty() {
                    out.push((child.name.clone(), group));
                }
            }
        }
        out
    }

    /// Every variant in the file with the geometry of its LODs. A file is
    /// split into variants by `outline::group_nodes`, the same way the asset
    /// importer describes it.
    pub fn extract_tree_groups(&self) -> Vec<crate::core::model::TreeGroup> {
        use crate::core::model::TreeGroup;
        let Some(root) = &self.model.root else {
            return Vec::new();
        };
        super::outline::group_nodes(root, &self.name)
            .into_iter()
            .map(|group| TreeGroup {
                name: group.name,
                lods: group
                    .lods
                    .into_iter()
                    .map(|(name, node)| {
                        let mut submeshes = Vec::new();
                        self.collect_node_submeshes(&node, &mut submeshes);
                        (name, submeshes)
                    })
                    .collect(),
            })
            .collect()
    }

    /// Draw only one variant of the file, at its most detailed LOD: the
    /// variant of this name, or the first one without a name. A file that
    /// is not split into variants or LODs is drawn whole either way.
    pub fn show_variant(&mut self, variant: Option<&str>) -> Result<(), String> {
        let Some(root) = &self.model.root else {
            return Ok(());
        };
        let groups = super::outline::group_nodes(root, &self.name);
        let group = match variant {
            Some(name) => groups
                .iter()
                .find(|group| group.name == name)
                .ok_or_else(|| format!("the model has no variant '{name}'"))?,
            None => match groups.first() {
                Some(group) => group,
                None => return Ok(()),
            },
        };
        let Some((_, node)) = group
            .lods
            .iter()
            .min_by_key(|(name, _)| super::outline::lod_priority(name))
        else {
            return Ok(());
        };
        let mut names = std::collections::HashSet::new();
        self.collect_mesh_names(node, &mut names);
        self.visible = Some(names);
        Ok(())
    }

    /// The meshes in the node of this name and below it, to draw with
    /// `render_meshes`, and where the node is: its translation from the
    /// file's root, at the model's scale. Meshes are drawn in their node's
    /// own space (node transforms are not applied), so a model made of
    /// parts, e.g. a vehicle with its wheels, draws each part's meshes
    /// moved by this (and turned about it as it likes). `None` if the file
    /// has no node of that name.
    pub fn node_part(
        &self,
        name: &str,
    ) -> Option<(std::collections::HashSet<String>, cgmath::Vector3<f32>)> {
        use crate::core::utils::ToMatrix4;
        fn find<'a>(
            node: &'a russimp_ng::node::Node,
            name: &str,
            parent: cgmath::Matrix4<f32>,
            out: &mut Option<(std::rc::Rc<russimp_ng::node::Node>, cgmath::Matrix4<f32>)>,
            this: Option<std::rc::Rc<russimp_ng::node::Node>>,
        ) {
            let transform = parent * node.transformation.to_matrix_4();
            if out.is_none() && node.name == name {
                *out = this.map(|n| (n, transform));
                return;
            }
            for child in node.children.borrow().iter() {
                find(child, name, transform, out, Some(child.clone()));
            }
        }
        let root = self.model.root.as_ref()?;
        let mut found = None;
        find(root, name, cgmath::SquareMatrix::identity(), &mut found, Some(root.clone()));
        let (node, transform) = found?;
        let mut names = std::collections::HashSet::new();
        self.collect_mesh_names(&node, &mut names);
        Some((names, transform.w.truncate() * self.scale))
    }

    fn collect_mesh_names(
        &self,
        node: &russimp_ng::node::Node,
        out: &mut std::collections::HashSet<String>,
    ) {
        for &index in &node.meshes {
            if let Some(mesh) = self.model.meshes.get(index as usize) {
                out.insert(mesh.name.clone());
            }
        }
        for child in node.children.borrow().iter() {
            self.collect_mesh_names(child, out);
        }
    }

    fn collect_node_submeshes(
        &self,
        node: &russimp_ng::node::Node,
        out: &mut Vec<crate::core::model::SubmeshData>,
    ) {
        use crate::core::model::SubmeshData;
        for &mesh_idx in &node.meshes {
            let mesh = &self.model.meshes[mesh_idx as usize];
            let positions: Vec<[f32; 3]> = mesh
                .vertices
                .iter()
                .map(|v| [v.x * self.scale, v.y * self.scale, v.z * self.scale])
                .collect();
            let normals: Vec<[f32; 3]> = if mesh.normals.len() == mesh.vertices.len() {
                mesh.normals.iter().map(|v| [v.x, v.y, v.z]).collect()
            } else {
                vec![[0.0, 1.0, 0.0]; positions.len()]
            };
            let uvs: Vec<[f32; 2]> = mesh
                .texture_coords
                .first()
                .and_then(|c| c.as_ref())
                .map(|c| c.iter().map(|v| [v.x, v.y]).collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
            let indices: Vec<u32> = mesh
                .faces
                .iter()
                .filter(|f| f.0.len() == 3)
                .flat_map(|f| [f.0[0], f.0[1], f.0[2]])
                .collect();
            out.push(SubmeshData {
                positions,
                normals,
                uvs,
                indices,
                material_index: mesh.material_index as usize,
            });
        }
        for child in node.children.borrow().iter() {
            self.collect_node_submeshes(child, out);
        }
    }

    pub fn get_vertices_scaled(&self) -> Vec<[f32; 3]> {
        self.model
            .meshes
            .iter()
            .flat_map(|mesh| {
                mesh.vertices
                    .iter()
                    .map(|v| [v.x * self.scale, v.y * self.scale, v.z * self.scale])
            })
            .collect()
    }

    /// Triangle index list across every mesh in the model, in the same vertex
    /// order as `get_vertices_scaled`. Indices from each successive mesh are
    /// offset by the running vertex count so that callers (e.g. trimesh
    /// collider builders) can use a single concatenated vertex buffer.
    pub fn get_indices(&self) -> Vec<[u32; 3]> {
        let mut indices = Vec::new();
        let mut vertex_offset: u32 = 0;
        for mesh in &self.model.meshes {
            for face in &mesh.faces {
                if face.0.len() == 3 {
                    indices.push([
                        face.0[0] + vertex_offset,
                        face.0[1] + vertex_offset,
                        face.0[2] + vertex_offset,
                    ]);
                }
            }
            vertex_offset += mesh.vertices.len() as u32;
        }
        indices
    }
}

#[cfg(test)]
mod tests {
    use crate::core::{
        assets::{database::tests::TempDir, test_models},
        model::Model,
    };

    #[test]
    fn a_part_is_found_by_its_node_with_its_meshes_and_place() {
        crate::core::headless::enable();
        let temp = TempDir::new("node-part");
        let file = temp.0.join("truck.gltf");
        test_models::write_gltf(&file, &[("Body", &[]), ("Wheel", &["Tyre"])], None);
        let text = std::fs::read_to_string(&file).unwrap().replace(
            r#"{"name": "Wheel", "children""#,
            r#"{"name": "Wheel", "translation": [1.0, 2.0, 3.0], "children""#,
        );
        std::fs::write(&file, text).unwrap();
        let model = Model::from_file(file.to_str().unwrap(), (0.0, 0.0, 0.0)).unwrap();
        let (meshes, at) = model.node_part("Wheel").expect("the wheel");
        assert_eq!(meshes.len(), 1);
        assert_eq!(at * (1.0 / model.scale()), cgmath::Vector3::new(1.0, 2.0, 3.0));
        let (body, at) = model.node_part("Body").expect("the body");
        assert_eq!(body.len(), 1);
        assert_eq!(at, cgmath::Vector3::new(0.0, 0.0, 0.0));
        assert!(model.node_part("Door").is_none());
    }
}
