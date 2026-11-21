//! Material assets (`*.material.ron`) and material graph assets
//! (`*.matgraph.ron`), and turning them into the materials models draw
//! with. See docs/materials.md.
//!
//! A material file names its graph and sets some of its parameters; what
//! it leaves out keeps the graph's default. A texture parameter may be
//! `Auto`: whatever the model loader found for that part of the model
//! (the file's own maps, or maps named after it), so an imported model
//! looks as it did before it had material assets.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::Path,
    rc::Rc,
};

use serde::{Deserialize, Serialize};

use crate::core::{
    assets::{self, AssetDatabase, AssetId, AssetKind, MaterialSettings},
    graphics::Param,
    model::Model,
    renderer::texture::Texture,
};

use super::{
    graph::{MaterialGraph, ParamType},
    standard, AlphaMode, CompiledGraph, Material, ParamValue,
};

/// What a material file's name ends with.
pub const MATERIAL_SUFFIX: &str = ".material.ron";

pub fn is_material_file(path: &Path) -> bool {
    ends_with(path, MATERIAL_SUFFIX)
}

pub fn is_graph_file(path: &Path) -> bool {
    ends_with(path, super::graph::MATERIAL_GRAPH_SUFFIX)
}

fn ends_with(path: &Path, suffix: &str) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_lowercase().ends_with(suffix))
}

/// A file's name without `suffix`.
pub fn name_without(path: &Path, suffix: &str) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.to_lowercase().ends_with(suffix) {
        name[..name.len() - suffix.len()].to_string()
    } else {
        name.to_string()
    }
}

/// Which graph a material uses.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum GraphRef {
    /// One of the engine's (`Standard PBR`).
    Builtin(String),
    /// A material graph asset.
    Asset(AssetId),
}

impl Default for GraphRef {
    fn default() -> Self {
        GraphRef::Builtin(standard::STANDARD_PBR.into())
    }
}

/// A texture parameter's value.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum TextureSetting {
    /// What the model loader finds for the part (see the module's notes).
    #[default]
    Auto,
    /// No texture.
    None,
    /// A texture asset.
    Asset(AssetId),
}

/// A parameter's value in a material file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ParamSetting {
    /// A number; may follow a graphics level.
    Float(Param<f32>),
    /// A vector or colour (and a switch, 0 or 1, as its first number).
    Vector([f32; 4]),
    Texture(TextureSetting),
}

fn yes() -> bool {
    true
}

fn cutout() -> AlphaMode {
    AlphaMode::Cutout(0.5)
}

/// A material asset's file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MaterialFile {
    #[serde(default)]
    pub graph: GraphRef,
    #[serde(default)]
    pub params: BTreeMap<String, ParamSetting>,
    #[serde(default = "cutout")]
    pub alpha: AlphaMode,
    #[serde(default)]
    pub two_sided: bool,
    #[serde(default)]
    pub foliage: bool,
    #[serde(default = "yes")]
    pub shadows: bool,
}

impl Default for MaterialFile {
    fn default() -> Self {
        MaterialFile {
            graph: GraphRef::default(),
            params: BTreeMap::new(),
            alpha: cutout(),
            two_sided: false,
            foliage: false,
            shadows: true,
        }
    }
}

impl MaterialFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_ron(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default().struct_names(false))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::core::utils::write_atomically(path, self.to_ron()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

thread_local! {
    /// Graph assets by (id, revision).
    static GRAPHS: RefCell<HashMap<AssetId, (u64, Rc<CompiledGraph>)>> = RefCell::new(HashMap::new());
    /// Material files by (id, revision).
    static FILES: RefCell<HashMap<AssetId, (u64, Rc<MaterialFile>)>> = RefCell::new(HashMap::new());
    /// Texture assets by (id, revision, read as colour).
    static TEXTURES: RefCell<HashMap<(AssetId, bool), (u64, Texture)>> = RefCell::new(HashMap::new());
}

/// Forget every loaded graph, file and texture (the context goes away).
pub fn release() {
    let _ = GRAPHS.try_with(|g| g.borrow_mut().clear());
    let _ = FILES.try_with(|f| f.borrow_mut().clear());
    let _ = TEXTURES.try_with(|t| t.borrow_mut().clear());
}

/// Changes whenever any asset may have changed: models compare it to
/// apply their materials again.
pub fn generation() -> u64 {
    assets::database().changes()
}

/// The graph `graph` names. A graph asset that is missing or cannot be
/// read is Standard PBR (with a warning); a builtin of another name too.
pub fn graph_for(graph: &GraphRef) -> Rc<CompiledGraph> {
    match graph {
        GraphRef::Builtin(_) => super::standard_graph(),
        GraphRef::Asset(id) => {
            let (path, revision, name) = {
                let database = assets::database();
                match database.get(*id) {
                    Some(entry) => (entry.source_path(), entry.revision, entry.meta.name.clone()),
                    None => {
                        log::warn!("material: graph asset {id} does not exist");
                        return super::standard_graph();
                    }
                }
            };
            if let Some(graph) = GRAPHS.with(|g| {
                g.borrow().get(id).filter(|(r, _)| *r == revision).map(|(_, graph)| graph.clone())
            }) {
                return graph;
            }
            let graph = match MaterialGraph::load(&path) {
                Ok(graph) => CompiledGraph::new(&name, graph),
                Err(e) => {
                    log::warn!("material: {e}");
                    super::standard_graph()
                }
            };
            GRAPHS.with(|g| g.borrow_mut().insert(*id, (revision, graph.clone())));
            graph
        }
    }
}

/// A material asset's file, cached by revision.
pub fn material_file(id: AssetId) -> Option<(Rc<MaterialFile>, String)> {
    let (path, revision, name) = {
        let database = assets::database();
        let entry = database.get(id)?;
        if !matches!(entry.meta.kind, AssetKind::Material(_)) {
            return None;
        }
        (entry.source_path(), entry.revision, entry.meta.name.clone())
    };
    if let Some(file) = FILES.with(|f| f.borrow().get(&id).filter(|(r, _)| *r == revision).map(|(_, f)| f.clone())) {
        return Some((file, name));
    }
    let file = match MaterialFile::load(&path) {
        Ok(file) => Rc::new(file),
        Err(e) => {
            log::warn!("material: {e}");
            return None;
        }
    };
    FILES.with(|f| f.borrow_mut().insert(id, (revision, file.clone())));
    Some((file, name))
}

thread_local! {
    /// Texture asset files decoded ahead (`predecode_textures`), taken by
    /// `texture_asset`.
    static PREDECODED: RefCell<HashMap<std::path::PathBuf, crate::core::renderer::texture::decode::Decoded>> =
        RefCell::new(HashMap::new());
}

/// Decode on all cores the texture assets the material assets `materials`
/// use and that are not loaded yet, for `texture_asset` to take: one by
/// one, a model's materials held the editor up while a scene loaded.
fn predecode_textures(materials: impl Iterator<Item = AssetId>) {
    let mut ids = Vec::new();
    for material in materials {
        let Some((file, _)) = material_file(material) else { continue };
        for setting in file.params.values() {
            if let ParamSetting::Texture(TextureSetting::Asset(id)) = setting {
                ids.push(*id);
            }
        }
    }
    let paths: Vec<std::path::PathBuf> = {
        let database = assets::database();
        ids.iter()
            .filter_map(|id| database.get(*id).map(|e| (*id, e.source_path(), e.revision)))
            .filter(|(id, _, revision)| {
                // Loaded already, in either colour space.
                !TEXTURES.with(|t| {
                    let t = t.borrow();
                    [true, false].iter().any(|c| t.get(&(*id, *c)).is_some_and(|(r, _)| r == revision))
                })
            })
            .map(|(_, path, _)| path)
            .collect()
    };
    // What a `PreparedModel` decoded already is not done again.
    let paths: Vec<std::path::PathBuf> =
        PREDECODED.with(|p| paths.into_iter().filter(|path| !p.borrow().contains_key(path)).collect());
    if paths.len() < 2 {
        return;
    }
    let decoded = crate::core::renderer::texture::decode::decode_files(&paths);
    PREDECODED.with(|p| p.borrow_mut().extend(decoded));
}

/// Texture asset files decoded elsewhere (a `PreparedModel`), for the next
/// `apply_to_model` to take.
pub fn offer_textures(decoded: HashMap<std::path::PathBuf, crate::core::renderer::texture::decode::Decoded>) {
    PREDECODED.with(|p| p.borrow_mut().extend(decoded));
}

/// Drop what `offer_textures` gave and nothing took.
pub fn forget_offered_textures() {
    PREDECODED.with(|p| p.borrow_mut().clear());
}

/// The files of the texture assets the material assets `materials` use,
/// read from their files (no caches: any thread).
pub fn texture_files_of(materials: &[AssetId]) -> Vec<std::path::PathBuf> {
    let sources: Vec<std::path::PathBuf> = {
        let database = assets::database();
        materials.iter().filter_map(|id| database.get(*id).map(|e| e.source_path())).collect()
    };
    let mut textures = Vec::new();
    for source in sources {
        let Ok(file) = MaterialFile::load(&source) else { continue };
        for setting in file.params.values() {
            if let ParamSetting::Texture(TextureSetting::Asset(id)) = setting {
                textures.push(*id);
            }
        }
    }
    let database = assets::database();
    textures.iter().filter_map(|id| database.get(*id).map(|e| e.source_path())).collect()
}

/// A texture asset, loaded once (per colour space) and shared.
pub fn texture_asset(id: AssetId, color: bool) -> Option<Texture> {
    let (path, revision) = {
        let database = assets::database();
        let entry = database.get(id)?;
        (entry.source_path(), entry.revision)
    };
    if let Some(texture) = TEXTURES.with(|t| {
        t.borrow().get(&(id, color)).filter(|(r, _)| *r == revision).map(|(_, t)| t.share())
    }) {
        return Some(texture);
    }
    let decoded = PREDECODED
        .with(|p| p.borrow_mut().remove(&path))
        .unwrap_or_else(|| crate::core::renderer::texture::decode::decode_file(&path));
    let image = match decoded {
        Ok(image) => image,
        Err(e) => {
            log::warn!("material: texture {}: {e}", path.display());
            return None;
        }
    };
    let texture = Texture::new();
    let (width, height) = image.dimensions();
    texture.load_material_map(width, height, image.into_raw(), color);
    let shared = texture.share();
    TEXTURES.with(|t| t.borrow_mut().insert((id, color), (revision, texture)));
    Some(shared)
}

/// The material `file` describes. `auto` is what the model loader made of
/// the part (its maps fill `Auto` textures; with the same graph, its
/// numbers stay where the file sets none, e.g. a glTF file's colours).
pub fn build(file: &MaterialFile, name: &str, auto: Option<&Material>) -> Material {
    let graph = graph_for(&file.graph);
    let mut material = match auto {
        Some(auto) if Rc::ptr_eq(auto.graph(), &graph) => auto.clone(),
        _ => Material::new(graph.clone()),
    };
    material.name = name.to_string();
    for decl in &graph.graph.params {
        let setting = file.params.get(&decl.name);
        match (decl.ty, setting) {
            (ParamType::Texture { color }, setting) => {
                let texture = match setting {
                    Some(ParamSetting::Texture(TextureSetting::Asset(id))) => {
                        let found = texture_asset(*id, color);
                        if found.is_none() {
                            log::warn!("material {name}: texture asset {id} does not exist");
                        }
                        found
                    }
                    Some(ParamSetting::Texture(TextureSetting::None)) => None,
                    // Auto (or nothing said).
                    _ => auto.and_then(|a| a.texture(&decl.name)).map(Texture::share),
                };
                material.set_texture(&decl.name, texture);
            }
            (_, Some(ParamSetting::Float(param))) => {
                material.set_param(&decl.name, param.clone(), [param.high(); 4]);
            }
            (_, Some(ParamSetting::Vector(v))) => {
                material.set_number(&decl.name, *v);
            }
            _ => {}
        }
    }
    material.alpha = file.alpha;
    material.two_sided = file.two_sided;
    material.foliage = file.foliage;
    material.shadows = file.shadows;
    material
}

/// The material asset `id` for a part the loader made `auto` of.
pub fn material_for_asset(id: AssetId, auto: Option<&Material>) -> Option<Material> {
    let (file, name) = material_file(id)?;
    Some(build(&file, &name, auto))
}

/// Draw `model`'s parts with the material assets its asset's `slots`
/// name (slot `i` is the file's material `i`); parts without one keep
/// what the loader made of them.
pub fn apply_to_model(model: &mut Model, slots: &[MaterialSettings]) {
    model.reset_materials();
    predecode_textures(slots.iter().filter_map(|slot| slot.material));
    for (index, slot) in slots.iter().enumerate() {
        let Some(id) = slot.material else { continue };
        let index = model.file_material_index(index, &slot.name);
        let auto = model.file_material(index).cloned();
        if let Some(material) = material_for_asset(id, auto.as_ref()) {
            model.set_material(index, material);
        }
    }
    // What no material took (a file that failed) is not kept around.
    PREDECODED.with(|p| p.borrow_mut().clear());
}

/// The material file a material of a model file starts as: Standard PBR,
/// its maps found by the loader, cut out where the colour map says, and
/// foliage by its name.
pub fn imported_material(material_name: &str) -> MaterialFile {
    let foliage = crate::core::model::outline::looks_like_foliage(material_name);
    MaterialFile { foliage, ..Default::default() }
}

/// A file name for a model's material: `<model>_<material>`, letters,
/// digits, `-` and `_` only.
pub fn material_file_name(model_stem: &str, material_name: &str) -> String {
    format!("{}_{}{MATERIAL_SUFFIX}", clean_file_name(model_stem), material_stem(material_name))
}

/// A file name for a material several models share: `<material>`.
pub fn shared_material_file_name(material_name: &str) -> String {
    format!("{}{MATERIAL_SUFFIX}", material_stem(material_name))
}

fn material_stem(material_name: &str) -> String {
    let material = clean_file_name(material_name);
    if material.is_empty() {
        "Material".to_string()
    } else {
        material
    }
}

/// Letters, digits, `-` and `_` only.
fn clean_file_name(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    s.trim_matches('_').to_string()
}

/// Give every material slot of model asset `model_id` that has none a
/// material asset, written next to the model. Slots that have one keep it
/// (and the user's edits); slots no mesh uses get none. Slots that load
/// alike share one (see `ensure_all_model_materials`). The asset's own map
/// overrides and foliage flags (from before materials) go into the new
/// materials. Returns how many were made.
pub fn ensure_model_materials(database: &mut AssetDatabase, model_id: AssetId) -> Result<usize, String> {
    let windy = models_planted_with_wind(database);
    let (made, problems) = ensure_models(database, &[model_id], &windy);
    match problems.into_iter().next() {
        Some(problem) => Err(problem),
        None => Ok(made),
    }
}

/// `ensure_model_materials` for every model asset of the project (the
/// move to material assets). Materials of models a scatter graph plants
/// with wind sway in the wind (`wind` 1: the whole model swayed before
/// materials had wind of their own).
///
/// Slots of models in one folder that load alike (same material name, same
/// properties in the file, same maps found by name, same legacy settings
/// and wind) share one material asset, named after the material: the
/// building parts of a game all made from one `.mtl` get one `wood`
/// material to change in one place, not one per part. A material that
/// already serves every model with such a slot in the folder is reused.
///
/// Returns how many materials were made and one message per model that
/// failed.
pub fn ensure_all_model_materials(database: &mut AssetDatabase) -> (usize, Vec<String>) {
    let models: Vec<AssetId> = database
        .iter()
        .filter(|e| e.meta.as_model().is_some_and(|m| m.materials.iter().any(|s| s.needs_material())))
        .map(|e| e.meta.id)
        .collect();
    if models.is_empty() {
        return (0, Vec::new());
    }
    let windy = models_planted_with_wind(database);
    ensure_models(database, &models, &windy)
}

/// What decides how a model's material slot loads while every texture of
/// its material is `Auto`: slots with the same key look the same, so they
/// can share one material asset.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SlotKey {
    /// The model's folder (texture references are relative to it).
    dir: std::path::PathBuf,
    name: String,
    /// `ModelOutline`'s fingerprint of the file's material.
    fingerprint: u64,
    /// Maps the loader finds by name for it.
    maps_by_name: Vec<(crate::core::model::material_maps::MapKind, std::path::PathBuf)>,
    /// Settings from before materials (they go into the material).
    legacy: [Option<AssetId>; 5],
    legacy_foliage: Option<bool>,
    windy: bool,
}

/// The key of each of `entry`'s slots, `None` for a slot no mesh uses
/// (or whose material the file no longer has). `None` when the file
/// cannot be read.
fn slot_keys(entry: &assets::AssetEntry, windy: bool) -> Option<Vec<Option<SlotKey>>> {
    use crate::core::model::{material_maps, outline};
    let settings = entry.meta.as_model()?;
    let found = outline::inspect(&entry.source_path()).ok()?;
    let stem = Path::new(&entry.meta.source).file_stem()?.to_string_lossy().into_owned();
    let keys = settings
        .materials
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            // As `Model::file_material_index` finds it.
            let material = match found.materials.get(index) {
                Some(m) if m.name == slot.name => Some(m),
                _ => found.materials.iter().find(|m| m.name == slot.name),
            };
            let material = material?;
            if !material.used {
                return None;
            }
            let name = (!material.name.is_empty()).then_some(material.name.as_str());
            let prefixes = material_maps::name_prefixes(name, &stem);
            let files = material_maps::images_with_prefix(&entry.dir, &prefixes);
            Some(SlotKey {
                dir: entry.dir.clone(),
                name: slot.name.clone(),
                fingerprint: material.fingerprint,
                maps_by_name: material_maps::find_maps(&files, &prefixes),
                legacy: [slot.base_color, slot.normal, slot.roughness, slot.metallic, slot.occlusion],
                legacy_foliage: slot.legacy_foliage,
                windy,
            })
        })
        .collect();
    Some(keys)
}

/// The material a slot with `key` would share: the one every model of the
/// folder with such a slot already uses. Several different ones: none (the
/// user chose per model).
fn existing_shares(
    database: &AssetDatabase,
    dirs: &std::collections::HashSet<std::path::PathBuf>,
    skip: &std::collections::HashSet<AssetId>,
    windy: &std::collections::HashSet<AssetId>,
) -> HashMap<SlotKey, AssetId> {
    let mut seen: HashMap<SlotKey, Option<AssetId>> = HashMap::new();
    let models: Vec<&assets::AssetEntry> = database
        .iter()
        .filter(|e| dirs.contains(&e.dir) && !skip.contains(&e.meta.id))
        .filter(|e| e.meta.as_model().is_some_and(|m| m.materials.iter().any(|s| s.material.is_some())))
        .collect();
    for entry in models {
        let Some(keys) = slot_keys(entry, windy.contains(&entry.meta.id)) else {
            continue;
        };
        for (key, slot) in keys.into_iter().zip(&entry.meta.as_model().expect("a model").materials) {
            let (Some(key), Some(material)) = (key, slot.material) else {
                continue;
            };
            seen.entry(key)
                .and_modify(|known| {
                    if *known != Some(material) {
                        *known = None;
                    }
                })
                .or_insert(Some(material));
        }
    }
    seen.into_iter()
        .filter_map(|(key, material)| Some((key, material?)))
        .filter(|(_, material)| database.get(*material).is_some())
        .collect()
}

/// Give the slots of `models` that need one a material asset; see
/// `ensure_all_model_materials`.
fn ensure_models(
    database: &mut AssetDatabase,
    models: &[AssetId],
    windy: &std::collections::HashSet<AssetId>,
) -> (usize, Vec<String>) {
    let mut problems = Vec::new();
    // Each model with the keys of its slots (None: unreadable, no sharing).
    let mut pending: Vec<(assets::AssetEntry, Option<Vec<Option<SlotKey>>>)> = Vec::new();
    for &id in models {
        match database.get(id) {
            Some(entry) if entry.meta.as_model().is_some_and(|m| m.materials.iter().any(|s| s.needs_material())) => {
                let keys = slot_keys(entry, windy.contains(&id));
                pending.push((entry.clone(), keys));
            }
            Some(_) => {}
            None => problems.push(format!("model {id}: asset {id} does not exist")),
        }
    }
    // How many waiting slots share each key: those get a material named
    // after the material alone.
    let mut counts: HashMap<SlotKey, usize> = HashMap::new();
    for (entry, keys) in &pending {
        let Some(keys) = keys else { continue };
        for (key, slot) in keys.iter().zip(&entry.meta.as_model().expect("a model").materials) {
            if let (Some(key), true) = (key, slot.needs_material()) {
                *counts.entry(key.clone()).or_default() += 1;
            }
        }
    }
    let dirs = pending.iter().map(|(e, _)| e.dir.clone()).collect();
    let skip = pending.iter().map(|(e, _)| e.meta.id).collect();
    let mut shared = existing_shares(database, &dirs, &skip, windy);

    let mut made = 0;
    for (entry, keys) in pending {
        let id = entry.meta.id;
        match ensure_slots(database, entry, keys, windy.contains(&id), &counts, &mut shared) {
            Ok(n) => made += n,
            Err(e) => problems.push(format!("model {id}: {e}")),
        }
    }
    (made, problems)
}

/// A material made for one model (`<model>_<material>`) that another
/// model now shares is renamed `<material>`, if that name is free.
fn name_after_material(database: &mut AssetDatabase, id: AssetId, material_name: &str) {
    let Some(entry) = database.get(id) else { return };
    let shared = shared_material_file_name(material_name);
    let made_for_one = entry.meta.source.ends_with(&format!("_{shared}"));
    if made_for_one && !entry.dir.join(&shared).exists() {
        if let Err(e) = database.rename(id, &material_stem(material_name)) {
            log::warn!("material {}: {e}", entry_name(database, id));
        }
    }
}

fn entry_name(database: &AssetDatabase, id: AssetId) -> String {
    database.get(id).map(|e| e.meta.name.clone()).unwrap_or_else(|| id.to_string())
}

/// One model of `ensure_models`.
fn ensure_slots(
    database: &mut AssetDatabase,
    entry: assets::AssetEntry,
    keys: Option<Vec<Option<SlotKey>>>,
    windy: bool,
    counts: &HashMap<SlotKey, usize>,
    shared: &mut HashMap<SlotKey, AssetId>,
) -> Result<usize, String> {
    let stem = Path::new(&entry.meta.source)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();
    let mut meta = entry.meta.clone();
    let mut made = 0;
    let settings = meta.as_model_mut().expect("a model");
    for (index, slot) in settings.materials.iter_mut().enumerate() {
        if !slot.needs_material() {
            continue;
        }
        let key = match &keys {
            // No mesh uses it: nothing to draw, no material.
            Some(keys) if keys[index].is_none() => {
                slot.unused = true;
                slot.clear_legacy();
                continue;
            }
            Some(keys) => keys[index].clone(),
            None => None,
        };
        if let Some(&material) = key.as_ref().and_then(|k| shared.get(k)) {
            name_after_material(database, material, &slot.name);
            slot.material = Some(material);
            slot.clear_legacy();
            continue;
        }
        let mut file = imported_material(&slot.name);
        if let Some(foliage) = slot.legacy_foliage {
            file.foliage = foliage;
        }
        if windy {
            file.params.insert(standard::params::WIND.into(), ParamSetting::Float(Param::Fixed(1.0)));
        }
        for (map, param) in [
            (slot.base_color, standard::params::BASE_COLOR_MAP),
            (slot.normal, standard::params::NORMAL_MAP),
            (slot.roughness, standard::params::ROUGHNESS_MAP),
            (slot.metallic, standard::params::METALLIC_MAP),
            (slot.occlusion, standard::params::OCCLUSION_MAP),
        ] {
            if let Some(id) = map {
                file.params.insert(param.into(), ParamSetting::Texture(TextureSetting::Asset(id)));
            }
        }
        let sharing = key.as_ref().is_some_and(|k| counts.get(k).copied().unwrap_or(0) > 1);
        let first = if sharing {
            shared_material_file_name(&slot.name)
        } else {
            material_file_name(&stem, &slot.name)
        };
        let base = first.trim_end_matches(MATERIAL_SUFFIX).to_string();
        let mut file_name = first;
        let mut n = 2;
        while entry.dir.join(&file_name).exists() {
            file_name = format!("{base}_{n}{MATERIAL_SUFFIX}");
            n += 1;
        }
        let path = entry.dir.join(&file_name);
        file.save(&path)?;
        let id = database.import(&path).map_err(|e| e.to_string())?;
        slot.material = Some(id);
        slot.clear_legacy();
        if let Some(key) = key {
            shared.insert(key, id);
        }
        made += 1;
    }
    database.update(meta).map_err(|e| e.to_string())?;
    Ok(made)
}

/// Make a new material (Standard PBR, defaults) or material graph (a copy
/// of Standard PBR to change) in `assets/materials/`, named `name` or the
/// first free "New material N". Returns its id. See `assets::create` for
/// other kinds and folders.
pub fn create(database: &mut AssetDatabase, name: Option<&str>, graph: bool) -> Result<AssetId, String> {
    let kind = if graph { assets::NewAsset::MaterialGraph } else { assets::NewAsset::Material };
    assets::create::create(database, kind, None, name)
}

/// Model assets a scatter graph asset plants with wind.
fn models_planted_with_wind(database: &AssetDatabase) -> std::collections::HashSet<AssetId> {
    use crate::terrain::scatter::graph::ScatterGraph;
    database
        .iter()
        .filter(|e| matches!(e.meta.kind, AssetKind::ScatterGraph(_)))
        .filter_map(|e| ScatterGraph::load(&e.source_path()).ok())
        .flat_map(|graph| {
            graph
                .outputs()
                .filter(|(_, spawn)| spawn.wind.high() > 0.0 || spawn.wind.is_bound())
                .filter_map(|(_, spawn)| spawn.asset.as_ref().map(|a| a.id))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The value of a material's parameter as a material file writes it.
pub fn setting_of(value: &ParamValue) -> Option<ParamSetting> {
    match value {
        ParamValue::Number(Param::Fixed(_), v) => Some(ParamSetting::Vector(*v)),
        ParamValue::Number(param, _) => Some(ParamSetting::Float(param.clone())),
        ParamValue::Texture(_) => None,
    }
}
