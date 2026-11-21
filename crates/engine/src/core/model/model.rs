use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3};
use russimp_ng::{
    material::{DataContent, PropertyTypeInfo, TextureType},
    scene::{PostProcess, Scene},
};

use crate::core::{
    gfx,
    material::{wind, Material, Pipeline},
    view_frustum::{Culled, ViewFrustum},
    assets::MaterialMap,
    renderer::{
        depth_pass,
        hdr,
        pbr::Lighting,
        shader::DynamicVertexArray,
        ssao,
        texture::Texture,
    },
};

use super::{
    model_materials::{
        canonical_slot, files_by_name, forget_predecoded, map_texture_type, maps_by_name, predecode, read_image,
        texture_candidates, upload_map,
    },
    Model, ModelBuilder, ModelMesh, ModelMeshVertex,
};

/// A pose revision no model had yet (see `Model::pose_revision`).
fn next_pose_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Length of `boneTransforms` in `vertex.glsl` (its `MAX_BONES`): bones
/// with a higher index are not uploaded.
const MAX_BONES: usize = 100;

/// The indices of `faces` that are triangles. Lines and points (an OBJ's
/// `l`, an FBX's curves and helpers) survive `Triangulate` and are not drawn.
fn triangle_indices(faces: &[russimp_ng::face::Face]) -> Vec<u32> {
    faces
        .iter()
        .filter(|f| f.0.len() == 3)
        .flat_map(|f| [f.0[0], f.0[1], f.0[2]])
        .collect()
}

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
        let model_dir = std::path::Path::new(full_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("assets/models")
            .to_string();
        let name = super::outline::default_variant_name(std::path::Path::new(full_path));
        Ok(Self::from_scene(scene, name, model_dir, position))
    }

    /// A model of an assimp scene made some other way (see `generated`):
    /// `name` as a file's stem, maps looked for next to it in `model_dir`.
    pub(super) fn from_scene<P: Into<Point3<f32>>>(scene: Scene, name: String, model_dir: String, position: P) -> Model {
        Model {
            model: scene,
            meshes: Vec::new(),
            materials: Vec::new(),
            file_materials: Vec::new(),
            shared_textures: Vec::new(),
            map_textures: Vec::new(),
            name,
            model_dir,
            position: position.into(),
            scale: 0.01,
            visible: None,
            hidden_bones: Vec::new(),
            tint: None,
            bone_cache: Default::default(),
            pose_revision: next_pose_revision(),
            cull_bounds: None,
        }
    }

    pub fn init(&mut self) {
        self.pose_changed();
        let materials = &self.model.materials;
        let mut all_maps: Vec<HashMap<TextureType, Texture>> = Vec::with_capacity(materials.len());
        self.shared_textures.clear();
        self.map_textures.clear();
        // Texture files by path and colour space, as uploaded: materials
        // often share one (an atlas), which is then read and uploaded once.
        let mut loaded: HashMap<(String, bool), usize> = HashMap::new();
        // Every image the materials below will read, decoded on all cores
        // first: one after the other on this thread, a scene's maps held
        // the editor for seconds.
        let mut embedded_images = self.predecode_maps();
        for (material_index, material) in materials.iter().enumerate() {
            let mut per_material: HashMap<TextureType, Texture> = HashMap::new();

            // 1) Embedded textures — russimp populates `material.textures` only
            //    for these. Each entry's `data` holds the encoded image bytes.
            for (tex_type, texture) in &material.textures {
                let tex = texture.borrow();
                let slot = canonical_slot(tex_type);
                if per_material.contains_key(&slot) {
                    continue;
                }
                if let DataContent::Bytes(_) = &tex.data {
                    match embedded_images.remove(&(material_index, slot.clone())) {
                        Some(Ok(image)) => {
                            per_material.insert(slot.clone(), upload_map(image, &slot));
                        }
                        Some(Err(e)) => log::warn!("model: embedded texture unreadable: {e}"),
                        None => {}
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
                if let Some(texture) = Self::load_external_texture(
                    raw_path,
                    &self.model_dir,
                    &slot,
                    &mut loaded,
                    &mut self.shared_textures,
                ) {
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

            // The model keeps the textures; materials hold handles on them,
            // so replacing a material (with a material asset's, say) never
            // frees a texture another material still draws with.
            let per_material = per_material
                .into_iter()
                .map(|(slot, texture)| {
                    let handle = texture.share();
                    self.map_textures.push(texture);
                    (slot, handle)
                })
                .collect();
            all_maps.push(per_material);
        }
        forget_predecoded();
        self.materials = self.materials_from_maps(all_maps);
        self.file_materials = self.materials.clone();
        for mesh in &self.model.meshes {
            let indices = triangle_indices(&mesh.faces);
            if indices.is_empty() {
                // Lines or points only: nothing to draw.
                continue;
            }
            if mesh.bones.len() > MAX_BONES {
                log::warn!(
                    "model {}: mesh \"{}\" has {} bones, only the first {MAX_BONES} are animated",
                    self.name,
                    mesh.name,
                    mesh.bones.len()
                );
            }
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
            let root_bone = self.find_root_bone(&mesh.bones);
            let mut model_mesh = ModelMesh::new(
                mesh.vertices
                    .iter()
                    .flat_map(|v| vec![v.x, v.y, v.z])
                    .collect(),
                indices,
                mesh.normals
                    .iter()
                    .flat_map(|v| vec![v.x, v.y, v.z])
                    .collect(),
                texture_coords,
                super::model_geometry::vertex_colors(mesh),
                root_bone,
                mesh.material_index as usize,
            );
            model_mesh.buffer_data();
            self.meshes.push((mesh.name.clone(), model_mesh));
        }
        self.cull_bounds = self.model_space_bounds();
    }

    /// See `cull_bounds`.
    fn model_space_bounds(&self) -> Option<(Point3<f32>, Point3<f32>)> {
        if self.meshes.iter().any(|(_, mesh)| mesh.root_bone.is_some()) {
            return None;
        }
        let mut vertices = self.model.meshes.iter().flat_map(|mesh| mesh.vertices.iter());
        let first = vertices.next()?;
        let mut min = Point3::new(first.x, first.y, first.z);
        let mut max = min;
        for v in vertices {
            min = Point3::new(min.x.min(v.x), min.y.min(v.y), min.z.min(v.z));
            max = Point3::new(max.x.max(v.x), max.y.max(v.y), max.z.max(v.z));
        }
        Some((min, max))
    }

    /// Decode on all cores what `init` will read: the external maps (the
    /// first candidate on disk), those found by name for the slots still
    /// empty (kept for `read_image`), and the embedded ones, returned by
    /// (material, slot).
    fn predecode_maps(
        &self,
    ) -> HashMap<(usize, TextureType), crate::core::renderer::texture::decode::Decoded> {
        use crate::core::renderer::texture::decode;
        let mut files = Vec::new();
        let mut embedded: Vec<((usize, TextureType), Vec<u8>)> = Vec::new();
        for (index, material) in self.model.materials.iter().enumerate() {
            let mut slots: Vec<TextureType> = Vec::new();
            for (tex_type, texture) in &material.textures {
                let slot = canonical_slot(tex_type);
                if slots.contains(&slot) {
                    continue;
                }
                if let DataContent::Bytes(bytes) = &texture.borrow().data {
                    embedded.push(((index, slot.clone()), bytes.clone()));
                    slots.push(slot);
                }
            }
            for property in &material.properties {
                if property.key != "$tex.file" {
                    continue;
                }
                let slot = canonical_slot(&property.semantic);
                if slots.contains(&slot) {
                    continue;
                }
                let PropertyTypeInfo::String(raw_path) = &property.data else {
                    continue;
                };
                let found = texture_candidates(raw_path, &self.model_dir)
                    .into_iter()
                    .find(|candidate| std::path::Path::new(candidate).is_file());
                if let Some(path) = found {
                    files.push(std::path::PathBuf::from(path));
                    slots.push(slot);
                }
            }
            let missing: Vec<MaterialMap> =
                MaterialMap::ALL.into_iter().filter(|map| !slots.contains(&map_texture_type(*map))).collect();
            if !missing.is_empty() {
                let name = Self::material_name(material);
                files.extend(files_by_name(name.as_deref(), &self.name, &self.model_dir, &missing));
            }
        }
        predecode(&files);
        let decoded = decode::decode_all(&embedded, |(_, bytes)| decode::decode_bytes(bytes));
        embedded.into_iter().map(|(key, _)| key).zip(decoded).collect()
    }

    pub(super) fn material_name(material: &russimp_ng::material::Material) -> Option<String> {
        super::outline::material_name(material)
    }

    /// Resolve a texture path referenced by a material and upload it.
    ///
    /// Exporters sometimes write absolute paths from the artist's machine
    /// (`C:\Users\...\bark.png`) or paths relative to the original asset
    /// folder. We only have the file name to work with at runtime, so try a
    /// few reasonable locations next to where models live in this project.
    /// A file uploaded before for the same colour space (in `loaded`, an
    /// index into `owners`) is shared instead of read again.
    fn load_external_texture(
        raw_path: &str,
        model_dir: &str,
        slot: &TextureType,
        loaded: &mut HashMap<(String, bool), usize>,
        owners: &mut Vec<Texture>,
    ) -> Option<Texture> {
        let srgb = *slot == TextureType::Diffuse;
        texture_candidates(raw_path, model_dir).into_iter().find_map(|candidate| {
            let key = (candidate, srgb);
            if let Some(&index) = loaded.get(&key) {
                return Some(owners[index].share());
            }
            let image = read_image(std::path::Path::new(&key.0))?;
            let texture = upload_map(image, slot);
            let shared = texture.share();
            loaded.insert(key, owners.len());
            owners.push(texture);
            Some(shared)
        })
    }

    pub fn render(
        &self,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        self.render_filtered(None, self.tint, lighting, parent_transform, camera_projection);
    }

    /// Draw the model as it is but see-through, `opacity` from 0 to 1 (1
    /// is `render`). With `see_inside` every surface shows through the
    /// ones in front of it (a vehicle's seats through its doors and
    /// windows); without, only its front-most surface shows (it is drawn
    /// into the depth first), so a faded character does not show what is
    /// inside it. For editor previews: it belongs to the transparent pass.
    pub fn render_faded(
        &self,
        opacity: f32,
        see_inside: bool,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        self.render_faded_meshes(None, opacity, see_inside, lighting, parent_transform, camera_projection);
    }

    /// `render_faded` of the `meshes` named only (`None`: what `render`
    /// draws).
    pub fn render_faded_meshes(
        &self,
        meshes: Option<&std::collections::HashSet<String>>,
        opacity: f32,
        see_inside: bool,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        if opacity >= 0.999 {
            self.render_filtered(meshes, None, lighting, parent_transform, camera_projection);
            return;
        }
        if opacity <= 0.001 || depth_pass::is_active() {
            return;
        }
        if !see_inside {
            self.draw_meshes_as(Look::DepthOnly, meshes, lighting, parent_transform, camera_projection);
        }
        self.draw_meshes_as(Look::Faded(opacity), meshes, lighting, parent_transform, camera_projection);
    }

    /// Draw only the `meshes` named (see `variant_meshes`), tinted as
    /// `set_tint` would (`None`: as they are). For a model whose parts are
    /// drawn each with its own transform, e.g. a vehicle's wheels, from
    /// one loaded model.
    pub fn render_meshes(
        &self,
        meshes: &std::collections::HashSet<String>,
        tint: Option<[f32; 4]>,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        self.render_filtered(Some(meshes), tint, lighting, parent_transform, camera_projection);
    }

    fn render_filtered(
        &self,
        only: Option<&std::collections::HashSet<String>>,
        tint: Option<[f32; 4]>,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        let model_transform = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        if let Some((min, max)) = self.cull_bounds {
            let visible = ViewFrustum::is_model_box_visible(camera_projection, &model_transform, min, max);
            ViewFrustum::count(Culled::Model, visible);
            if !visible {
                return;
            }
        }
        // A tinted model writes no depth: nothing to draw in a depth pass.
        let depth_only = depth_pass::is_active();
        if depth_only && tint.is_some() {
            return;
        }
        self.draw_meshes(depth_only, only, tint, lighting, parent_transform, camera_projection);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_meshes(
        &self,
        depth_only: bool,
        only: Option<&std::collections::HashSet<String>>,
        tint: Option<[f32; 4]>,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        let look = match (depth_only, tint) {
            (true, _) => Look::DepthPass,
            (false, Some(tint)) => Look::Tint(tint),
            (false, None) => Look::Normal,
        };
        self.draw_meshes_as(look, only, lighting, parent_transform, camera_projection);
    }

    fn draw_meshes_as(
        &self,
        look: Look,
        only: Option<&std::collections::HashSet<String>>,
        lighting: &Lighting,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        let depth_only = matches!(look, Look::DepthPass);
        let pipeline = if depth_only { Pipeline::MeshDepth } else { Pipeline::Mesh };
        // Shadow maps and the editor's selection mask draw into their own
        // framebuffers and keep every bone. The ambient occlusion's depth
        // pass has its own too, but sees what the camera sees.
        let draws_to_screen = hdr::draws_to_view() || ssao::in_depth_pass();
        let (mode, [r, g, b, a]) = match look {
            Look::Tint(tint) => (1, tint),
            Look::Faded(opacity) => (2, [1.0, 1.0, 1.0, opacity]),
            _ => (0, [1.0; 4]),
        };
        // The program and culling set up last: meshes of materials of the
        // same graph (most of them) share them.
        let mut current: Option<(u32, bool)> = None;
        for (index, (name, mesh)) in self.meshes.iter().enumerate() {
            let shown = match only {
                Some(only) => only.contains(name),
                None => self.visible.as_ref().is_none_or(|visible| visible.contains(name)),
            };
            if !shown {
                continue;
            }
            if !mesh.is_buffered() {
                panic!("Mesh is not buffered");
            }
            let fallback;
            let material = match self.materials.get(mesh.material_index) {
                Some(material) => material,
                None => {
                    fallback = Material::standard();
                    &fallback
                }
            };
            if depth_only && !material.shadows && !draws_to_screen {
                continue;
            }
            let shader = material.program(pipeline);
            let shader = &*shader;
            if current != Some((shader.id(), material.two_sided)) {
                current = Some((shader.id(), material.two_sided));
                let mut desc = gfx::PipelineDesc {
                    vertex_layouts: crate::core::gfx::Layouts::of(&[DynamicVertexArray::<ModelMeshVertex>::layout()]),
                    depth: gfx::DepthState::LESS,
                    cull: if material.two_sided { gfx::Cull::None } else { gfx::Cull::Back },
                    ..Default::default()
                };
                match look {
                    // A tinted model is see-through and writes no depth.
                    Look::Tint(_) => {
                        desc.blend = Some(gfx::BlendState::ALPHA);
                        desc.depth.write = false;
                    }
                    Look::DepthOnly => desc.color_write = false,
                    // Over its own depth (if drawn first: only the front-most
                    // surface) and what is opaque.
                    Look::Faded(_) => {
                        desc.blend = Some(gfx::BlendState::ALPHA);
                        desc.depth = gfx::DepthState { write: false, ..gfx::DepthState::LEQUAL };
                    }
                    Look::Normal | Look::DepthPass => {}
                }
                gfx::set_pipeline(shader.pipeline(&desc));
                if !depth_only {
                    lighting.apply(shader);
                    shader.set_uniform_1i("useTint", mode);
                    shader.set_uniform_4f("tint", r, g, b, a);
                } else {
                    let eye = lighting.camera_position;
                    shader.set_uniform_3f("cameraPosition", eye.x, eye.y, eye.z);
                }
                shader.set_uniform_mat4("viewProjection", camera_projection);
                wind::apply(shader, 1.0);
            }
            if let Some(root_bone) = &mesh.root_bone {
                let collapse = draws_to_screen && !self.hidden_bones.is_empty();
                let mut cache = self.bone_cache[collapse as usize].borrow_mut();
                if !cache.contains_key(&index) {
                    let hidden: &[String] = if collapse { &self.hidden_bones } else { &[] };
                    let bone_transforms = Model::get_bone_transformations(
                        root_bone,
                        Matrix4::identity(),
                        hidden,
                        None,
                    );
                    let max_id = bone_transforms.iter().map(|(id, _)| *id).max().unwrap_or(0);
                    let array_size = std::cmp::min(max_id + 1, MAX_BONES);
                    let mut sorted = vec![Matrix4::identity(); array_size];
                    for (id, transform) in bone_transforms {
                        if id < sorted.len() {
                            sorted[id] = transform;
                        }
                    }
                    cache.insert(index, sorted);
                }
                shader
                    .set_uniform_mat4_array("boneTransforms", &cache[&index]);
            } else {
                // Unskinned mesh: vertices were authored with bone slot 0 + full
                // weight (see ModelMesh::new), so feed the shader an identity at
                // that slot.
                shader
                    .set_uniform_mat4_array("boneTransforms", &vec![Matrix4::identity()]);
            }
            material.bind(shader);
            mesh.render(
                shader,
                parent_transform * Matrix4::from_translation(self.position.to_vec().into()),
                Some(self.scale),
            );
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

    pub fn reset_position(&mut self) -> Vector3<f32> {
        let position = self.position;
        self.position = Point3::new(0.0, 0.0, 0.0);
        position.to_vec()
    }

    /// Scale from the file's units to the scene's.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Lowest point of the meshes in the file's pose, in the model's units
    /// times its scale (as `get_vertices_scaled`).
    pub fn lowest_vertex(&self) -> Option<f32> {
        self.model
            .meshes
            .iter()
            .flat_map(|mesh| mesh.vertices.iter().map(|v| v.y * self.scale))
            .min_by(f32::total_cmp)
    }

    /// Model space of the bones: `parent_transform` with the model's own
    /// position and scale; see `ik::Skeleton`.
    pub fn root_transform(&self, parent_transform: &Matrix4<f32>) -> Matrix4<f32> {
        parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale)
    }

    /// Bones to draw collapsed when drawing to the screen; see the field.
    pub fn set_hidden_bones(&mut self, names: Vec<String>) {
        self.hidden_bones = names;
        self.pose_changed();
    }

    /// Drop the cached skinning matrices (see `bone_cache`).
    pub(super) fn pose_changed(&mut self) {
        for cache in &mut self.bone_cache {
            cache.get_mut().clear();
        }
        self.pose_revision = next_pose_revision();
    }

    /// A number that changes whenever the bones are posed (or the meshes
    /// loaded, or the hidden bones set), and that no other model or pose
    /// has: for what is worked out from the pose, to be kept until it
    /// changes. The model's position and scale are not part of it.
    pub fn pose_revision(&self) -> u64 {
        self.pose_revision
    }

    /// Draw the model as a see-through, lit silhouette of `rgba` (alpha
    /// is how opaque), e.g. as the preview of something to be placed;
    /// `None` draws it as it is. A tinted model writes no depth and
    /// belongs to the transparent pass.
    pub fn set_tint(&mut self, tint: Option<[f32; 4]>) {
        self.tint = tint;
    }
}

/// How `draw_meshes_as` draws.
#[derive(Clone, Copy)]
enum Look {
    Normal,
    /// Into a shadow or depth pass (its own shader).
    DepthPass,
    /// A see-through silhouette (`set_tint`).
    Tint([f32; 4]),
    /// Depth only, no colour: before `Faded`.
    DepthOnly,
    /// As it is, this opaque.
    Faded(f32),
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
    fn a_texture_file_several_materials_use_is_loaded_once() {
        crate::core::headless::enable();
        let temp = crate::core::assets::database::tests::TempDir::new("shared-texture");
        crate::core::assets::test_models::write_png(&temp.0.join("atlas.png"));
        crate::core::assets::test_models::write_png(&temp.0.join("bumps.png"));
        let mut mtl = String::new();
        let mut obj = String::from("mtllib props.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nvt 0 0\n");
        for (i, name) in ["bark", "leaves", "stone"].iter().enumerate() {
            mtl += &format!("newmtl {name}\nmap_Kd atlas.png\n");
            if i == 2 {
                // The same file as data (linear): a texture of its own.
                mtl += "map_Bump atlas.png\nmap_Ks bumps.png\n";
            }
            obj += &format!("o part{i}\nusemtl {name}\nf 1/1 2/1 3/1\nf 2/1 4/1 3/1\n");
        }
        std::fs::write(temp.0.join("props.mtl"), mtl).unwrap();
        std::fs::write(temp.0.join("props.obj"), obj).unwrap();

        let mut model = Model::from_file(temp.0.join("props.obj").to_str().unwrap(), (0.0, 0.0, 0.0)).unwrap();
        model.init();
        // atlas.png as colour, atlas.png as normals, bumps.png.
        assert_eq!(model.shared_textures.len(), 3);
        for name in ["bark", "leaves", "stone"] {
            let index = (0..model.material_count())
                .find(|&i| model.material_name_at(i).as_deref() == Some(name))
                .expect(name);
            let material = model.material(index).unwrap();
            assert!(material.texture("base_color_map").is_some(), "{name}");
        }
    }

    #[test]
    fn textures_are_looked_for_relative_to_the_model_first() {
        let c = texture_candidates("../building/textures/wood_BaseColor.jpg", "assets/models/props");
        assert_eq!(c[0], "assets/models/props/../building/textures/wood_BaseColor.jpg");
        assert!(c.contains(&"assets/models/props/wood_BaseColor.jpg".to_string()));
        assert!(c.contains(&"assets/models/props/wood BaseColor.jpg".to_string()));
        // An artist's absolute path only by its file name.
        let c = texture_candidates("C:\\Users\\me\\bark.png", "m");
        assert_eq!(c[0], "m/bark.png");
    }

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
