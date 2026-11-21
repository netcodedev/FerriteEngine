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
    let bytes = std::fs::read(&path).ok()?;
    let image = match image::load_from_memory(&bytes) {
        Ok(image) => image.to_rgba8(),
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
    for (index, slot) in slots.iter().enumerate() {
        let Some(id) = slot.material else { continue };
        let auto = model.file_material(index).cloned();
        if let Some(material) = material_for_asset(id, auto.as_ref()) {
            model.set_material(index, material);
        }
    }
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
    let clean = |s: &str| -> String {
        let s: String = s
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        s.trim_matches('_').to_string()
    };
    let material = clean(material_name);
    let material = if material.is_empty() { "Material".to_string() } else { material };
    format!("{}_{material}{MATERIAL_SUFFIX}", clean(model_stem))
}

/// Give every material slot of model asset `model_id` that has none a
/// material asset of its own, written next to the model. Slots that have
/// one keep it (and the user's edits). The asset's own map overrides and
/// foliage flags (from before materials) go into the new materials.
/// Returns how many were made.
pub fn ensure_model_materials(database: &mut AssetDatabase, model_id: AssetId) -> Result<usize, String> {
    ensure_model_materials_with(database, model_id, false)
}

/// `ensure_model_materials`; with `windy`, the new materials sway in the
/// wind (`wind` 1): the model is planted by a scatter output with wind,
/// which swayed the whole model before materials had wind of their own.
fn ensure_model_materials_with(database: &mut AssetDatabase, model_id: AssetId, windy: bool) -> Result<usize, String> {
    let entry = database.get(model_id).ok_or_else(|| format!("asset {model_id} does not exist"))?.clone();
    let Some(settings) = entry.meta.as_model() else {
        return Ok(0);
    };
    if settings.materials.iter().all(|m| m.material.is_some()) {
        return Ok(0);
    }
    let stem = Path::new(&entry.meta.source)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("model")
        .to_string();
    let mut meta = entry.meta.clone();
    let mut made = 0;
    let mut taken: Vec<String> = Vec::new();
    {
        let settings = meta.as_model_mut().expect("a model");
        for slot in settings.materials.iter_mut().filter(|m| m.material.is_none()) {
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
            let first = material_file_name(&stem, &slot.name);
            let base = first.trim_end_matches(MATERIAL_SUFFIX).to_string();
            let mut file_name = first;
            let mut n = 2;
            while taken.contains(&file_name) || entry.dir.join(&file_name).exists() {
                file_name = format!("{base}_{n}{MATERIAL_SUFFIX}");
                n += 1;
            }
            taken.push(file_name.clone());
            let path = entry.dir.join(&file_name);
            file.save(&path)?;
            let id = database.import(&path).map_err(|e| e.to_string())?;
            slot.material = Some(id);
            slot.clear_legacy();
            made += 1;
        }
    }
    database.update(meta).map_err(|e| e.to_string())?;
    Ok(made)
}

/// `ensure_model_materials` for every model asset of the project (the
/// move to material assets). Returns how many materials were made and
/// one message per model that failed.
pub fn ensure_all_model_materials(database: &mut AssetDatabase) -> (usize, Vec<String>) {
    let models: Vec<AssetId> = database
        .iter()
        .filter(|e| e.meta.as_model().is_some_and(|m| m.materials.iter().any(|s| s.material.is_none())))
        .map(|e| e.meta.id)
        .collect();
    let windy = models_planted_with_wind(database);
    let mut made = 0;
    let mut problems = Vec::new();
    for id in models {
        match ensure_model_materials_with(database, id, windy.contains(&id)) {
            Ok(n) => made += n,
            Err(e) => problems.push(format!("model {id}: {e}")),
        }
    }
    (made, problems)
}

/// Make a new material (Standard PBR, defaults) or material graph (a copy
/// of Standard PBR to change) in `assets/materials/`, named `name` or the
/// first free "New material N". Returns its id.
pub fn create(database: &mut AssetDatabase, name: Option<&str>, graph: bool) -> Result<AssetId, String> {
    let suffix = if graph { super::graph::MATERIAL_GRAPH_SUFFIX } else { MATERIAL_SUFFIX };
    let folder = database.root().join("materials");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let base = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let stem = match base {
        Some(name) => {
            if folder.join(format!("{name}{suffix}")).exists() {
                return Err(format!("'{name}' exists already"));
            }
            name
        }
        None => {
            let word = if graph { "New material graph" } else { "New material" };
            (1..)
                .map(|n| format!("{word} {n}"))
                .find(|n| !folder.join(format!("{n}{suffix}")).exists())
                .expect("a free name")
        }
    };
    let path = folder.join(format!("{stem}{suffix}"));
    if graph {
        standard::standard_pbr().save(&path)?;
    } else {
        MaterialFile { alpha: AlphaMode::Opaque, ..Default::default() }.save(&path)?;
    }
    database.import(&path).map_err(|e| e.to_string())
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
