//! Scatter layers, as stored and as loaded: each a scatter graph asset
//! (the only way models are scattered over the terrain).

use std::{rc::Rc, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::core::assets::{self, AssetKind, AssetRef};

use super::graph::{NodeId, ScatterGraph, ScatterRange, SpawnMesh};

use super::*;

/// One scatter layer as stored in a scene file: the scatter graph it plants.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScatterLayerData {
    pub asset: AssetRef,
}

impl ScatterLayerData {
    pub fn graph(asset: AssetRef) -> Self {
        Self { asset }
    }
}

/// Seconds an edited layer has to stay untouched before it is applied, so
/// edits in quick succession do not replant on every change.
pub(super) const APPLY_DELAY: f64 = 0.6;

/// What a layer plants right now: its graph and the models of the graph's
/// outputs.
pub(super) struct Loaded {
    /// The graph as the world plants it (see `ScatterGraph::with_world_seed`).
    pub(super) graph: Arc<ScatterGraph>,
    /// A hash of the graph's file text (see `ScatterSpec::graph_key`).
    pub(super) graph_key: u64,
    /// Per `SpawnMesh` node, in node order: the node, its model and range.
    pub(super) outputs: Vec<LoadedOutput>,
}

pub(super) struct LoadedOutput {
    pub(super) node: NodeId,
    pub(super) model: Rc<ScatterModel>,
    pub(super) range: ScatterRange,
    pub(super) spawn: SpawnMesh,
}

pub(super) struct ScatterLayer {
    /// The scatter graph asset.
    pub(super) asset: AssetRef,
    /// What is planted right now.
    pub(super) loaded: Option<Loaded>,
    /// Revision of the asset `loaded` was built from.
    loaded_revision: u64,
    pub(super) error: Option<String>,
    /// Countdown until the layer is (re)loaded; `None` when it is.
    pub(super) apply_in: Option<f64>,
}

impl ScatterLayer {
    pub(super) fn new(data: ScatterLayerData) -> Self {
        Self { asset: data.asset, loaded: None, loaded_revision: 0, error: None, apply_in: Some(0.0) }
    }

    /// Stable per-graph salt, so a graph keeps its instances' identity when
    /// layers are reordered.
    pub(super) fn salt(&self) -> u64 {
        fnv1a(&format!("{}/{}", self.asset.id, self.asset.part.as_deref().unwrap_or("")))
    }

    /// The salt of the instances of `output`.
    pub(super) fn output_salt(&self, output: NodeId) -> u64 {
        fnv1a(&format!("{}#{}", self.salt(), output.0))
    }

    /// What the layer scatters, for messages.
    pub(super) fn label(&self) -> String {
        assets::database()
            .display_name(&self.asset)
            .unwrap_or_else(|| format!("missing asset {}", self.asset.id))
    }

    /// Whether the asset was saved again since it was loaded.
    pub(super) fn is_outdated(&self) -> bool {
        if self.loaded.is_none() {
            return false;
        }
        assets::database()
            .get(self.asset.id)
            .is_some_and(|entry| entry.revision != self.loaded_revision)
    }

    /// Load what the layer plants. `preview`: a graph to plant in place of
    /// the asset's file (one being edited, unsaved); `world_seed`: the
    /// world's, for nodes that take it.
    pub(super) fn apply(&mut self, preview: Option<&Arc<ScatterGraph>>, world_seed: u64) {
        self.error = None;
        self.loaded = None;
        match self.load(preview, world_seed) {
            Ok(loaded) => self.loaded = Some(loaded),
            Err(e) => {
                log::warn!("terrain scatter: could not load {}: {e}", self.label());
                self.error = Some(e);
            }
        }
    }

    /// The graph and the models of its outputs.
    fn load(&mut self, preview: Option<&Arc<ScatterGraph>>, world_seed: u64) -> Result<Loaded, String> {
        let (source, revision) = {
            let database = assets::database();
            let entry = database
                .get(self.asset.id)
                .ok_or_else(|| format!("asset {} does not exist", self.asset.id))?;
            if !matches!(entry.meta.kind, AssetKind::ScatterGraph(_)) {
                return Err(format!("'{}' is not a scatter graph", entry.meta.name));
            }
            (entry.source_path(), entry.revision)
        };
        let graph = match preview {
            Some(graph) => (**graph).clone(),
            None => ScatterGraph::load(&source)?,
        };
        if let Some(problem) = graph.problems().first() {
            return Err(problem.to_string());
        }
        let graph = graph.with_world_seed(world_seed);
        self.loaded_revision = revision;
        let mut outputs = Vec::new();
        for (node, spawn) in graph.outputs() {
            let model = match (&spawn.asset, spawn.model.trim()) {
                (Some(asset), _) => ScatterModel::load_asset(asset)?.0,
                (None, "") => continue,
                (None, file) => ScatterModel::load(&file.replace('\\', "/"))?,
            };
            outputs.push(LoadedOutput { node, model, range: spawn.range, spawn: spawn.clone() });
        }
        Ok(Loaded { graph_key: fnv1a(&graph.to_ron()), graph: Arc::new(graph), outputs })
    }
}
