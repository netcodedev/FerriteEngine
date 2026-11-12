//! Models scattered over the terrain (trees, rocks, ...). The terrain owns a
//! list of scatter layers, each a model file plus a density; nothing here is
//! required for a terrain to exist — without layers, or while a model is
//! missing, the terrain just stays bare.
//!
//! A model file can hold several variants (e.g. a tree pack), each with
//! discrete LODs. Loaded models are kept in a thread-local cache, so
//! rebuilding the scene (e.g. entering and leaving play mode) does not load
//! them again. Each chunk gets one `ScatterInstances` per layer, which picks
//! a variant per placement (deterministically hashed from its XZ position)
//! and drives one `glDrawElementsInstanced` call per (variant, LOD-submesh)
//! bucket it actually contains.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cgmath::{EuclideanSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Vector3, Zero};
use gl::types::{GLsizei, GLsizeiptr, GLuint, GLvoid};
use glfw::WindowEvent;
use rapier3d::prelude::{
    ColliderBuilder, ColliderHandle, Pose, Rotation, SharedShape, TriMeshFlags, Vector,
};
use russimp_ng::material::TextureType;
use serde::{Deserialize, Serialize};

use crate::core::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    model::{Model, ModelBuilder, TreeGroup},
    physics::physics_engine::PhysicsEngine,
    renderer::{light::skylight::SkyLight, shader::Shader, texture::Texture},
    scene::Scene,
};

use super::{dual_contouring::WATER_LEVEL, CHUNK_SIZE_FLOAT};

/// Height band scattered models may stand in: clear of the water and below
/// the bare peaks.
pub const SCATTER_MIN_Y: f32 = WATER_LEVEL + 4.0;
pub const SCATTER_MAX_Y: f32 = 78.0;

const MODEL_SCALE: f32 = 1.0;

/// Distance bands for LOD selection. With 4 LODs available (Acer pack), each
/// instance picks tier 0/1/2/3 by ascending distance; models with fewer LODs
/// clamp to their last tier.
const LOD_DIST_NEAR: f32 = 60.0;
const LOD_DIST_MID: f32 = 130.0;
const LOD_DIST_FAR: f32 = 250.0;

/// Layers of the default scene: `(model, density)`. Models are paths
/// relative to `assets/models/`.
pub const DEFAULT_SCATTER: &[(&str, f32)] = &[
    ("Trees/Trees.gltf", 21.0),
    ("Trees/Acer tree pack.fbx", 21.0),
];

/// Density a layer starts with when only its model is given.
pub const DEFAULT_DENSITY: f32 = 20.0;
/// Upper end of the density slider.
pub const MAX_DENSITY: f32 = 200.0;
/// Area the density refers to: placements tried per 100 m x 100 m.
const DENSITY_AREA: f32 = 100.0 * 100.0;

const POS_OFFSET: usize = 0;
const NORMAL_OFFSET: usize = 12;
const UV_OFFSET: usize = 24;
const VERTEX_STRIDE: i32 = 32;

const INSTANCE_STRIDE: i32 = 64;

thread_local! {
    static MODEL_CACHE: RefCell<ModelCache> = RefCell::new(ModelCache::default());
}

/// Everything loaded so far, by model path. Failed loads are not cached, so
/// a model that was missing is tried again the next time it is asked for.
#[derive(Default)]
struct ModelCache {
    shader: Option<Rc<Shader>>,
    models: HashMap<String, Rc<ScatterModel>>,
}

/// One loaded model file: its variants, ready to be instanced. Shared by the
/// layer that asked for it and every chunk's instances of it.
pub struct ScatterModel {
    shader: Rc<Shader>,
    variants: Vec<ModelVariant>,
}

impl ScatterModel {
    /// Load a model (path relative to `assets/models/`), or hand out the
    /// cached one. Must run on the GL thread.
    pub fn load(path: &str) -> Result<Rc<ScatterModel>, String> {
        if let Some(cached) = MODEL_CACHE.with(|cache| cache.borrow().models.get(path).cloned()) {
            return Ok(cached);
        }
        let variants = load_variants(path).map_err(|e| e.to_string())?;
        let shader = MODEL_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .shader
                .get_or_insert_with(|| {
                    Rc::new(Shader::new(
                        include_str!("scatter_instanced_vertex.glsl"),
                        include_str!("scatter_instanced_fragment.glsl"),
                    ))
                })
                .clone()
        });
        let model = Rc::new(ScatterModel { shader, variants });
        MODEL_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .models
                .insert(path.to_string(), model.clone())
        });
        Ok(model)
    }

    /// Number of variants in the file (always at least one).
    pub fn variant_count(&self) -> usize {
        self.variants.len()
    }
}

/// What one layer wants on every chunk.
#[derive(Clone)]
pub struct ScatterSpec {
    pub model: Rc<ScatterModel>,
    /// Placements tried per chunk; some are rejected by height.
    pub attempts: u32,
    /// Makes each model scatter to its own spots.
    pub salt: u64,
}

impl ScatterSpec {
    fn same_as(&self, other: &ScatterSpec) -> bool {
        Rc::ptr_eq(&self.model, &other.model)
            && self.attempts == other.attempts
            && self.salt == other.salt
    }
}

struct ModelVariant {
    /// Shared with sibling variants loaded from the same multi-variant asset, so
    /// material textures live in exactly one place.
    model: Rc<Model>,
    /// Detail tiers sorted from highest quality (index 0) to lowest.
    lods: Vec<VariantLod>,
    collider_shape: SharedShape,
}

struct VariantLod {
    submeshes: Vec<SubmeshGpu>,
}

struct SubmeshGpu {
    vbo: GLuint,
    ibo: GLuint,
    index_count: GLsizei,
    material_index: usize,
}

impl Drop for SubmeshGpu {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteBuffers(1, &self.ibo);
        }
    }
}

/// Free the cached models and textures. Must run on the GL thread while the
/// context is still alive; otherwise they would be dropped with the
/// thread-local at process exit, after the context is gone.
pub fn release_gl_resources() {
    let _ = MODEL_CACHE.try_with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.models.clear();
        cache.shader = None;
    });
}

/// Sort key for an LOD's name. Lower numbers are higher detail.
fn lod_priority(name: &str) -> u32 {
    let lc = name.to_lowercase();
    if lc.contains("lod0") {
        0
    } else if lc.contains("lod1") {
        1
    } else if lc.contains("lod2") {
        2
    } else if lc.contains("lod3") || lc.contains("billboard") {
        3
    } else if lc.contains("high") {
        0
    } else if lc.contains("last") {
        2
    } else if lc.contains("low") {
        1
    } else {
        99
    }
}

fn load_variants(path: &str) -> Result<Vec<ModelVariant>, Box<dyn std::error::Error>> {
    let mut model = ModelBuilder::new(path)?.with_scale(MODEL_SCALE).build();
    model.init();
    let mut groups = model.extract_tree_groups();
    if groups.is_empty() {
        // No LOD-named nodes: a plain model. Use all of it as one variant
        // with a single detail tier.
        let submeshes = model.extract_submeshes();
        if submeshes.iter().all(|sd| sd.indices.is_empty()) {
            return Err(format!("no geometry found in '{path}'").into());
        }
        groups.push(TreeGroup {
            name: path.to_string(),
            lods: vec![("lod0".to_string(), submeshes)],
        });
    }

    let shared_model = Rc::new(model);

    let mut out = Vec::with_capacity(groups.len());
    for group in groups.iter_mut() {
        group.lods.sort_by_key(|(name, _)| lod_priority(name));

        // Build GPU LODs.
        let mut gpu_lods: Vec<VariantLod> = Vec::with_capacity(group.lods.len());
        for (_, submeshes) in &group.lods {
            let mut submesh_gpu = Vec::with_capacity(submeshes.len());
            for sd in submeshes {
                let mut vertex_data: Vec<f32> = Vec::with_capacity(sd.positions.len() * 8);
                for i in 0..sd.positions.len() {
                    let p = sd.positions[i];
                    let n = sd.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
                    let uv = sd.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
                    vertex_data.extend_from_slice(&[
                        p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1],
                    ]);
                }
                let (vbo, ibo) = unsafe {
                    let mut vbo = 0;
                    let mut ibo = 0;
                    gl::GenBuffers(1, &mut vbo);
                    gl::GenBuffers(1, &mut ibo);
                    gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
                    gl::BufferData(
                        gl::ARRAY_BUFFER,
                        (vertex_data.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                        vertex_data.as_ptr() as *const GLvoid,
                        gl::STATIC_DRAW,
                    );
                    gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ibo);
                    gl::BufferData(
                        gl::ELEMENT_ARRAY_BUFFER,
                        (sd.indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                        sd.indices.as_ptr() as *const GLvoid,
                        gl::STATIC_DRAW,
                    );
                    gl::BindBuffer(gl::ARRAY_BUFFER, 0);
                    gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);
                    (vbo, ibo)
                };
                submesh_gpu.push(SubmeshGpu {
                    vbo,
                    ibo,
                    index_count: sd.indices.len() as GLsizei,
                    material_index: sd.material_index,
                });
            }
            gpu_lods.push(VariantLod { submeshes: submesh_gpu });
        }

        // Pick an LOD to base the collider on: prefer "low" (still 3D, much
        // cheaper than the high-poly tier), fall back to whatever has 3D
        // geometry that isn't the billboard.
        let collider_idx = group
            .lods
            .iter()
            .position(|(name, _)| {
                let lc = name.to_lowercase();
                lc.contains("lod1") || lc.contains("low")
            })
            .or_else(|| {
                group.lods.iter().position(|(name, _)| {
                    let lc = name.to_lowercase();
                    !lc.contains("lod3") && !lc.contains("billboard") && !lc.contains("last")
                })
            })
            .unwrap_or(0);
        let collider_shape = build_collider(&shared_model, &group.lods[collider_idx].1)
            .ok_or_else(|| format!("'{}' failed to build collider trimesh", group.name))?;

        out.push(ModelVariant {
            model: shared_model.clone(),
            lods: gpu_lods,
            collider_shape,
        });
    }
    Ok(out)
}

/// Build a shared trimesh collider from a single LOD's submeshes, skipping
/// any submesh whose material name reads as foliage. The player should bump
/// into the trunk/branches but walk through the canopy.
fn build_collider(
    model: &Model,
    submeshes: &[crate::core::model::SubmeshData],
) -> Option<SharedShape> {
    if submeshes.is_empty() {
        return None;
    }
    let mut verts: Vec<Vector> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();
    let mut offset: u32 = 0;
    for sd in submeshes {
        if is_foliage_material(model, sd.material_index) {
            continue;
        }
        for p in &sd.positions {
            verts.push(Vector::new(p[0], p[1], p[2]));
        }
        for tri in sd.indices.chunks_exact(3) {
            indices.push([tri[0] + offset, tri[1] + offset, tri[2] + offset]);
        }
        offset += sd.positions.len() as u32;
    }
    // Some LODs (typically the billboard tier) only have foliage geometry,
    // which would leave us with nothing. Fall back to *all* submeshes in
    // that case so we still get a sensible collision shape.
    if verts.is_empty() {
        offset = 0;
        for sd in submeshes {
            for p in &sd.positions {
                verts.push(Vector::new(p[0], p[1], p[2]));
            }
            for tri in sd.indices.chunks_exact(3) {
                indices.push([tri[0] + offset, tri[1] + offset, tri[2] + offset]);
            }
            offset += sd.positions.len() as u32;
        }
    }
    if verts.is_empty() || indices.is_empty() {
        return None;
    }
    SharedShape::trimesh_with_flags(verts, indices, TriMeshFlags::FIX_INTERNAL_EDGES).ok()
}

fn is_foliage_material(model: &Model, material_index: usize) -> bool {
    let Some(name) = model.material_name_at(material_index) else {
        return false;
    };
    let lc = name.to_lowercase();
    lc.contains("cluster")
        || lc.contains("leaf")
        || lc.contains("leaves")
        || lc.contains("leav")    // Paine pine names its foliage material "leav"
        || lc.contains("foliage")
        || lc.contains("billboard")
}

/// Stable hash of (x, z) → variant index. Same input always picks the same variant,
/// so a chunk's scatter doesn't reshuffle on reload.
fn pick_variant_index(x: f32, z: f32, variant_count: usize) -> usize {
    if variant_count == 0 {
        return 0;
    }
    let mut h = (x.to_bits() as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h ^= (z.to_bits() as u64).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    (h as usize) % variant_count
}

/// One layer's instances on one chunk.
pub struct ScatterInstances {
    spec: ScatterSpec,
    /// `instances_by_variant[k]` — placements assigned to variant `k`. Empty slots
    /// are preserved so indices align with `ScatterModel::variants`.
    instances_by_variant: Vec<Vec<Matrix4<f32>>>,
    center: Point3<f32>,
    /// Colliders these instances added, so they can be taken out again
    /// without removing the chunk they stand on.
    colliders: Vec<ColliderHandle>,
    gpu: RefCell<Option<ScatterGpu>>,
}

struct ScatterGpu {
    variants: Vec<Option<VariantGpu>>,
}

struct VariantGpu {
    instance_vbo: GLuint,
    /// `vaos[lod_idx][submesh_idx]`
    vaos: Vec<Vec<GLuint>>,
    instance_count: GLsizei,
}

impl Drop for ScatterGpu {
    fn drop(&mut self) {
        unsafe {
            for kind in &self.variants {
                let Some(kind) = kind else { continue };
                gl::DeleteBuffers(1, &kind.instance_vbo);
                for lod_vaos in &kind.vaos {
                    if !lod_vaos.is_empty() {
                        gl::DeleteVertexArrays(lod_vaos.len() as GLsizei, lod_vaos.as_ptr());
                    }
                }
            }
        }
    }
}

impl ScatterInstances {
    pub fn new(spec: &ScatterSpec, placements: &[(f32, f32, f32, f32)]) -> Self {
        let spec = spec.clone();
        if placements.is_empty() {
            return Self {
                spec,
                instances_by_variant: Vec::new(),
                center: Point3::new(0.0, 0.0, 0.0),
                colliders: Vec::new(),
                gpu: RefCell::new(None),
            };
        }
        let variant_count = spec.model.variants.len();
        let mut instances_by_variant: Vec<Vec<Matrix4<f32>>> =
            (0..variant_count).map(|_| Vec::new()).collect();

        for &(x, y, z, yaw) in placements {
            let variant_idx = pick_variant_index(x, z, variant_count);
            let translation = Matrix4::from_translation(Vector3::new(x, y, z));
            let yaw_rot: Matrix4<f32> = Quaternion::from_angle_y(Rad(yaw)).into();
            instances_by_variant[variant_idx].push(translation * yaw_rot);
        }

        let sum: Vector3<f32> = placements
            .iter()
            .fold(Vector3::zero(), |acc, &(x, y, z, _)| {
                acc + Vector3::new(x, y, z)
            });
        let center = Point3::from_vec(sum / placements.len() as f32);

        Self {
            spec,
            instances_by_variant,
            center,
            colliders: Vec::new(),
            gpu: RefCell::new(None),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.instances_by_variant.iter().all(|v| v.is_empty())
    }

    /// Whether these instances are exactly what `spec` asks for.
    pub fn matches(&self, spec: &ScatterSpec) -> bool {
        self.spec.same_as(spec)
    }

    /// `parent` is the chunk's fixed body; parenting the colliders to it lets
    /// them be removed together with the chunk.
    pub fn add_colliders(
        &mut self,
        placements: &[(f32, f32, f32, f32)],
        scene: &mut Scene,
        parent: Option<rapier3d::prelude::RigidBodyHandle>,
    ) {
        let variant_count = self.spec.model.variants.len();
        for &(x, y, z, yaw) in placements {
            let variant_idx = pick_variant_index(x, z, variant_count);
            let translation = Vector::new(x, y, z);
            let rotation = Rotation::from_rotation_y(yaw);
            let pose = Pose::from_parts(translation, rotation);
            let collider = ColliderBuilder::new(self.spec.model.variants[variant_idx].collider_shape.clone())
                .position(pose)
                .build();
            self.colliders
                .push(scene.physics_engine.add_collider(collider, parent));
        }
    }

    /// Take these instances' colliders out of the physics world. Harmless
    /// if they are already gone with the chunk body.
    pub fn remove_colliders(&mut self, physics: &mut PhysicsEngine) {
        for handle in self.colliders.drain(..) {
            physics.remove_collider(handle);
        }
    }

    fn ensure_gpu(&self) {
        if self.gpu.borrow().is_some() {
            return;
        }
        let gpu = build_instances_gpu(&self.instances_by_variant, &self.spec.model);
        *self.gpu.borrow_mut() = Some(gpu);
    }
}

fn build_instances_gpu(
    instances_by_variant: &[Vec<Matrix4<f32>>],
    r: &ScatterModel,
) -> ScatterGpu {
    let mut kinds: Vec<Option<VariantGpu>> = Vec::with_capacity(instances_by_variant.len());
    for (variant_idx, instances) in instances_by_variant.iter().enumerate() {
        if instances.is_empty() {
            kinds.push(None);
            continue;
        }
        let kind = &r.variants[variant_idx];

        // Per-variant instance VBO.
        let mut buf: Vec<f32> = Vec::with_capacity(instances.len() * 16);
        for m in instances {
            let cols: [[f32; 4]; 4] = (*m).into();
            for c in &cols {
                buf.extend_from_slice(c);
            }
        }
        let instance_vbo = unsafe {
            let mut vbo = 0;
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (buf.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                buf.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            vbo
        };

        // One VAO per (LOD, submesh) within this variant.
        let mut vaos_per_lod: Vec<Vec<GLuint>> = Vec::with_capacity(kind.lods.len());
        for lod in &kind.lods {
            let mut lod_vaos: Vec<GLuint> = vec![0; lod.submeshes.len()];
            if !lod_vaos.is_empty() {
                unsafe {
                    gl::GenVertexArrays(lod_vaos.len() as GLsizei, lod_vaos.as_mut_ptr());
                    for (vao, sub) in lod_vaos.iter().zip(lod.submeshes.iter()) {
                        gl::BindVertexArray(*vao);

                        gl::BindBuffer(gl::ARRAY_BUFFER, sub.vbo);
                        gl::EnableVertexAttribArray(0);
                        gl::VertexAttribPointer(
                            0,
                            3,
                            gl::FLOAT,
                            gl::FALSE,
                            VERTEX_STRIDE,
                            POS_OFFSET as *const GLvoid,
                        );
                        gl::EnableVertexAttribArray(1);
                        gl::VertexAttribPointer(
                            1,
                            3,
                            gl::FLOAT,
                            gl::FALSE,
                            VERTEX_STRIDE,
                            NORMAL_OFFSET as *const GLvoid,
                        );
                        gl::EnableVertexAttribArray(2);
                        gl::VertexAttribPointer(
                            2,
                            2,
                            gl::FLOAT,
                            gl::FALSE,
                            VERTEX_STRIDE,
                            UV_OFFSET as *const GLvoid,
                        );

                        gl::BindBuffer(gl::ARRAY_BUFFER, instance_vbo);
                        for col in 0..4 {
                            let loc = 3 + col as GLuint;
                            let offset = (col * 16) as usize;
                            gl::EnableVertexAttribArray(loc);
                            gl::VertexAttribPointer(
                                loc,
                                4,
                                gl::FLOAT,
                                gl::FALSE,
                                INSTANCE_STRIDE,
                                offset as *const GLvoid,
                            );
                            gl::VertexAttribDivisor(loc, 1);
                        }
                        gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, sub.ibo);
                    }
                }
            }
            vaos_per_lod.push(lod_vaos);
        }
        unsafe {
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);
        }

        kinds.push(Some(VariantGpu {
            instance_vbo,
            vaos: vaos_per_lod,
            instance_count: instances.len() as GLsizei,
        }));
    }
    ScatterGpu { variants: kinds }
}

fn pick_lod_index(distance: f32, lod_count: usize) -> usize {
    if lod_count == 0 {
        return 0;
    }
    let max_lod = lod_count - 1;
    if distance < LOD_DIST_NEAR {
        return 0;
    }
    if distance < LOD_DIST_MID {
        return 1.min(max_lod);
    }
    if distance < LOD_DIST_FAR {
        return 2.min(max_lod);
    }
    3.min(max_lod)
}

impl Component for ScatterInstances {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        if self.is_empty() {
            return;
        }
        let Some(skylight) = scene.get_component::<SkyLight>() else {
            return;
        };
        let light_pos = skylight.get_position();

        let camera_pos = scene
            .get_active_camera()
            .map(|c| c.get_camera().get_eye_position());
        let distance = camera_pos
            .map(|p| {
                let d = self.center - p;
                (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
            })
            .unwrap_or(0.0);

        self.ensure_gpu();
        let gpu_borrow = self.gpu.borrow();
        let gpu = gpu_borrow.as_ref().expect("gpu just initialized");

        {
            let r = &*self.spec.model;
            r.shader.bind();
            r.shader.set_uniform_mat4("viewProjection", view_projection);
            r.shader.set_uniform_mat4("parentTransform", parent_transform);
            r.shader
                .set_uniform_3f("lightPosition", light_pos.x, light_pos.y, light_pos.z);

            unsafe {
                gl::Enable(gl::DEPTH_TEST);
                gl::DepthMask(gl::TRUE);
                gl::DepthFunc(gl::LESS);
                gl::Disable(gl::CULL_FACE);
                for (variant_idx, variant_gpu_opt) in gpu.variants.iter().enumerate() {
                    let Some(variant_gpu) = variant_gpu_opt else { continue };
                    let kind = &r.variants[variant_idx];
                    let lod_idx = pick_lod_index(distance, kind.lods.len());
                    let lod = &kind.lods[lod_idx];
                    let vaos = &variant_gpu.vaos[lod_idx];
                    for (sub_idx, sub) in lod.submeshes.iter().enumerate() {
                        bind_material_textures(
                            &r.shader,
                            kind.model.material_textures(sub.material_index),
                        );
                        gl::BindVertexArray(vaos[sub_idx]);
                        gl::DrawElementsInstanced(
                            gl::TRIANGLES,
                            sub.index_count,
                            gl::UNSIGNED_INT,
                            std::ptr::null(),
                            variant_gpu.instance_count,
                        );
                    }
                }
                gl::BindVertexArray(0);
                gl::Enable(gl::CULL_FACE);
                gl::Disable(gl::DEPTH_TEST);
            }
        }
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn on_remove(&mut self, physics: &mut PhysicsEngine) {
        self.remove_colliders(physics);
    }
}

fn bind_material_textures(
    shader: &Shader,
    textures: Option<&HashMap<TextureType, Texture>>,
) {
    let Some(textures) = textures else {
        return;
    };
    // PBR-authored materials (Acer pack) expose albedo as `BaseColor`; legacy
    // ones (Paine pine, the older Tree.fbx) use `Diffuse`. Pick whichever the
    // material actually carries — and if neither is tagged, fall back to any
    // non-normal texture so we don't inherit the previous draw's binding.
    let diffuse = textures
        .get(&TextureType::Diffuse)
        .or_else(|| textures.get(&TextureType::BaseColor))
        .or_else(|| {
            textures.iter().find_map(|(t, tex)| {
                if matches!(
                    t,
                    TextureType::Normals
                        | TextureType::NormalCamera
                        | TextureType::Height
                        | TextureType::Roughness
                        | TextureType::Metalness
                        | TextureType::AmbientOcclusion
                ) {
                    None
                } else {
                    Some(tex)
                }
            })
        });
    if let Some(diffuse) = diffuse {
        unsafe { gl::ActiveTexture(gl::TEXTURE0) };
        diffuse.bind();
        shader.set_uniform_1i("texture_diffuse", 0);
    }
    let normal = textures
        .get(&TextureType::Normals)
        .or_else(|| textures.get(&TextureType::NormalCamera))
        .or_else(|| textures.get(&TextureType::Height));
    if let Some(normal) = normal {
        unsafe { gl::ActiveTexture(gl::TEXTURE1) };
        normal.bind();
        shader.set_uniform_1i("texture_normals", 1);
    }
}

/// One scatter layer as stored in a scene file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScatterLayerData {
    /// Model path relative to `assets/models/`.
    pub model: String,
    /// Placements tried per 100 m x 100 m of terrain.
    pub density: f32,
}

/// Seconds an edited layer has to stay untouched before it is applied, so
/// typing a path or dragging the density does not replant on every change.
const APPLY_DELAY: f64 = 0.6;

struct ScatterLayer {
    /// As typed; empty for an unused slot.
    model: String,
    density: f32,
    /// What is planted right now.
    loaded: Option<Rc<ScatterModel>>,
    applied_density: f32,
    error: Option<String>,
    /// Countdown until the edited values are applied; `None` when they are.
    apply_in: Option<f64>,
}

impl ScatterLayer {
    fn new(model: String, density: f32) -> Self {
        Self {
            model,
            density,
            loaded: None,
            applied_density: density,
            error: None,
            apply_in: Some(0.0),
        }
    }

    /// The model path as used for loading and saving: trimmed, with forward
    /// slashes so a scene saved on Windows also loads on Linux.
    fn path(&self) -> String {
        self.model.trim().replace('\\', "/")
    }

    fn apply(&mut self) {
        self.applied_density = self.density;
        self.error = None;
        let path = self.path();
        if path.is_empty() {
            self.loaded = None;
            return;
        }
        let path = path.as_str();
        match ScatterModel::load(path) {
            Ok(model) => self.loaded = Some(model),
            Err(e) => {
                log::warn!("terrain scatter: could not load {path}: {e}");
                self.loaded = None;
                self.error = Some(e);
            }
        }
    }
}

/// The scatter layers of one terrain.
pub struct Scatter {
    layers: Vec<ScatterLayer>,
}

impl Scatter {
    /// Models are loaded on the first `tick`, on the GL thread; a missing
    /// model is reported in the status property and the log, never fatal.
    pub fn new(layers: Vec<ScatterLayerData>) -> Self {
        Self {
            layers: layers
                .into_iter()
                .map(|l| ScatterLayer::new(l.model, l.density))
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Append a layer; its model is loaded on the next `tick`.
    pub fn add_layer(&mut self, model: String, density: f32) {
        self.layers.push(ScatterLayer::new(model, density.max(0.0)));
    }

    /// Remove a layer. The terrain clears its instances on the next update.
    pub fn remove_layer(&mut self, index: usize) {
        if index < self.layers.len() {
            self.layers.remove(index);
        }
    }

    /// Point a layer at another model file (loaded on the next `tick`).
    pub fn set_model(&mut self, index: usize, model: String) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.model = model;
            layer.apply_in = Some(0.0);
        }
    }

    pub fn default_layers() -> Vec<ScatterLayerData> {
        DEFAULT_SCATTER
            .iter()
            .map(|&(model, density)| ScatterLayerData {
                model: model.to_string(),
                density,
            })
            .collect()
    }

    /// The layers that name a model, for saving.
    pub fn to_data(&self) -> Vec<ScatterLayerData> {
        self.layers
            .iter()
            .filter(|l| !l.path().is_empty())
            .map(|l| ScatterLayerData {
                model: l.path(),
                density: l.density,
            })
            .collect()
    }

    /// Apply edits whose delay has run out (loading models as needed).
    pub fn tick(&mut self, delta_time: f64) {
        for layer in &mut self.layers {
            let Some(remaining) = layer.apply_in else {
                continue;
            };
            let remaining = remaining - delta_time;
            if remaining > 0.0 {
                layer.apply_in = Some(remaining);
            } else {
                layer.apply_in = None;
                layer.apply();
            }
        }
    }

    /// What every chunk should carry right now, one entry per active layer.
    pub fn specs(&self) -> Vec<ScatterSpec> {
        let mut specs: Vec<ScatterSpec> = Vec::new();
        for layer in &self.layers {
            let Some(model) = &layer.loaded else {
                continue;
            };
            let chunk_area = CHUNK_SIZE_FLOAT * CHUNK_SIZE_FLOAT;
            let attempts = (layer.applied_density.max(0.0) * chunk_area / DENSITY_AREA).round() as u32;
            if attempts == 0 {
                continue;
            }
            let spec = ScatterSpec {
                model: model.clone(),
                attempts,
                salt: path_salt(&layer.path()),
            };
            // The same model at the same density twice would land on the
            // very same spots; plant it once.
            if !specs.iter().any(|s| s.same_as(&spec)) {
                specs.push(spec);
            }
        }
        specs
    }

    pub fn properties(&self) -> Vec<Property> {
        let mut properties = Vec::new();
        for (index, layer) in self.layers.iter().enumerate() {
            let number = index + 1;
            properties.push(Property::text(
                format!("{MODEL_PROPERTY}{number}"),
                layer.model.clone(),
            ));
            properties.push(Property::ranged(
                format!("{DENSITY_PROPERTY}{number}"),
                layer.density,
                0.0,
                MAX_DENSITY,
            ));
        }
        properties.push(Property::text("scatter", self.status()).read_only());
        properties
    }

    /// Index of the layer a `model N` property belongs to.
    pub fn model_property_layer(name: &str) -> Option<usize> {
        property_layer(name, MODEL_PROPERTY)
    }

    /// Returns whether `name` was one of the scatter properties.
    pub fn set_property(&mut self, name: &str, value: &PropertyValue) -> bool {
        if let Some(index) = property_layer(name, MODEL_PROPERTY) {
            if let (Some(layer), Some(text)) = (self.layers.get_mut(index), value.as_text()) {
                if text != layer.model {
                    layer.model = text.to_string();
                    layer.apply_in = Some(APPLY_DELAY);
                }
            }
            return true;
        }
        if let Some(index) = property_layer(name, DENSITY_PROPERTY) {
            if let (Some(layer), Some(density)) = (self.layers.get_mut(index), value.as_f32()) {
                let density = density.max(0.0);
                if density != layer.density {
                    layer.density = density;
                    layer.apply_in = Some(APPLY_DELAY);
                }
            }
            return true;
        }
        false
    }

    fn status(&self) -> String {
        let mut loaded = 0;
        let mut problems = Vec::new();
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.path().is_empty() {
                continue;
            }
            if layer.apply_in.is_some() {
                problems.push(format!("model {}: loading", index + 1));
            } else if let Some(error) = &layer.error {
                problems.push(format!("model {}: {error}", index + 1));
            } else if layer.loaded.is_some() {
                loaded += 1;
            }
        }
        if problems.is_empty() {
            match loaded {
                0 => "no models".into(),
                1 => "1 model loaded".into(),
                n => format!("{n} models loaded"),
            }
        } else {
            format!("{loaded} loaded; {}", problems.join("; "))
        }
    }
}

/// Property name prefixes of a layer; the 1-based layer number follows.
const MODEL_PROPERTY: &str = "model ";
const DENSITY_PROPERTY: &str = "density ";

fn property_layer(name: &str, prefix: &str) -> Option<usize> {
    name.strip_prefix(prefix)?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

/// Stable per-model salt (FNV-1a of the path), so a model keeps its spots
/// when layers are reordered.
fn path_salt(path: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
