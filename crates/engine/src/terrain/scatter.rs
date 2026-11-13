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
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cgmath::{EuclideanSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Vector3, Zero};
use gl::types::{GLsizei, GLsizeiptr, GLuint, GLvoid};
use glfw::WindowEvent;
use rapier3d::prelude::{
    ColliderBuilder, ColliderHandle, Pose, Rotation, SharedShape, TriMeshFlags, Vector,
};
use serde::{Deserialize, Serialize};

use crate::core::{
    assets::{self, AssetDatabase, AssetKind, AssetRef, MaterialMap, ModelSettings},
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    model::{outline, Model, ModelBuilder},
    physics::physics_engine::PhysicsEngine,
    renderer::{
        pbr::{self, Lighting},
        shader::Shader,
    },
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

/// Everything loaded so far, by model path or asset (with its revision).
/// Failed loads are not cached, so a model that was missing is tried again
/// the next time it is asked for.
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

/// Which detail tier to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LodChoice {
    /// The tier for this distance from the camera.
    Distance(f32),
    /// This tier (clamped to what the variant has), whatever the distance.
    Fixed(usize),
}

/// How a model file is turned into scatter variants.
struct LoadOptions {
    scale: f32,
    /// Only the variant of this name; all of them without.
    variant: Option<String>,
    /// Per material: maps replaced by these image files.
    maps: Vec<Vec<(MaterialMap, PathBuf)>>,
    /// Per material: leave it out of the collision shape. Materials beyond
    /// the end are judged by their name.
    foliage: Vec<bool>,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            variant: None,
            maps: Vec::new(),
            foliage: Vec::new(),
        }
    }
}

impl LoadOptions {
    /// The options a model asset's settings ask for. Texture assets that do
    /// not exist are skipped with a warning.
    fn for_settings(
        settings: &ModelSettings,
        variant: Option<String>,
        database: &AssetDatabase,
    ) -> Self {
        let texture = |id: &Option<assets::AssetId>| {
            let id = (*id)?;
            let found = database.get(id).map(|texture| texture.source_path());
            if found.is_none() {
                log::warn!("terrain scatter: texture asset {id} does not exist");
            }
            found
        };
        LoadOptions {
            scale: settings.scale,
            variant,
            maps: settings
                .materials
                .iter()
                .map(|m| {
                    MaterialMap::ALL
                        .into_iter()
                        .filter_map(|map| texture(&m.map(map)).map(|file| (map, file)))
                        .collect()
                })
                .collect(),
            foliage: settings.materials.iter().map(|m| m.foliage).collect(),
        }
    }
}

impl ScatterModel {
    /// Load a model file (path relative to `assets/models/`), or hand out
    /// the cached one. Must run on the GL thread.
    pub fn load(path: &str) -> Result<Rc<ScatterModel>, String> {
        Self::load_cached(
            path.to_string(),
            &format!("assets/models/{path}"),
            &LoadOptions::default(),
        )
    }

    /// Load a model asset — all of it, or the variant the reference names —
    /// with the asset's settings applied. Returns the model and the asset
    /// revision it was loaded at. Must run on the GL thread.
    pub fn load_asset(asset: &AssetRef) -> Result<(Rc<ScatterModel>, u64), String> {
        // Take what is needed out of the database and let go of it before
        // the (slow) load.
        let (source, options, revision) = {
            let database = assets::database();
            let entry = database
                .get(asset.id)
                .ok_or_else(|| format!("asset {} does not exist", asset.id))?;
            let AssetKind::Model(settings) = &entry.meta.kind else {
                return Err(format!("'{}' is not a model", entry.meta.name));
            };
            let options = LoadOptions::for_settings(settings, asset.part.clone(), &database);
            (entry.source_path(), options, entry.revision)
        };
        let part = asset.part.as_deref().unwrap_or("");
        let prefix = format!("asset:{}:{part}:", asset.id);
        let key = format!("{prefix}{revision}");
        // Older revisions of the same asset are of no use any more.
        MODEL_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .models
                .retain(|k, _| !k.starts_with(&prefix) || *k == key)
        });
        let model = Self::load_cached(key, &source.to_string_lossy().replace('\\', "/"), &options)?;
        Ok((model, revision))
    }

    /// Load a model file with the given settings, which need not be saved
    /// (or belong to a registered asset) yet — for previewing edits. Not
    /// cached. Must run on the GL thread.
    pub fn load_preview(
        source: &Path,
        settings: &ModelSettings,
        variant: Option<&str>,
    ) -> Result<Rc<ScatterModel>, String> {
        let options = {
            let database = assets::database();
            LoadOptions::for_settings(settings, variant.map(str::to_string), &database)
        };
        Self::build(&source.to_string_lossy().replace('\\', "/"), &options)
    }

    fn load_cached(
        key: String,
        full_path: &str,
        options: &LoadOptions,
    ) -> Result<Rc<ScatterModel>, String> {
        if let Some(cached) = MODEL_CACHE.with(|cache| cache.borrow().models.get(&key).cloned()) {
            return Ok(cached);
        }
        let model = Self::build(full_path, options)?;
        MODEL_CACHE.with(|cache| cache.borrow_mut().models.insert(key, model.clone()));
        Ok(model)
    }

    fn build(full_path: &str, options: &LoadOptions) -> Result<Rc<ScatterModel>, String> {
        let variants = load_variants(full_path, options).map_err(|e| e.to_string())?;
        let shader = MODEL_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .shader
                .get_or_insert_with(|| {
                    Rc::new(Shader::new(
                        include_str!("scatter_instanced_vertex.glsl"),
                        &pbr::fragment_source(include_str!("scatter_instanced_fragment.glsl")),
                    ))
                })
                .clone()
        });
        Ok(Rc::new(ScatterModel { shader, variants }))
    }

    /// Number of variants loaded (always at least one).
    pub fn variant_count(&self) -> usize {
        self.variants.len()
    }

    /// Number of detail tiers of a variant.
    pub fn lod_count(&self, variant: usize) -> usize {
        self.variants.get(variant).map_or(0, |v| v.lods.len())
    }

    /// Corners of the box around a variant's most detailed tier, in model
    /// space with the asset's scale applied.
    pub fn bounds(&self, variant: usize) -> Option<([f32; 3], [f32; 3])> {
        self.variants.get(variant).map(|v| v.bounds)
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
    /// Box around the most detailed tier: `(min, max)`.
    bounds: ([f32; 3], [f32; 3]),
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

fn load_variants(
    path: &str,
    options: &LoadOptions,
) -> Result<Vec<ModelVariant>, Box<dyn std::error::Error>> {
    let mut model = ModelBuilder::from_file(path)?
        .with_scale(MODEL_SCALE * options.scale)
        .build();
    model.init();
    // Textures assigned to the asset replace what the file brought along.
    for (material, maps) in options.maps.iter().enumerate() {
        for (map, file) in maps {
            if !model.set_material_map_file(material, *map, file) {
                log::warn!("terrain scatter: could not load texture {}", file.display());
            }
        }
    }
    let mut groups = model.extract_tree_groups();
    if let Some(wanted) = &options.variant {
        groups.retain(|group| group.name == *wanted);
        if groups.is_empty() {
            return Err(format!("the model has no variant '{wanted}'").into());
        }
    }
    if groups.is_empty() {
        return Err(format!("no geometry found in '{path}'").into());
    }

    let shared_model = Rc::new(model);

    let mut out = Vec::with_capacity(groups.len());
    for group in groups.iter_mut() {
        group.lods.sort_by_key(|(name, _)| outline::lod_priority(name));

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
        let collider_shape =
            build_collider(&shared_model, &group.lods[collider_idx].1, &options.foliage)
            .ok_or_else(|| format!("'{}' failed to build collider trimesh", group.name))?;

        let mut bounds = ([f32::MAX; 3], [f32::MIN; 3]);
        for position in group.lods[0].1.iter().flat_map(|sd| sd.positions.iter()) {
            for axis in 0..3 {
                bounds.0[axis] = bounds.0[axis].min(position[axis]);
                bounds.1[axis] = bounds.1[axis].max(position[axis]);
            }
        }
        if bounds.0[0] > bounds.1[0] {
            bounds = ([0.0; 3], [0.0; 3]);
        }

        out.push(ModelVariant {
            model: shared_model.clone(),
            lods: gpu_lods,
            collider_shape,
            bounds,
        });
    }
    Ok(out)
}

/// Build a shared trimesh collider from a single LOD's submeshes, skipping
/// any submesh whose material is foliage. The player should bump into the
/// trunk/branches but walk through the canopy.
fn build_collider(
    model: &Model,
    submeshes: &[crate::core::model::SubmeshData],
    foliage: &[bool],
) -> Option<SharedShape> {
    if submeshes.is_empty() {
        return None;
    }
    let mut verts: Vec<Vector> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();
    let mut offset: u32 = 0;
    for sd in submeshes {
        if is_foliage_material(model, sd.material_index, foliage) {
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

/// `foliage` holds the asset's per-material flags; a material it does not
/// cover (a raw model file has no flags at all) is judged by its name.
fn is_foliage_material(model: &Model, material_index: usize, foliage: &[bool]) -> bool {
    match foliage.get(material_index) {
        Some(flag) => *flag,
        None => model
            .material_name_at(material_index)
            .is_some_and(|name| outline::looks_like_foliage(&name)),
    }
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
        let Some(lighting) = Lighting::from_scene(scene) else {
            return;
        };
        let d = self.center - lighting.camera_position;
        let distance = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        self.draw(
            view_projection,
            parent_transform,
            &lighting,
            LodChoice::Distance(distance),
        );
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn on_remove(&mut self, physics: &mut PhysicsEngine) {
        self.remove_colliders(physics);
    }
}

impl ScatterInstances {
    /// Draw the instances. Needs no scene, so previews can use it too.
    pub fn draw(
        &self,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
        lighting: &Lighting,
        lod: LodChoice,
    ) {
        if self.is_empty() {
            return;
        }
        self.ensure_gpu();
        let gpu_borrow = self.gpu.borrow();
        let gpu = gpu_borrow.as_ref().expect("gpu just initialized");

        {
            let r = &*self.spec.model;
            r.shader.bind();
            r.shader.set_uniform_mat4("viewProjection", view_projection);
            r.shader.set_uniform_mat4("parentTransform", parent_transform);
            lighting.apply(&r.shader);

            unsafe {
                gl::Enable(gl::DEPTH_TEST);
                gl::DepthMask(gl::TRUE);
                gl::DepthFunc(gl::LESS);
                gl::Disable(gl::CULL_FACE);
                for (variant_idx, variant_gpu_opt) in gpu.variants.iter().enumerate() {
                    let Some(variant_gpu) = variant_gpu_opt else { continue };
                    let kind = &r.variants[variant_idx];
                    let lod_idx = match lod {
                        LodChoice::Distance(distance) => pick_lod_index(distance, kind.lods.len()),
                        LodChoice::Fixed(tier) => tier.min(kind.lods.len().saturating_sub(1)),
                    };
                    let lod = &kind.lods[lod_idx];
                    let vaos = &variant_gpu.vaos[lod_idx];
                    for (sub_idx, sub) in lod.submeshes.iter().enumerate() {
                        pbr::bind_material(
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
}

/// One scatter layer as stored in a scene file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScatterLayerData {
    /// The model asset to scatter: all its variants, or the one named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    /// A raw model file, as a path relative to `assets/models/`. Used by
    /// scenes from before assets existed and when `asset` is not set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Placements tried per 100 m x 100 m of terrain.
    pub density: f32,
}

impl ScatterLayerData {
    pub fn asset(asset: AssetRef, density: f32) -> Self {
        Self {
            asset: Some(asset),
            model: String::new(),
            density,
        }
    }

    pub fn file(model: impl Into<String>, density: f32) -> Self {
        Self {
            asset: None,
            model: model.into(),
            density,
        }
    }
}

/// Seconds an edited layer has to stay untouched before it is applied, so
/// typing a path or dragging the density does not replant on every change.
const APPLY_DELAY: f64 = 0.6;

struct ScatterLayer {
    /// The asset to scatter. Takes precedence over `model`.
    asset: Option<AssetRef>,
    /// Raw model file as typed; empty when unused.
    model: String,
    density: f32,
    /// What is planted right now.
    loaded: Option<Rc<ScatterModel>>,
    /// Revision of the asset `loaded` was built from.
    loaded_revision: u64,
    applied_density: f32,
    error: Option<String>,
    /// Countdown until the edited values are applied; `None` when they are.
    apply_in: Option<f64>,
}

impl ScatterLayer {
    fn new(mut data: ScatterLayerData) -> Self {
        // A raw file that is registered as an asset is referred to as that
        // asset from here on; scenes from before assets existed are brought
        // up to date this way.
        if data.asset.is_none() && !data.model.trim().is_empty() {
            let database = assets::database();
            let source = database
                .root()
                .join("models")
                .join(data.model.trim().replace('\\', "/"));
            if let Some(id) = database.find_by_source(&source) {
                data.asset = Some(AssetRef::whole(id));
                data.model.clear();
            }
        }
        Self {
            asset: data.asset,
            model: data.model,
            density: data.density,
            loaded: None,
            loaded_revision: 0,
            applied_density: data.density,
            error: None,
            apply_in: Some(0.0),
        }
    }

    /// Stable per-model salt, so a model keeps its spots when layers are
    /// reordered.
    fn salt(&self) -> u64 {
        match &self.asset {
            Some(asset) => fnv1a(&format!(
                "{}/{}",
                asset.id,
                asset.part.as_deref().unwrap_or("")
            )),
            None => fnv1a(&self.path()),
        }
    }

    /// The model path as used for loading and saving: trimmed, with forward
    /// slashes so a scene saved on Windows also loads on Linux.
    fn path(&self) -> String {
        self.model.trim().replace('\\', "/")
    }

    /// Neither an asset nor a file: the layer scatters nothing.
    fn is_unused(&self) -> bool {
        self.asset.is_none() && self.path().is_empty()
    }

    /// What the layer scatters, for messages.
    fn label(&self) -> String {
        match &self.asset {
            Some(asset) => assets::database()
                .display_name(asset)
                .unwrap_or_else(|| format!("missing asset {}", asset.id)),
            None => self.path(),
        }
    }

    /// Whether the asset was saved again since it was loaded.
    fn is_outdated(&self) -> bool {
        let (Some(asset), Some(_)) = (&self.asset, &self.loaded) else {
            return false;
        };
        assets::database()
            .get(asset.id)
            .is_some_and(|entry| entry.revision != self.loaded_revision)
    }

    fn apply(&mut self) {
        self.applied_density = self.density;
        self.error = None;
        self.loaded = None;
        let result = match &self.asset {
            Some(asset) => ScatterModel::load_asset(asset).map(|(model, revision)| {
                self.loaded_revision = revision;
                model
            }),
            None => {
                let path = self.path();
                if path.is_empty() {
                    return;
                }
                ScatterModel::load(&path)
            }
        };
        match result {
            Ok(model) => self.loaded = Some(model),
            Err(e) => {
                log::warn!("terrain scatter: could not load {}: {e}", self.label());
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
            layers: layers.into_iter().map(ScatterLayer::new).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Append a layer scattering a raw model file; it is loaded on the next
    /// `tick`.
    pub fn add_layer(&mut self, model: String, density: f32) {
        self.layers
            .push(ScatterLayer::new(ScatterLayerData::file(model, density.max(0.0))));
    }

    /// Append a layer scattering a model asset; it is loaded on the next
    /// `tick`.
    pub fn add_asset_layer(&mut self, asset: AssetRef, density: f32) {
        self.layers
            .push(ScatterLayer::new(ScatterLayerData::asset(asset, density.max(0.0))));
    }

    /// Point a layer at a model asset (loaded on the next `tick`).
    pub fn set_asset(&mut self, index: usize, asset: AssetRef) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.asset = Some(asset);
            layer.model.clear();
            layer.apply_in = Some(0.0);
        }
    }

    /// The asset a layer scatters, if it uses one.
    pub fn asset(&self, index: usize) -> Option<&AssetRef> {
        self.layers.get(index)?.asset.as_ref()
    }

    /// Remove a layer. The terrain clears its instances on the next update.
    pub fn remove_layer(&mut self, index: usize) {
        if index < self.layers.len() {
            self.layers.remove(index);
        }
    }

    /// Point a layer at a raw model file (loaded on the next `tick`).
    pub fn set_model(&mut self, index: usize, model: String) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.asset = None;
            layer.model = model;
            layer.apply_in = Some(0.0);
        }
    }

    pub fn default_layers() -> Vec<ScatterLayerData> {
        DEFAULT_SCATTER
            .iter()
            .map(|&(model, density)| ScatterLayerData::file(model, density))
            .collect()
    }

    /// The layers that scatter something, for saving.
    pub fn to_data(&self) -> Vec<ScatterLayerData> {
        self.layers
            .iter()
            .filter(|l| !l.is_unused())
            .map(|l| match &l.asset {
                Some(asset) => ScatterLayerData::asset(asset.clone(), l.density),
                None => ScatterLayerData::file(l.path(), l.density),
            })
            .collect()
    }

    /// Apply edits whose delay has run out (loading models as needed) and
    /// reload layers whose asset was saved again.
    pub fn tick(&mut self, delta_time: f64) {
        for layer in &mut self.layers {
            if layer.apply_in.is_none() && layer.is_outdated() {
                layer.apply_in = Some(0.0);
            }
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
                salt: layer.salt(),
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
            let name = format!("{MODEL_PROPERTY}{number}");
            properties.push(match &layer.asset {
                // An asset is chosen, not typed.
                Some(_) => Property::text(name, layer.label()).read_only(),
                None => Property::text(name, layer.model.clone()),
            });
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
                if layer.asset.is_none() && text != layer.model {
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
            if layer.is_unused() {
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

/// FNV-1a.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::AssetId;

    #[test]
    fn layers_from_before_assets_still_load() {
        let layer: ScatterLayerData =
            ron::from_str("(model: \"Trees/Trees.gltf\", density: 21.0)").unwrap();
        assert_eq!(layer, ScatterLayerData::file("Trees/Trees.gltf", 21.0));
    }

    #[test]
    fn asset_layers_round_trip_without_an_empty_path() {
        let layer = ScatterLayerData::asset(AssetRef::part(AssetId::from(0xabc), "Oak"), 12.5);
        let text = ron::to_string(&layer).unwrap();
        assert!(!text.contains("model"), "{text}");
        assert_eq!(ron::from_str::<ScatterLayerData>(&text).unwrap(), layer);

        let file = ron::to_string(&ScatterLayerData::file("rock.gltf", 3.0)).unwrap();
        assert!(!file.contains("asset"), "{file}");
    }

    #[test]
    fn an_asset_and_a_file_scatter_to_different_spots() {
        let id = AssetId::from(7);
        let whole = ScatterLayer::new(ScatterLayerData::asset(AssetRef::whole(id), 1.0));
        let part = ScatterLayer::new(ScatterLayerData::asset(AssetRef::part(id, "Oak"), 1.0));
        let file = ScatterLayer::new(ScatterLayerData::file("rock.gltf", 1.0));
        assert_ne!(whole.salt(), part.salt());
        assert_ne!(whole.salt(), file.salt());
        // Unused: neither an asset nor a path.
        assert!(ScatterLayer::new(ScatterLayerData::file("  ", 1.0)).is_unused());
        assert!(!whole.is_unused());
    }
}
