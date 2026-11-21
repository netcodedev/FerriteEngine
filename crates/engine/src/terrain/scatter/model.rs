//! Loading scatter models: their variants, LODs, GPU buffers and colliders.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use rapier3d::prelude::{SharedShape, TriMeshFlags, Vector};

use crate::core::{
    assets::{self, AssetDatabase, AssetKind, AssetRef, MaterialMap, ModelSettings},
    gfx,
    model::{outline, Model, ModelBuilder},
    renderer::{
        pbr::{self},
        shader::Shader,
    },
};

use super::*;
use super::graph::{NodeId, ScatterGraph, ScatterRange};

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
    pub(super) shader: Rc<Shader>,
    pub(super) variants: Vec<ModelVariant>,
    /// The variants pictured for far away, once asked for.
    impostors: std::cell::OnceCell<Vec<super::impostor::Impostor>>,
}

/// Which detail tier to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LodChoice {
    /// The tier for this distance from the camera.
    Distance(f32),
    /// This tier (clamped to what the variant has), whatever the distance.
    Fixed(usize),
    /// The tier for a shadow map at this distance from the camera (see
    /// `shadow_lod_index`).
    Shadow(f32),
    /// Per group of nearby instances: culled against the view, and the
    /// tier for their distance from `eye` (for a shadow map, if `shadow`).
    PerCell { eye: Point3<f32>, shadow: bool },
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
        Ok(Rc::new(ScatterModel {
            shader,
            variants,
            impostors: std::cell::OnceCell::new(),
        }))
    }

    /// Number of variants loaded (always at least one).
    pub fn variant_count(&self) -> usize {
        self.variants.len()
    }

    /// Number of detail tiers of a variant.
    pub fn lod_count(&self, variant: usize) -> usize {
        self.variants.get(variant).map_or(0, |v| v.lods.len())
    }

    /// Every variant pictured for far away (see `impostor`), pictured on
    /// the first call. Must run on the GL thread.
    pub fn impostors(&self) -> &[super::impostor::Impostor] {
        self.impostors
            .get_or_init(|| self.variants.iter().map(super::impostor::bake).collect())
    }

    /// Corners of the box around a variant's most detailed tier, in model
    /// space with the asset's scale applied.
    pub fn bounds(&self, variant: usize) -> Option<([f32; 3], [f32; 3])> {
        self.variants.get(variant).map(|v| v.bounds)
    }
}

/// What one landscape output of a layer wants on every chunk: the model
/// planted where the output of the graph puts it.
#[derive(Clone)]
pub struct ScatterSpec {
    pub model: Rc<ScatterModel>,
    /// The layer's rules (for a model layer, `ScatterGraph::from_layer`).
    pub graph: Arc<ScatterGraph>,
    /// The `SpawnMesh` node of `graph` this is.
    pub output: NodeId,
    /// The graph's fingerprint (a hash of its file text): what was worked
    /// out for one graph is not taken for another.
    pub graph_key: u64,
    /// Tells this output's instances from others' (felled trees are kept
    /// by it). A model layer's is the one it always had.
    pub salt: u64,
    /// Planted over every loaded chunk, or around the camera.
    pub range: ScatterRange,
    /// Whether the models are drawn into the shadow maps.
    pub shadows: bool,
    /// How much they sway in the wind, 0..1.
    pub wind: f32,
    /// How far their shading normals bend towards the sky, 0..1.
    pub normals_up: f32,
}

impl ScatterSpec {
    /// A model on its own, placed by hand (previews): no rules of its own.
    pub fn of_model(model: Rc<ScatterModel>) -> Self {
        Self {
            model,
            graph: Arc::new(ScatterGraph::default()),
            output: NodeId(0),
            graph_key: 0,
            salt: 0,
            range: ScatterRange::Landscape,
            shadows: true,
            wind: 0.0,
            normals_up: 0.0,
        }
    }

    pub(super) fn same_as(&self, other: &ScatterSpec) -> bool {
        Rc::ptr_eq(&self.model, &other.model)
            && self.output == other.output
            && self.graph_key == other.graph_key
            && self.salt == other.salt
            && self.range == other.range
            && self.shadows == other.shadows
            && self.wind == other.wind
            && self.normals_up == other.normals_up
            && (Arc::ptr_eq(&self.graph, &other.graph) || self.graph == other.graph)
    }
}

pub(super) struct ModelVariant {
    /// Shared with sibling variants loaded from the same multi-variant asset, so
    /// material textures live in exactly one place.
    pub(super) model: Rc<Model>,
    /// Detail tiers sorted from highest quality (index 0) to lowest.
    pub(super) lods: Vec<VariantLod>,
    pub(super) collider_shape: SharedShape,
    /// Box around the most detailed tier: `(min, max)`.
    pub(super) bounds: ([f32; 3], [f32; 3]),
}

pub(super) struct VariantLod {
    pub(super) submeshes: Vec<SubmeshGpu>,
}

pub(super) struct SubmeshGpu {
    /// Interleaved as `vertex_layout`.
    pub(super) vertices: gfx::Buffer,
    pub(super) indices: gfx::Buffer,
    pub(super) index_count: u32,
    pub(super) material_index: usize,
}

impl SubmeshGpu {
    /// Bind the vertices (slot 0) and indices to the current pipeline.
    pub(super) fn bind(&self) {
        gfx::set_vertex_buffer(0, &self.vertices, 0, vertex_layout().stride);
        gfx::set_index_buffer(&self.indices, gfx::IndexFormat::U32);
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
                // Nothing is made where nothing draws (headless).
                let usage = gfx::BufferUsage::VERTEX | gfx::BufferUsage::RAY_TRACING_INPUT;
                let vertices = gfx::Buffer::from_slice(usage, gfx::BufferMemory::Static, &vertex_data);
                let usage = gfx::BufferUsage::INDEX | gfx::BufferUsage::RAY_TRACING_INPUT;
                let indices = gfx::Buffer::from_slice(usage, gfx::BufferMemory::Static, &sd.indices);
                submesh_gpu.push(SubmeshGpu {
                    vertices,
                    indices,
                    index_count: sd.indices.len() as u32,
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
pub(crate) fn pick_variant_index(x: f32, z: f32, variant_count: usize) -> usize {
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
