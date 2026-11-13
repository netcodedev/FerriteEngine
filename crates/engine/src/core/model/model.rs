use std::{collections::HashMap, rc::Rc};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4, Zero};
use russimp_ng::{
    material::{DataContent, PropertyTypeInfo, TextureType},
    node::Node,
    scene::{PostProcess, Scene},
};

use crate::core::{
    assets::MaterialMap,
    renderer::{
        line::{Line, LineRenderer},
        pbr::{self, Lighting},
        shader::Shader,
        ssao,
        texture::Texture,
    },
};

use super::{
    material_maps::{self, MapKind},
    Bone, Model, ModelBuilder, ModelMesh, Pose,
};
use crate::core::utils::ToMatrix4;

impl Model {
    /// Load a model by its path below `assets/models/`.
    pub fn new<P: Into<Point3<f32>>>(
        path: &str,
        position: P,
    ) -> Result<Model, Box<dyn std::error::Error>> {
        Self::from_file(&format!("assets/models/{path}"), position)
    }

    /// Load a model from a path relative to the working directory.
    pub fn from_file<P: Into<Point3<f32>>>(
        full_path: &str,
        position: P,
    ) -> Result<Model, Box<dyn std::error::Error>> {
        let scene = Scene::from_file(
            full_path,
            vec![
                PostProcess::Triangulate,
                // PostProcess::JoinIdenticalVertices,
                PostProcess::GenerateSmoothNormals,
                PostProcess::FlipUVs,
            ],
        )?;
        let shader = Shader::new(
            include_str!("vertex.glsl"),
            &pbr::fragment_source(include_str!("fragment.glsl")),
        );
        let model_dir = std::path::Path::new(full_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("assets/models")
            .to_string();
        Ok(Model {
            model: scene,
            meshes: HashMap::<String, ModelMesh>::new(),
            shader,
            textures: Vec::new(),
            name: super::outline::default_variant_name(std::path::Path::new(full_path)),
            model_dir,
            position: position.into(),
            scale: 0.01,
            visible: None,
            hidden_bones: Vec::new(),
        })
    }

    pub fn init(&mut self) {
        let materials = &self.model.materials;
        self.textures = Vec::with_capacity(materials.len());
        for material in materials {
            let mut per_material: HashMap<TextureType, Texture> = HashMap::new();

            // 1) Embedded textures — russimp populates `material.textures` only
            //    for these. Each entry's `data` holds the encoded image bytes.
            for (tex_type, texture) in &material.textures {
                let tex = texture.borrow();
                let slot = canonical_slot(tex_type);
                if per_material.contains_key(&slot) {
                    continue;
                }
                if let DataContent::Bytes(texture_data) = &tex.data {
                    match image::load_from_memory(texture_data.as_slice()) {
                        Ok(image) => {
                            per_material.insert(slot.clone(), upload_map(image.to_rgba8(), &slot));
                        }
                        Err(e) => log::warn!("model: embedded texture unreadable: {e}"),
                    }
                }
            }

            // 2) External textures — FBX files commonly reference textures by
            //    file path rather than embedding the pixels. Those references
            //    live in the material properties under the key `$tex.file`, and
            //    `MaterialProperty::semantic` says which TextureType slot it
            //    belongs to. We try the assets/models and assets/textures
            //    directories so artists can drop the maps next to the .fbx.
            for property in &material.properties {
                if property.key != "$tex.file" {
                    continue;
                }
                let slot = canonical_slot(&property.semantic);
                if per_material.contains_key(&slot) {
                    continue; // already supplied as embedded data
                }
                let PropertyTypeInfo::String(raw_path) = &property.data else {
                    continue;
                };
                if let Some(texture) = Self::load_external_texture(raw_path, &self.model_dir, &slot)
                {
                    per_material.insert(slot, texture);
                }
            }

            // 3) By file name — many exports (and scanned assets) do not
            //    connect their maps at all. Maps next to the model named
            //    after the material or the file fill the slots still empty.
            let missing: Vec<MaterialMap> = MaterialMap::ALL
                .into_iter()
                .filter(|map| !per_material.contains_key(&map_texture_type(*map)))
                .collect();
            if !missing.is_empty() {
                let material_name = Self::material_name(material);
                let maps =
                    maps_by_name(material_name.as_deref(), &self.name, &self.model_dir, &missing);
                for (map, texture) in maps {
                    per_material.insert(map_texture_type(map), texture);
                }
            }

            self.textures.push(per_material);
        }
        for mesh in &self.model.meshes {
            // UVs must come from THIS mesh's first channel — using a single
            // shared `texture_coords` derived from `meshes[0]` (as the code
            // used to) silently fed the wrong UVs to every additional mesh in
            // a multi-mesh model, producing visibly stretched / tiled
            // textures. Concatenating all UV channels was wrong for the same
            // reason: only channel 0 corresponds to `layout(location=2)` in
            // the model shader.
            let texture_coords: Vec<f32> = mesh
                .texture_coords
                .first()
                .and_then(|c| c.as_ref())
                .map(|c| c.iter().flat_map(|v| [v.x, v.y]).collect())
                .unwrap_or_default();
            let mut root_bone = None;
            if let Some(root_node) = &self.model.root {
                for node in root_node.children.borrow().iter() {
                    for (id, bone) in mesh.bones.iter().enumerate() {
                        if bone.name != node.name {
                            continue;
                        }
                        root_bone = Some(Bone {
                            id,
                            name: bone.name.clone(),
                            current_transform: node.transformation.to_matrix_4(),
                            offset_matrix: bone.offset_matrix.to_matrix_4(),
                            weights: bone
                                .weights
                                .iter()
                                .map(|w| (w.vertex_id, w.weight))
                                .collect(),
                            children: self.get_child_bones(node, &mesh.bones),
                            last_translation: Vector3::zero(),
                        });
                    }
                }
            }
            let mut model_mesh = ModelMesh::new(
                mesh.vertices
                    .iter()
                    .flat_map(|v| vec![v.x, v.y, v.z])
                    .collect(),
                mesh.faces
                    .iter()
                    .flat_map(|f| vec![f.0[0], f.0[1], f.0[2]])
                    .collect::<Vec<u32>>(),
                mesh.normals
                    .iter()
                    .flat_map(|v| vec![v.x, v.y, v.z])
                    .collect(),
                texture_coords,
                root_bone,
                mesh.material_index as usize,
            );
            model_mesh.buffer_data();
            self.meshes.insert(mesh.name.clone(), model_mesh);
        }
    }

    fn material_name(material: &russimp_ng::material::Material) -> Option<String> {
        super::outline::material_name(material)
    }

    /// Resolve a texture path referenced by a material and upload it.
    ///
    /// Exporters sometimes write absolute paths from the artist's machine
    /// (`C:\Users\...\bark.png`) or paths relative to the original asset
    /// folder. We only have the file name to work with at runtime, so try a
    /// few reasonable locations next to where models live in this project.
    fn load_external_texture(
        raw_path: &str,
        model_dir: &str,
        slot: &TextureType,
    ) -> Option<Texture> {
        let trimmed = raw_path.trim_end_matches('\0').trim();
        let file_name = super::outline::texture_file_name(raw_path)?;
        // Some FBX exporters write spaces in texture names while the on-disk
        // file uses underscores (or vice versa), so try both spellings.
        let space_underscore = file_name.replace(' ', "_");
        let underscore_space = file_name.replace('_', " ");

        let mut name_variants = vec![file_name.to_string()];
        if space_underscore != file_name {
            name_variants.push(space_underscore);
        }
        if underscore_space != file_name {
            name_variants.push(underscore_space);
        }

        // Search the model's own directory first (most assets ship their
        // textures alongside the model file), then fall back to the project's
        // standard texture/model directories.
        let model_dir_string = model_dir.to_string();
        let model_dir_textures = format!("{model_dir}/textures");
        let dirs: [&str; 5] = [
            model_dir_string.as_str(),
            model_dir_textures.as_str(),
            "assets/models",
            "assets/models/textures",
            "assets/textures",
        ];

        let mut candidates: Vec<String> = Vec::new();
        for dir in &dirs {
            for name in &name_variants {
                candidates.push(format!("{dir}/{name}"));
            }
        }
        candidates.push(trimmed.to_string());

        candidates
            .iter()
            .find_map(|candidate| read_image(std::path::Path::new(candidate)))
            .map(|image| upload_map(image, slot))
    }

    pub fn render(
        &self,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        self.shader.bind();
        lighting.apply(&self.shader);
        // Shadow maps and the editor's selection mask draw into their own
        // framebuffers and keep every bone. The ambient occlusion's depth
        // pass has its own too, but sees what the camera sees.
        let mut framebuffer = 0;
        unsafe { gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut framebuffer) };
        let draws_to_screen = framebuffer == 0 || ssao::in_depth_pass();
        self.shader
            .set_uniform_mat4("viewProjection", &camera_projection);
        for (name, mesh) in self.meshes.iter() {
            if self.visible.as_ref().is_some_and(|visible| !visible.contains(name)) {
                continue;
            }
            if !mesh.is_buffered() {
                panic!("Mesh is not buffered");
            }
            if let Some(root_bone) = &mesh.root_bone {
                let hidden: &[String] = if draws_to_screen { &self.hidden_bones } else { &[] };
                let bone_transforms =
                    Model::get_bone_transformations(root_bone, Matrix4::identity(), hidden, None);
                let max_id = bone_transforms.iter().map(|(id, _)| *id).max().unwrap_or(0);
                let array_size = std::cmp::min(max_id + 1, 100);
                let mut sorted = vec![Matrix4::identity(); array_size];
                for (id, transform) in bone_transforms {
                    if id < sorted.len() {
                        sorted[id] = transform;
                    }
                }
                self.shader
                    .set_uniform_mat4_array("boneTransforms", &sorted);
            } else {
                // Unskinned mesh: vertices were authored with bone slot 0 + full
                // weight (see ModelMesh::new), so feed the shader an identity at
                // that slot.
                self.shader
                    .set_uniform_mat4_array("boneTransforms", &vec![Matrix4::identity()]);
            }
            pbr::bind_material(&self.shader, self.textures.get(mesh.material_index));
            unsafe { gl::Disable(gl::CULL_FACE) };
            mesh.render(
                &self.shader,
                parent_transform * Matrix4::from_translation(self.position.to_vec().into()),
                Some(self.scale),
            );
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
    }

    /// Distance along the ray (`dir` normalized) to the nearest triangle of
    /// the meshes that are drawn, with the model drawn at `parent_transform`.
    /// Bones are not applied: a skinned mesh is tested in its rest pose.
    pub fn raycast(
        &self,
        parent_transform: &Matrix4<f32>,
        origin: Point3<f32>,
        dir: Vector3<f32>,
    ) -> Option<f32> {
        use cgmath::Transform;
        let model = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        let inverse = model.inverse_transform()?;
        // The transform is affine, so the ray parameter in model space is
        // the distance along the world ray.
        let local_origin = inverse.transform_point(origin);
        let local_dir = inverse.transform_vector(dir);
        let mut nearest: Option<f32> = None;
        for mesh in &self.model.meshes {
            if self.visible.as_ref().is_some_and(|visible| !visible.contains(&mesh.name)) {
                continue;
            }
            let vertex = |i: u32| {
                mesh.vertices
                    .get(i as usize)
                    .map(|v| Point3::new(v.x, v.y, v.z))
            };
            for face in &mesh.faces {
                let [a, b, c] = face.0[..] else {
                    continue;
                };
                let (Some(a), Some(b), Some(c)) = (vertex(a), vertex(b), vertex(c)) else {
                    continue;
                };
                if let Some(t) = ray_triangle(local_origin, local_dir, a, b, c) {
                    if nearest.map_or(true, |n| t < n) {
                        nearest = Some(t);
                    }
                }
            }
        }
        nearest
    }

    pub fn render_bones(&self, view_projection: &Matrix4<f32>, parent_transform: &Matrix4<f32>) {
        let root = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        let mut lines: Vec<Line> = Vec::new();
        for mesh in self.meshes.values() {
            if let Some(root_bone) = &mesh.root_bone {
                lines.extend(self.render_child_bones(root_bone, root));
            }
        }
        LineRenderer::render_lines(
            view_projection,
            &lines,
            cgmath::Vector3::new(1.0, 0.0, 0.0),
            true,
        );
    }

    /// Returns (bone_name, parent_world_pos, bone_world_pos) for every bone, in world space.
    /// `entity_transform` should be the entity's translation * rotation matrix.
    pub fn get_bone_segments(
        &self,
        entity_transform: &Matrix4<f32>,
    ) -> Vec<(String, Point3<f32>, Point3<f32>)> {
        let root = entity_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        let mut out = Vec::new();
        for mesh in self.meshes.values() {
            if let Some(bone) = &mesh.root_bone {
                self.collect_bone_segments(bone, root, &mut out);
            }
        }
        out
    }

    fn collect_bone_segments(
        &self,
        bone: &Bone,
        parent_transform: Matrix4<f32>,
        out: &mut Vec<(String, Point3<f32>, Point3<f32>)>,
    ) {
        use cgmath::Transform;
        let bone_transform = parent_transform * bone.current_transform;
        let parent_pos = parent_transform.transform_point(Point3::new(0.0, 0.0, 0.0));
        let bone_pos = bone_transform.transform_point(Point3::new(0.0, 0.0, 0.0));
        out.push((bone.name.clone(), parent_pos, bone_pos));
        if let Some(children) = &bone.children {
            for child in children {
                self.collect_bone_segments(child, bone_transform, out);
            }
        }
    }

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

    /// Display name of the material at `idx`, if assimp reported one.
    pub fn material_name_at(&self, idx: usize) -> Option<String> {
        self.model
            .materials
            .get(idx)
            .and_then(|m| Self::material_name(m))
    }

    /// Texture set for a given material index, as collected by `init`.
    pub fn material_textures(
        &self,
        idx: usize,
    ) -> Option<&std::collections::HashMap<russimp_ng::material::TextureType, crate::core::renderer::texture::Texture>> {
        self.textures.get(idx)
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

    /// Replace the texture a material uses in one slot, e.g. with one the
    /// user assigned to the asset.
    pub fn set_material_texture(
        &mut self,
        material_index: usize,
        slot: TextureType,
        texture: Texture,
    ) {
        if let Some(textures) = self.textures.get_mut(material_index) {
            textures.insert(slot, texture);
        }
    }

    /// Use the image file at `path` as one map of a material, e.g. one the
    /// user assigned to the asset. Returns false if it cannot be read.
    pub fn set_material_map_file(
        &mut self,
        material_index: usize,
        map: MaterialMap,
        path: &std::path::Path,
    ) -> bool {
        let Some(image) = read_image(path) else {
            return false;
        };
        let slot = map_texture_type(map);
        let texture = upload_map(image, &slot);
        self.set_material_texture(material_index, slot, texture);
        true
    }

    /// Number of materials in the file.
    pub fn material_count(&self) -> usize {
        self.model.materials.len()
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

    pub fn reset_position(&mut self) -> Vector3<f32> {
        let position = self.position;
        self.position = Point3::new(0.0, 0.0, 0.0);
        position.to_vec()
    }

    pub fn apply_pose(&mut self, pose: &Pose) {
        let mut root_translation = Vector3::zero();
        for mesh in self.meshes.values_mut() {
            if let Some(root_bone) = &mut mesh.root_bone {
                root_translation += root_bone.apply_pose(pose, true);
            }
        }
        self.position += root_translation * self.scale;
    }

    fn render_child_bones(&self, bone: &Bone, root: cgmath::Matrix4<f32>) -> Vec<Line> {
        let position = root * bone.current_transform;
        let pos_vec = (position * Vector4::new(0.0, 0.0, 0.0, 1.0)).truncate();
        let root_vec = (root * Vector4::new(0.0, 0.0, 0.0, 1.0)).truncate();
        let direction = pos_vec - root_vec;
        let mut lines = vec![Line {
            position: Point3::new(root_vec.x, root_vec.y, root_vec.z),
            direction: direction.normalize(),
            length: direction.magnitude(),
        }];
        if let Some(children) = &bone.children {
            for child in children {
                lines.extend(self.render_child_bones(child, position));
            }
        }
        lines
    }

    fn get_child_bones(
        &self,
        node: &Rc<Node>,
        bones: &Vec<russimp_ng::bone::Bone>,
    ) -> Option<Vec<Bone>> {
        if node.children.borrow().len() == 0 {
            return None;
        }
        let mut children = Vec::<Bone>::new();
        for child in node.children.borrow().iter() {
            let mut bone_id = usize::MAX;
            let mut bone_offset_matrix = Matrix4::identity();
            let mut weights = Vec::new();

            for (id, bone) in bones.iter().enumerate() {
                if bone.name == child.name {
                    bone_id = id;
                    bone_offset_matrix = bone.offset_matrix.to_matrix_4();
                    weights = bone
                        .weights
                        .iter()
                        .map(|w| (w.vertex_id, w.weight))
                        .collect();
                    break;
                }
            }

            children.push(Bone {
                id: bone_id,
                name: child.name.clone(),
                current_transform: child.transformation.to_matrix_4(),
                offset_matrix: bone_offset_matrix,
                weights,
                children: self.get_child_bones(child, bones),
                last_translation: Vector3::zero(),
            });
        }
        Some(children)
    }

    /// Skinning matrices of `bone` and its descendants. A bone named in
    /// `hidden`, and everything below it, maps all its vertices onto the
    /// joint `collapse_to` (the hidden bone's own joint).
    fn get_bone_transformations(
        bone: &Bone,
        parent_transform: Matrix4<f32>,
        hidden: &[String],
        collapse_to: Option<Vector3<f32>>,
    ) -> Vec<(usize, Matrix4<f32>)> {
        let mut transformations = Vec::<(usize, Matrix4<f32>)>::new();
        let global_transformation = parent_transform * bone.current_transform;
        let collapse_to = collapse_to.or_else(|| {
            hidden
                .contains(&bone.name)
                .then(|| global_transformation.w.truncate())
        });

        if bone.id != usize::MAX {
            let transform = match collapse_to {
                Some(joint) => Matrix4::from_translation(joint) * Matrix4::from_scale(0.0),
                None => global_transformation * bone.offset_matrix,
            };
            transformations.push((bone.id, transform));
        }

        if let Some(children) = &bone.children {
            for child in children {
                transformations.extend(Self::get_bone_transformations(
                    child,
                    global_transformation,
                    hidden,
                    collapse_to,
                ));
            }
        }
        transformations
    }

    /// Bones to draw collapsed when drawing to the screen; see the field.
    pub fn set_hidden_bones(&mut self, names: Vec<String>) {
        self.hidden_bones = names;
    }
}

impl ModelBuilder {
    pub fn new(path: &str) -> Result<ModelBuilder, Box<dyn std::error::Error>> {
        Ok(ModelBuilder {
            model: Model::new(path, (0.0, 0.0, 0.0))?,
        })
    }

    /// Like `new`, for a path relative to the working directory.
    pub fn from_file(full_path: &str) -> Result<ModelBuilder, Box<dyn std::error::Error>> {
        Ok(ModelBuilder {
            model: Model::from_file(full_path, (0.0, 0.0, 0.0))?,
        })
    }

    pub fn with_position<P: Into<Point3<f32>>>(mut self, position: P) -> ModelBuilder {
        self.model.position = position.into();
        self
    }

    pub fn with_scale(mut self, scale: f32) -> ModelBuilder {
        self.model.scale = scale;
        self
    }

    pub fn build(self) -> Model {
        self.model
    }
}

/// The slot a texture of this assimp type is kept in. Spellings of the same
/// map share one slot.
fn canonical_slot(texture_type: &TextureType) -> TextureType {
    match texture_type {
        TextureType::BaseColor => TextureType::Diffuse,
        TextureType::NormalCamera => TextureType::Normals,
        other => other.clone(),
    }
}

/// The slot a material map is kept in.
fn map_texture_type(map: MaterialMap) -> TextureType {
    match map {
        MaterialMap::BaseColor => TextureType::Diffuse,
        MaterialMap::Normal => TextureType::Normals,
        MaterialMap::Roughness => TextureType::Roughness,
        MaterialMap::Metallic => TextureType::Metalness,
        MaterialMap::Occlusion => TextureType::AmbientOcclusion,
    }
}

fn read_image(path: &std::path::Path) -> Option<image::RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    match image::load_from_memory(&bytes) {
        Ok(image) => Some(image.to_rgba8()),
        Err(e) => {
            log::warn!("model: could not read texture {}: {e}", path.display());
            None
        }
    }
}

/// Upload an image as the map in `slot`; only the colour map is sRGB.
fn upload_map(image: image::RgbaImage, slot: &TextureType) -> Texture {
    let texture = Texture::new();
    let (width, height) = image.dimensions();
    texture.load_material_map(width, height, image.into_raw(), *slot == TextureType::Diffuse);
    texture
}

/// The maps in `wanted` found next to the model by file name (see
/// `material_maps`). A separate opacity map goes into the colour map's
/// alpha; a gloss map is inverted into roughness.
fn maps_by_name(
    material_name: Option<&str>,
    model_stem: &str,
    model_dir: &str,
    wanted: &[MaterialMap],
) -> Vec<(MaterialMap, Texture)> {
    let prefixes = material_maps::name_prefixes(material_name, model_stem);
    let files = material_maps::images_with_prefix(std::path::Path::new(model_dir), &prefixes);
    let found = material_maps::find_maps(&files, &prefixes);
    let opacity = found
        .iter()
        .find(|(kind, _)| *kind == MapKind::Opacity)
        .and_then(|(_, path)| read_image(path));

    let mut maps = Vec::new();
    for (kind, path) in &found {
        let Some(map) = kind.slot().filter(|map| wanted.contains(map)) else {
            continue;
        };
        let Some(mut image) = read_image(path) else {
            continue;
        };
        if *kind == MapKind::Gloss {
            for pixel in image.pixels_mut() {
                for channel in &mut pixel.0[..3] {
                    *channel = 255 - *channel;
                }
            }
        }
        if map == MaterialMap::BaseColor {
            if let Some(opacity) = &opacity {
                if opacity.dimensions() == image.dimensions() {
                    for (pixel, alpha) in image.pixels_mut().zip(opacity.pixels()) {
                        pixel.0[3] = alpha.0[0];
                    }
                } else {
                    log::warn!(
                        "model: opacity map size {:?} differs from {} {:?}, not used",
                        opacity.dimensions(),
                        path.display(),
                        image.dimensions()
                    );
                }
            }
        }
        maps.push((map, upload_map(image, &map_texture_type(map))));
    }
    maps
}

/// Möller–Trumbore, two-sided: the ray parameter of the hit, if the ray
/// meets the triangle in front of its origin.
fn ray_triangle(
    origin: Point3<f32>,
    dir: Vector3<f32>,
    a: Point3<f32>,
    b: Point3<f32>,
    c: Point3<f32>,
) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_hits_a_triangle_from_either_side() {
        let (a, b, c) = (
            Point3::new(-1.0, -1.0, 0.0),
            Point3::new(1.0, -1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        );
        let down = Vector3::new(0.0, 0.0, -1.0);
        assert_eq!(ray_triangle(Point3::new(0.0, 0.0, 5.0), down, a, b, c), Some(5.0));
        assert_eq!(ray_triangle(Point3::new(0.0, 0.0, -5.0), -down, a, b, c), Some(5.0));
    }

    #[test]
    fn a_ray_misses_beside_or_behind_a_triangle() {
        let (a, b, c) = (
            Point3::new(-1.0, -1.0, 0.0),
            Point3::new(1.0, -1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        );
        let down = Vector3::new(0.0, 0.0, -1.0);
        assert_eq!(ray_triangle(Point3::new(3.0, 0.0, 5.0), down, a, b, c), None);
        assert_eq!(ray_triangle(Point3::new(0.0, 0.0, -5.0), down, a, b, c), None);
    }
}
