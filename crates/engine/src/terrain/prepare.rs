//! A scene's world made ready before the scene loads: a game's menu
//! prepares the world the game will play in (see the demo's
//! `docs/menus_and_loading.md`). `prepare_scene` makes the world's source
//! from the scene file's terrain (its generator starts planning on its
//! threads and keeps its plans in the world cache) and asks for the
//! models of its scatter graphs; `update`, every frame on the render
//! thread, reads and uploads them one after the other into the scatter
//! model cache. When the scene loads, its terrain takes the source made
//! here (`take`) and finds the models cached.

use std::sync::{Arc, Mutex};

use crate::core::{
    assets::{self, AssetKind, AssetRef},
    scene::scene_file::{ComponentData, EntityData, SceneFile},
};

use super::{generators, scatter::graph::ScatterGraph, scatter::ScatterModel, TerrainSource};

struct Prepared {
    scene: String,
    fingerprint: u64,
    source: Arc<dyn TerrainSource>,
    /// Models of the scatter graphs not read yet.
    models: Vec<AssetRef>,
    /// How many there were.
    model_count: usize,
}

static PREPARED: Mutex<Option<Prepared>> = Mutex::new(None);

/// How far preparing the world is (for a loading line).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progress {
    /// What the world itself is busy with (planning), if anything.
    pub world: Option<String>,
    pub models_done: usize,
    pub models: usize,
}

impl Progress {
    pub fn is_done(&self) -> bool {
        self.world.is_none() && self.models_done >= self.models
    }
}

/// Start preparing the world of the scene file at `path` (project
/// relative). Nothing happens when it is prepared or being prepared
/// already, or when the scene has no terrain.
pub fn prepare_scene(path: &str) -> Result<(), String> {
    if PREPARED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|p| p.scene == path) {
        return Ok(());
    }
    let text = String::from_utf8(assets::vfs::read(std::path::Path::new(path))?).map_err(|e| e.to_string())?;
    let file: SceneFile = ron::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let Some((seed, generator, settings, scatter)) = find_terrain(&file.entities) else {
        return Ok(());
    };
    let entry = generators::find(&generator).ok_or_else(|| format!("no world '{generator}' is loaded"))?;
    let fingerprint = entry.fingerprint(seed, &settings);
    let source = entry.create(seed, &settings)?;
    let mut models = Vec::new();
    for graph in scatter {
        for model in graph_models(&graph) {
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    log::info!("preparing the world of {path}: {} models to read", models.len());
    let model_count = models.len();
    *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Prepared { scene: path.to_string(), fingerprint, source, models, model_count });
    Ok(())
}

/// Read the next of the prepared world's models (call every frame on the
/// render thread). Returns how far preparing is, `None` when nothing is
/// being prepared.
pub fn update() -> Option<Progress> {
    let mut prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    let prepared = prepared.as_mut()?;
    // Asked for all at once (read side by side, see `model::background`);
    // those that are there are uploaded into the cache.
    prepared.models.retain(|asset| match ScatterModel::load_asset_background(asset) {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(e) => {
            log::warn!("preparing the world: {e}");
            false
        }
    });
    Some(Progress {
        world: prepared.source.status(),
        models_done: prepared.model_count - prepared.models.len(),
        models: prepared.model_count,
    })
}

/// How far preparing is, without doing more.
pub fn progress() -> Option<Progress> {
    let prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    let prepared = prepared.as_ref()?;
    Some(Progress {
        world: prepared.source.status(),
        models_done: prepared.model_count - prepared.models.len(),
        models: prepared.model_count,
    })
}

/// The source prepared for the world of `fingerprint`, taken (a terrain
/// making that world uses it instead of making it again).
pub(crate) fn take(fingerprint: u64) -> Option<Arc<dyn TerrainSource>> {
    let mut prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    if prepared.as_ref().is_some_and(|p| p.fingerprint == fingerprint) {
        return prepared.take().map(|p| p.source);
    }
    None
}

/// Stop preparing (the source and what is not read yet are dropped).
pub fn forget() {
    *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

type TerrainOf = (u64, String, ron::Value, Vec<AssetRef>);

fn find_terrain(entities: &[EntityData]) -> Option<TerrainOf> {
    for entity in entities {
        for component in &entity.components {
            if let ComponentData::Terrain { seed, generator, settings, scatter } = component {
                return Some((*seed, generator.clone(), settings.clone(), scatter.iter().map(|l| l.asset.clone()).collect()));
            }
        }
        if let Some(found) = find_terrain(&entity.children) {
            return Some(found);
        }
    }
    None
}

/// The model assets the scatter graph `graph` plants.
fn graph_models(graph: &AssetRef) -> Vec<AssetRef> {
    let source = {
        let database = assets::database();
        match database.get(graph.id) {
            Some(entry) if matches!(entry.meta.kind, AssetKind::ScatterGraph(_)) => entry.source_path(),
            _ => return Vec::new(),
        }
    };
    match ScatterGraph::load(&source) {
        Ok(graph) => graph.outputs().filter_map(|(_, spawn)| spawn.asset.clone()).collect(),
        Err(e) => {
            log::warn!("preparing the world: {}: {e}", source.display());
            Vec::new()
        }
    }
}
