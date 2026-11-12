use std::{collections::HashMap, rc::Rc};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4, Zero};
use russimp_ng::{
    material::{DataContent, PropertyTypeInfo, TextureType},
    node::Node,
    scene::{PostProcess, Scene},
};

use crate::core::renderer::{
    line::{Line, LineRenderer},
    shader::Shader,
    texture::Texture,
};

use super::{Bone, Model, ModelBuilder, ModelMesh, Pose};
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
        let shader: Shader =
            Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
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
                if let DataContent::Bytes(texture_data) = &tex.data {
                    let data = image::load_from_memory(texture_data.as_slice()).unwrap();
                    let texture = Texture::new();
                    texture.load_from_data(data.width(), data.height(), data.to_rgba8().into_raw());
                    per_material.insert(tex_type.clone(), texture);
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
                if per_material.contains_key(&property.semantic) {
                    continue; // already supplied as embedded data
                }
                let PropertyTypeInfo::String(raw_path) = &property.data else {
                    continue;
                };
                if let Some(texture) = Self::load_external_texture(raw_path, &self.model_dir) {
                    per_material.insert(property.semantic, texture);
                }
            }

            // 3) Heuristic fallback by material name — some FBX exports drop
            //    the texture connections entirely (or reference filenames that
            //    aren't in the on-disk asset). Match the material's name (e.g.
            //    `Bark_Mat`) against files in the model's directory so we can
            //    still recover the diffuse/normal pair.
            let needs_diffuse = !per_material.contains_key(&TextureType::Diffuse)
                && !per_material.contains_key(&TextureType::BaseColor);
            let needs_normal = !per_material.contains_key(&TextureType::Normals);
            if needs_diffuse || needs_normal {
                if let Some(mat_name) = Self::material_name(material) {
                    let (diffuse, normal) =
                        Self::heuristic_textures_by_name(&mat_name, &self.model_dir);
                    if needs_diffuse {
                        if let Some(t) = diffuse {
                            per_material.insert(TextureType::Diffuse, t);
                        }
                    }
                    if needs_normal {
                        if let Some(t) = normal {
                            per_material.insert(TextureType::Normals, t);
                        }
                    }
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

    /// Best-effort recovery for materials whose texture connections were
    /// lost during export. Strips a trailing `_Mat`/`_Material` suffix from
    /// the material name to get a prefix, then looks in `model_dir` for
    /// files starting with that prefix and a keyword in the filename:
    /// * `color`, `albedo`, `basecolor`, `diffuse` → diffuse
    /// * `normal` → normal map
    /// Skips obvious non-color maps (roughness, metalness, AO).
    fn heuristic_textures_by_name(
        material_name: &str,
        model_dir: &str,
    ) -> (Option<Texture>, Option<Texture>) {
        let lc = material_name.to_lowercase();
        let prefix = lc
            .trim_end_matches("_material")
            .trim_end_matches("_mat")
            .trim_end_matches('_')
            .trim();
        if prefix.is_empty() {
            return (None, None);
        }

        let Ok(read) = std::fs::read_dir(model_dir) else {
            return (None, None);
        };
        let mut entries: Vec<(String, std::path::PathBuf)> = read
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .map(|s| (s.to_lowercase(), e.path()))
            })
            .collect();
        // Sort for stable, deterministic selection — `read_dir` order is OS-
        // dependent and HashMap-style randomness here would mean different
        // textures on different runs.
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        // If the asset ships a separate opacity texture for this prefix
        // (e.g. `Cluster_Opacity.png` next to `Cluster_albedo.png`), we'll
        // bake it into the diffuse texture's alpha channel so the existing
        // alpha-cutout shader code does the right thing without needing a
        // second sampler.
        let opacity_path = entries
            .iter()
            .find(|(lc, _)| lc.starts_with(prefix) && lc.contains("opacity"))
            .map(|(_, p)| p.clone());

        let mut diffuse: Option<Texture> = None;
        let mut normal: Option<Texture> = None;
        for (lc_name, path) in &entries {
            if !lc_name.starts_with(prefix) {
                continue;
            }
            let is_normal = lc_name.contains("normal");
            let is_aux = lc_name.contains("roughness")
                || lc_name.contains("metallic")
                || lc_name.contains("metalness")
                || lc_name.contains("ambientocclusion")
                || lc_name.contains("_ao")
                || lc_name.contains("opacity");

            if is_normal {
                if normal.is_none() {
                    normal = Self::load_texture_from_disk(path);
                }
            } else if !is_aux {
                let looks_like_diffuse = lc_name.contains("color")
                    || lc_name.contains("albedo")
                    || lc_name.contains("basecolor")
                    || lc_name.contains("diffuse");
                if diffuse.is_none() && looks_like_diffuse {
                    diffuse = Self::load_diffuse_with_opacity(path, opacity_path.as_deref());
                }
            }
        }
        (diffuse, normal)
    }

    /// Load a diffuse texture, optionally replacing its alpha channel with
    /// the luminance of an external opacity map. Falls back to the plain
    /// diffuse load if the opacity image can't be read or has mismatched
    /// dimensions.
    fn load_diffuse_with_opacity(
        diffuse_path: &std::path::Path,
        opacity_path: Option<&std::path::Path>,
    ) -> Option<Texture> {
        let bytes = std::fs::read(diffuse_path).ok()?;
        let mut diffuse_rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
        if let Some(op_path) = opacity_path {
            if let Ok(op_bytes) = std::fs::read(op_path) {
                if let Ok(op_img) = image::load_from_memory(&op_bytes) {
                    let op_gray = op_img.to_luma8();
                    if op_gray.dimensions() == diffuse_rgba.dimensions() {
                        for (px_d, px_o) in
                            diffuse_rgba.pixels_mut().zip(op_gray.pixels())
                        {
                            px_d.0[3] = px_o.0[0];
                        }
                    } else {
                        eprintln!(
                            "Model::init: opacity map '{}' size {:?} != diffuse '{}' size {:?} \
                             — skipping merge",
                            op_path.display(),
                            op_gray.dimensions(),
                            diffuse_path.display(),
                            diffuse_rgba.dimensions(),
                        );
                    }
                }
            }
        }
        let texture = Texture::new();
        texture.load_from_data(
            diffuse_rgba.width(),
            diffuse_rgba.height(),
            diffuse_rgba.into_raw(),
        );
        Some(texture)
    }

    fn load_texture_from_disk(path: &std::path::Path) -> Option<Texture> {
        let bytes = std::fs::read(path).ok()?;
        let image = image::load_from_memory(&bytes).ok()?;
        let texture = Texture::new();
        texture.load_from_data(image.width(), image.height(), image.to_rgba8().into_raw());
        Some(texture)
    }

    /// Resolve a texture path referenced by a material and upload it.
    ///
    /// Exporters sometimes write absolute paths from the artist's machine
    /// (`C:\Users\...\bark.png`) or paths relative to the original asset
    /// folder. We only have the file name to work with at runtime, so try a
    /// few reasonable locations next to where models live in this project.
    fn load_external_texture(raw_path: &str, model_dir: &str) -> Option<Texture> {
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

        for candidate in &candidates {
            let Ok(bytes) = std::fs::read(candidate) else {
                continue;
            };
            let Ok(image) = image::load_from_memory(&bytes) else {
                continue;
            };
            let texture = Texture::new();
            texture.load_from_data(image.width(), image.height(), image.to_rgba8().into_raw());
            return Some(texture);
        }
        None
    }

    pub fn render(
        &self,
        light_position: &Point3<f32>,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        for mesh in self.meshes.values() {
            if !mesh.is_buffered() {
                panic!("Mesh is not buffered");
            }
            self.shader.bind();
            self.shader.set_uniform_3f(
                "lightPosition",
                light_position.x,
                light_position.y,
                light_position.z,
            );
            self.shader
                .set_uniform_mat4("viewProjection", &camera_projection);
            if let Some(root_bone) = &mesh.root_bone {
                let bone_transforms =
                    Model::get_bone_transformations(root_bone, Matrix4::identity());
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
            // Bind this mesh's material textures. Models that contain multiple
            // materials (e.g. trunk + foliage on a tree) used to all sample the
            // last-loaded texture because every material's maps got merged
            // into a single shared HashMap; now each mesh picks the entry
            // matching its `material_index`.
            if let Some(material_textures) = self.textures.get(mesh.material_index) {
                for (i, (texture_type, texture)) in material_textures.iter().enumerate() {
                    unsafe { gl::ActiveTexture(gl::TEXTURE0 + i as u32) };
                    texture.bind();
                    match texture_type {
                        TextureType::Diffuse => {
                            self.shader.set_uniform_1i("texture_diffuse", i as i32)
                        }
                        TextureType::Shininess => {
                            self.shader.set_uniform_1i("texture_shininess", i as i32)
                        }
                        TextureType::Normals => {
                            self.shader.set_uniform_1i("texture_normal", i as i32)
                        }
                        TextureType::Specular => {
                            self.shader.set_uniform_1i("texture_specular", i as i32)
                        }
                        _ => {}
                    }
                }
            }
            unsafe { gl::Disable(gl::CULL_FACE) };
            mesh.render(
                &self.shader,
                parent_transform * Matrix4::from_translation(self.position.to_vec().into()),
                Some(self.scale),
            );
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
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

    /// Upload an image file as a texture.
    pub fn load_texture_file(path: &std::path::Path) -> Option<Texture> {
        Self::load_texture_from_disk(path)
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

    fn get_bone_transformations(
        bone: &Bone,
        parent_transform: Matrix4<f32>,
    ) -> Vec<(usize, Matrix4<f32>)> {
        let mut transformations = Vec::<(usize, Matrix4<f32>)>::new();
        let global_transformation = parent_transform * bone.current_transform;

        if bone.id != usize::MAX {
            transformations.push((bone.id, global_transformation * bone.offset_matrix));
        }

        if let Some(children) = &bone.children {
            for child in children {
                transformations
                    .extend(Self::get_bone_transformations(child, global_transformation));
            }
        }
        transformations
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
