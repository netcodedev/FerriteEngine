//! Scatter layers, as stored and as loaded: a model plus a density (and a
//! tag), or a scatter graph.

use std::{rc::Rc, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::core::assets::{self, AssetKind, AssetRef};

use super::graph::{NodeId, ScatterGraph, ScatterRange};

use super::*;

/// One scatter layer as stored in a scene file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScatterLayerData {
    /// The asset to scatter: a model (all its variants, or the one named),
    /// or a scatter graph (then `density` and `tag` are not used).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    /// A raw model file, as a path relative to `assets/models/`. Used by
    /// scenes from before assets existed and when `asset` is not set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Placements tried per 100 m x 100 m of terrain.
    pub density: f32,
    /// What the world makes of the layer: its source may let the models of
    /// a "tree" layer grow thick in forests and thin in the plains (see
    /// `TerrainSource::scatter_chance`). Empty: untagged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
}

impl ScatterLayerData {
    pub fn asset(asset: AssetRef, density: f32) -> Self {
        Self {
            asset: Some(asset),
            model: String::new(),
            density,
            tag: String::new(),
        }
    }

    pub fn file(model: impl Into<String>, density: f32) -> Self {
        Self {
            asset: None,
            model: model.into(),
            density,
            tag: String::new(),
        }
    }
}

/// Seconds an edited layer has to stay untouched before it is applied, so
/// typing a path or dragging the density does not replant on every change.
pub(super) const APPLY_DELAY: f64 = 0.6;

/// What a layer plants right now: its graph and the models of the graph's
/// outputs.
pub(super) struct Loaded {
    pub(super) graph: Arc<ScatterGraph>,
    /// Per `SpawnMesh` node, in node order: the node, its model and range.
    pub(super) outputs: Vec<LoadedOutput>,
}

pub(super) struct LoadedOutput {
    pub(super) node: NodeId,
    pub(super) model: Rc<ScatterModel>,
    pub(super) range: ScatterRange,
}

pub(super) struct ScatterLayer {
    /// The asset to scatter: a model, or a scatter graph. Takes precedence
    /// over `model`.
    pub(super) asset: Option<AssetRef>,
    /// Raw model file as typed; empty when unused.
    pub(super) model: String,
    pub(super) density: f32,
    pub(super) tag: String,
    /// What is planted right now.
    pub(super) loaded: Option<Loaded>,
    /// Revision of the asset `loaded` was built from.
    loaded_revision: u64,
    pub(super) error: Option<String>,
    /// Countdown until the edited values are applied; `None` when they are.
    pub(super) apply_in: Option<f64>,
}

impl ScatterLayer {
    pub(super) fn new(mut data: ScatterLayerData) -> Self {
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
            tag: data.tag,
            loaded: None,
            loaded_revision: 0,
            error: None,
            apply_in: Some(0.0),
        }
    }

    /// Stable per-model salt, so a model keeps its spots when layers are
    /// reordered.
    pub(super) fn salt(&self) -> u64 {
        match &self.asset {
            Some(asset) => fnv1a(&format!(
                "{}/{}",
                asset.id,
                asset.part.as_deref().unwrap_or("")
            )),
            None => fnv1a(&self.path()),
        }
    }

    /// The salt of the instances of `output`: a model layer's own salt
    /// (its one output), else mixed with the node.
    pub(super) fn output_salt(&self, output: NodeId) -> u64 {
        if self.is_graph() {
            fnv1a(&format!("{}#{}", self.salt(), output.0))
        } else {
            self.salt()
        }
    }

    /// The model path as used for loading and saving: trimmed, with forward
    /// slashes so a scene saved on Windows also loads on Linux.
    pub(super) fn path(&self) -> String {
        self.model.trim().replace('\\', "/")
    }

    /// Neither an asset nor a file: the layer scatters nothing.
    pub(super) fn is_unused(&self) -> bool {
        self.asset.is_none() && self.path().is_empty()
    }

    /// Whether the layer's asset is a scatter graph (rather than a model).
    pub(super) fn is_graph(&self) -> bool {
        let Some(asset) = &self.asset else {
            return false;
        };
        assets::database()
            .get(asset.id)
            .is_some_and(|entry| matches!(entry.meta.kind, AssetKind::ScatterGraph(_)))
    }

    /// What the layer scatters, for messages.
    pub(super) fn label(&self) -> String {
        match &self.asset {
            Some(asset) => assets::database()
                .display_name(asset)
                .unwrap_or_else(|| format!("missing asset {}", asset.id)),
            None => self.path(),
        }
    }

    /// Whether the asset was saved again since it was loaded.
    pub(super) fn is_outdated(&self) -> bool {
        let (Some(asset), Some(_)) = (&self.asset, &self.loaded) else {
            return false;
        };
        assets::database()
            .get(asset.id)
            .is_some_and(|entry| entry.revision != self.loaded_revision)
    }

    pub(super) fn apply(&mut self) {
        self.error = None;
        self.loaded = None;
        if self.is_unused() {
            return;
        }
        let result = if self.is_graph() { self.load_graph() } else { self.load_model() };
        match result {
            Ok(loaded) => self.loaded = Some(loaded),
            Err(e) => {
                log::warn!("terrain scatter: could not load {}: {e}", self.label());
                self.error = Some(e);
            }
        }
    }

    /// A model layer: its model on the rules of `ScatterGraph::from_layer`.
    fn load_model(&mut self) -> Result<Loaded, String> {
        let model = match &self.asset {
            Some(asset) => {
                let (model, revision) = ScatterModel::load_asset(asset)?;
                self.loaded_revision = revision;
                model
            }
            None => ScatterModel::load(&self.path())?,
        };
        let data = ScatterLayerData {
            asset: self.asset.clone(),
            model: self.path(),
            density: self.density,
            tag: self.tag.clone(),
        };
        let graph = Arc::new(ScatterGraph::from_layer(&data, self.salt()));
        let outputs = graph
            .outputs()
            .map(|(node, spawn)| LoadedOutput { node, model: model.clone(), range: spawn.range })
            .collect();
        Ok(Loaded { graph, outputs })
    }

    /// A graph layer: the graph file and the models of its outputs.
    fn load_graph(&mut self) -> Result<Loaded, String> {
        let asset = self.asset.clone().expect("a graph layer has an asset");
        let (source, revision) = {
            let database = assets::database();
            let entry = database
                .get(asset.id)
                .ok_or_else(|| format!("asset {} does not exist", asset.id))?;
            (entry.source_path(), entry.revision)
        };
        let graph = ScatterGraph::load(&source)?;
        if let Some(problem) = graph.problems().first() {
            return Err(problem.to_string());
        }
        self.loaded_revision = revision;
        let mut outputs = Vec::new();
        for (node, spawn) in graph.outputs() {
            let model = match (&spawn.asset, spawn.model.trim()) {
                (Some(asset), _) => ScatterModel::load_asset(asset)?.0,
                (None, "") => continue,
                (None, file) => ScatterModel::load(&file.replace('\\', "/"))?,
            };
            outputs.push(LoadedOutput { node, model, range: spawn.range });
        }
        Ok(Loaded { graph: Arc::new(graph), outputs })
    }
}
