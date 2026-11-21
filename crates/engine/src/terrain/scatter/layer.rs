//! Scatter layers, as stored and as loaded: each a scatter graph asset
//! (the only way models are scattered over the terrain).

use std::{rc::Rc, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::core::{
    assets::{self, AssetKind, AssetRef},
    graphics::{self, Resolved},
};

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
    /// The graph as the world plants it (see `ScatterGraph::with_world_seed`),
    /// its values still bound to graphics settings.
    source: ScatterGraph,
    /// Planted for the landscape outputs: `source` at High, whatever the
    /// settings (see `ScatterGraph::with_settings`), so changing them never
    /// replants the landscape.
    pub(super) landscape: Planted,
    /// Planted for the detail outputs: `source` with the settings now.
    pub(super) detail: Planted,
    /// Per `SpawnMesh` node, in node order: the node and its model.
    pub(super) outputs: Vec<LoadedOutput>,
}

/// A graph as planted, with its fingerprint.
pub(super) struct Planted {
    pub(super) graph: Arc<ScatterGraph>,
    /// A hash of the graph's text (see `ScatterSpec::graph_key`).
    pub(super) key: u64,
}

impl Planted {
    fn new(source: &ScatterGraph, settings: &Resolved) -> Self {
        let graph = source.with_settings(settings);
        Self { key: fnv1a(&graph.to_ron()), graph: Arc::new(graph) }
    }
}

impl Loaded {
    /// The output `node` as planted: its range and settings, fixed.
    pub(super) fn spawn(&self, node: NodeId) -> Option<(ScatterRange, SpawnMesh, &Planted)> {
        let landscape = self.landscape.graph.outputs().find(|(id, _)| *id == node)?.1;
        let planted = match landscape.range.is_landscape() {
            true => &self.landscape,
            false => &self.detail,
        };
        let spawn = planted.graph.outputs().find(|(id, _)| *id == node)?.1;
        Some((spawn.range.resolved(), spawn.clone(), planted))
    }

    /// Follow new graphics settings: whether what the detail outputs plant
    /// changed.
    pub(super) fn follow_settings(&mut self, settings: &Resolved) -> bool {
        let detail = Planted::new(&self.source, settings);
        if detail.key == self.detail.key {
            return false;
        }
        self.detail = detail;
        true
    }
}

pub(super) struct LoadedOutput {
    pub(super) node: NodeId,
    pub(super) model: Rc<ScatterModel>,
}

/// The settings everything is at High with: what landscape outputs are
/// planted with (see `ScatterGraph::with_settings`).
fn at_high() -> Arc<Resolved> {
    static HIGH: std::sync::LazyLock<Arc<Resolved>> =
        std::sync::LazyLock::new(|| graphics::Graphics::default().resolved());
    HIGH.clone()
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
    /// Its models are read in the background: until they are all there,
    /// what was planted before stays and `apply_in` asks again next frame.
    pub(super) fn apply(&mut self, preview: Option<&Arc<ScatterGraph>>, world_seed: u64) {
        match self.load(preview, world_seed) {
            Ok(Some(loaded)) => {
                self.error = None;
                self.loaded = Some(loaded);
            }
            Ok(None) => self.apply_in = Some(0.0),
            Err(e) => {
                log::warn!("terrain scatter: could not load {}: {e}", self.label());
                self.error = Some(e);
                self.loaded = None;
            }
        }
    }

    /// The graph and the models of its outputs.
    /// `None` while models are still being read.
    fn load(&mut self, preview: Option<&Arc<ScatterGraph>>, world_seed: u64) -> Result<Option<Loaded>, String> {
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
        let mut outputs = Vec::new();
        // Every model asked for at once (they are read side by side); the
        // layer is planted once all are there.
        let mut waiting = false;
        for (node, spawn) in graph.outputs() {
            let model = match (&spawn.asset, spawn.model.trim()) {
                (Some(asset), _) => match ScatterModel::load_asset_background(asset)? {
                    Some((model, _)) => model,
                    None => {
                        waiting = true;
                        continue;
                    }
                },
                (None, "") => continue,
                (None, file) => ScatterModel::load(&file.replace('\\', "/"))?,
            };
            outputs.push(LoadedOutput { node, model });
        }
        if waiting {
            return Ok(None);
        }
        self.loaded_revision = revision;
        Ok(Some(Loaded {
            landscape: Planted::new(&graph, &at_high()),
            detail: Planted::new(&graph, &graphics::current()),
            source: graph,
            outputs,
        }))
    }
}
