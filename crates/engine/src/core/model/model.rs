use std::collections::HashMap;

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Zero};
use russimp_ng::{
    material::{DataContent, PropertyTypeInfo, TextureType},
    scene::{PostProcess, Scene},
};

use crate::core::{
    view_frustum::ViewFrustum,
    assets::MaterialMap,
    renderer::{
        hdr,
        pbr::{self, Lighting},
        shader::Shader,
        ssao,
        texture::Texture,
    },
};

use super::{
    model_materials::{
        canonical_slot, map_texture_type, maps_by_name, read_image, texture_candidates, upload_map,
    },
    Bone, Model, ModelBuilder, ModelMesh,
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
            tint: None,
            bone_cache: Default::default(),
            cull_bounds: None,
        })
    }

    pub fn init(&mut self) {
        self.pose_changed();
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
        self.cull_bounds = self.model_space_bounds();
    }

    /// See `cull_bounds`.
    fn model_space_bounds(&self) -> Option<(Point3<f32>, Point3<f32>)> {
        if self.meshes.values().any(|mesh| mesh.root_bone.is_some()) {
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

    pub(super) fn material_name(material: &russimp_ng::material::Material) -> Option<String> {
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
        texture_candidates(raw_path, model_dir)
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
        let model_transform = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        if let Some((min, max)) = self.cull_bounds {
            if !ViewFrustum::is_box_visible(&(camera_projection * model_transform), min, max) {
                return;
            }
        }
        self.shader.bind();
        lighting.apply(&self.shader);
        // Shadow maps and the editor's selection mask draw into their own
        // framebuffers and keep every bone. The ambient occlusion's depth
        // pass has its own too, but sees what the camera sees.
        let draws_to_screen = hdr::draws_to_view() || ssao::in_depth_pass();
        self.shader
            .set_uniform_mat4("viewProjection", &camera_projection);
        let [r, g, b, a] = self.tint.unwrap_or([1.0; 4]);
        self.shader.set_uniform_1i("useTint", self.tint.is_some() as i32);
        self.shader.set_uniform_4f("tint", r, g, b, a);
        if self.tint.is_some() {
            unsafe {
                gl::Enable(gl::BLEND);
                gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
                gl::DepthMask(gl::FALSE);
            }
        }
        for (name, mesh) in self.meshes.iter() {
            if self.visible.as_ref().is_some_and(|visible| !visible.contains(name)) {
                continue;
            }
            if !mesh.is_buffered() {
                panic!("Mesh is not buffered");
            }
            if let Some(root_bone) = &mesh.root_bone {
                let collapse = draws_to_screen && !self.hidden_bones.is_empty();
                let mut cache = self.bone_cache[collapse as usize].borrow_mut();
                if !cache.contains_key(name) {
                    let hidden: &[String] = if collapse { &self.hidden_bones } else { &[] };
                    let bone_transforms = Model::get_bone_transformations(
                        root_bone,
                        Matrix4::identity(),
                        hidden,
                        None,
                    );
                    let max_id = bone_transforms.iter().map(|(id, _)| *id).max().unwrap_or(0);
                    let array_size = std::cmp::min(max_id + 1, 100);
                    let mut sorted = vec![Matrix4::identity(); array_size];
                    for (id, transform) in bone_transforms {
                        if id < sorted.len() {
                            sorted[id] = transform;
                        }
                    }
                    cache.insert(name.clone(), sorted);
                }
                self.shader
                    .set_uniform_mat4_array("boneTransforms", &cache[name]);
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
        if self.tint.is_some() {
            unsafe {
                gl::DepthMask(gl::TRUE);
                gl::Disable(gl::BLEND);
            }
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
    }

    /// Draw the model as a see-through, lit silhouette of `rgba` (alpha
    /// is how opaque), e.g. as the preview of something to be placed;
    /// `None` draws it as it is. A tinted model writes no depth and
    /// belongs to the transparent pass.
    pub fn set_tint(&mut self, tint: Option<[f32; 4]>) {
        self.tint = tint;
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
