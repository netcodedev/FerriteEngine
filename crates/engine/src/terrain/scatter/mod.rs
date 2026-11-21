//! Models scattered over the terrain (trees, rocks, grass, ...). The terrain
//! owns a list of scatter layers, each a scatter graph asset (see `graph`);
//! nothing here is required for a terrain to exist — without layers, or
//! while a graph or model is missing, the terrain just stays bare.
//!
//! A model file can hold several variants (e.g. a tree pack), each with
//! discrete LODs. Loaded models are kept in a thread-local cache, so
//! rebuilding the scene (e.g. entering and leaving play mode) does not load
//! them again. Each chunk gets one `ScatterInstances` per layer, which picks
//! a variant per placement (deterministically hashed from its XZ position)
//! and draws it with one instanced draw per (variant, LOD-submesh)
//! bucket it actually contains.

use std::collections::HashSet;

use cgmath::Point3;

use std::sync::Arc;

use crate::core::{
    assets::{self, AssetRef},
    entity::component::Property,
    gfx,
};

use super::CHUNK_SIZE_FLOAT;

pub mod detail;
pub mod graph;
pub mod impostor;
mod instances;
mod layer;
mod model;
#[cfg(test)]
mod tests;

pub use instances::*;
pub use layer::*;
pub use model::*;

const MODEL_SCALE: f32 = 1.0;

/// Distance bands for LOD selection. With 4 LODs available (Acer pack), each
/// instance picks tier 0/1/2/3 by ascending distance; models with fewer LODs
/// clamp to their last tier.
const LOD_DIST_NEAR: f32 = 60.0;
const LOD_DIST_MID: f32 = 130.0;
const LOD_DIST_FAR: f32 = 250.0;

/// A model's vertices: position, normal and texture coordinates at
/// locations 0, 1 and 2.
fn vertex_layout() -> gfx::VertexLayout {
    use gfx::VertexFormat::*;
    gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[F32x3, F32x3, F32x2])
}

/// An instance: its transform's columns at locations 3 to 6.
fn instance_layout() -> gfx::VertexLayout {
    gfx::VertexLayout::packed(3, gfx::StepMode::Instance, &[gfx::VertexFormat::F32x4; 4])
}

/// The scatter layers of one terrain.
pub struct Scatter {
    layers: Vec<ScatterLayer>,
    /// Instances that were taken out (felled trees), by layer salt and
    /// position, so chunks loaded again leave them out. Not saved.
    removed: HashSet<(u64, [i32; 3])>,
    /// Graphs planted in place of their asset's file: the ones open in the
    /// editor, unsaved edits and all (see `preview_graph`).
    previews: std::collections::HashMap<assets::AssetId, Arc<graph::ScatterGraph>>,
    /// The world's seed (see `set_world_seed`).
    world_seed: u64,
}

impl Scatter {
    /// Models are loaded on the first `tick`, on the GL thread; a missing
    /// model is reported in the status property and the log, never fatal.
    pub fn new(layers: Vec<ScatterLayerData>) -> Self {
        Self {
            layers: layers.into_iter().map(ScatterLayer::new).collect(),
            removed: HashSet::new(),
            previews: std::collections::HashMap::new(),
            world_seed: 0,
        }
    }

    /// Plant `graph` in place of the scatter graph asset `asset`'s file
    /// (the editor's copy as it is being edited), or the file again with
    /// `None`. The layers using it replant after the usual short delay.
    pub fn preview_graph(&mut self, asset: assets::AssetId, graph: Option<graph::ScatterGraph>) {
        let unchanged = match (&graph, self.previews.get(&asset)) {
            (Some(new), Some(old)) => **old == *new,
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        match graph {
            Some(graph) => {
                self.previews.insert(asset, Arc::new(graph));
            }
            None => {
                self.previews.remove(&asset);
            }
        }
        for layer in &mut self.layers {
            if layer.asset.id == asset {
                layer.apply_in = Some(APPLY_DELAY);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Append a layer planting the scatter graph `asset`; it is loaded on
    /// the next `tick`.
    pub fn add_layer(&mut self, asset: AssetRef) {
        self.layers.push(ScatterLayer::new(ScatterLayerData::graph(asset)));
    }

    /// Point a layer at another scatter graph (loaded on the next `tick`).
    pub fn set_asset(&mut self, index: usize, asset: AssetRef) {
        if let Some(layer) = self.layers.get_mut(index) {
            layer.asset = asset;
            layer.apply_in = Some(0.0);
        }
    }

    /// The scatter graph a layer plants.
    pub fn asset(&self, index: usize) -> Option<&AssetRef> {
        self.layers.get(index).map(|l| &l.asset)
    }

    /// Remove a layer. The terrain clears its instances on the next update.
    pub fn remove_layer(&mut self, index: usize) {
        if index < self.layers.len() {
            self.layers.remove(index);
        }
    }

    /// The world's seed, for graph nodes that take it; a new one reloads
    /// the layers.
    pub fn set_world_seed(&mut self, seed: u64) {
        if seed != self.world_seed {
            self.world_seed = seed;
            for layer in &mut self.layers {
                layer.apply_in = Some(0.0);
            }
        }
    }

    /// The layers, for saving.
    pub fn to_data(&self) -> Vec<ScatterLayerData> {
        self.layers.iter().map(|l| ScatterLayerData::graph(l.asset.clone())).collect()
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
                let preview = self.previews.get(&layer.asset.id);
                layer.apply(preview, self.world_seed);
            }
        }
    }

    /// Leave the instance of the layer with `salt` at `position` out from
    /// now on.
    pub fn mark_removed(&mut self, salt: u64, position: Point3<f32>) {
        self.removed.insert((salt, removal_key(position)));
    }

    /// Whether the instance of the layer with `salt` at `position` was
    /// taken out.
    pub fn is_removed(&self, salt: u64, position: Point3<f32>) -> bool {
        self.removed.contains(&(salt, removal_key(position)))
    }

    /// The instances taken out, by layer salt and spot (`[x, z]` to the
    /// millimetre, see `spot_key`), for the far models (`far_scatter`).
    pub fn removed_spots(&self) -> HashSet<(u64, [i32; 2])> {
        self.removed.iter().map(|&(salt, [x, _, z])| (salt, [x, z])).collect()
    }

    /// How many instances were taken out: changes when one more is.
    pub fn removed_count(&self) -> usize {
        self.removed.len()
    }

    /// What every chunk should carry right now: one entry per landscape
    /// output of every loaded layer.
    pub fn specs(&self) -> Vec<ScatterSpec> {
        self.specs_where(|range| range == graph::ScatterRange::Landscape)
    }

    /// What is planted around the camera: one entry per detail output of
    /// every loaded layer.
    pub fn detail_specs(&self) -> Vec<ScatterSpec> {
        self.specs_where(|range| matches!(range, graph::ScatterRange::Detail { .. }))
    }

    fn specs_where(&self, wanted: impl Fn(graph::ScatterRange) -> bool) -> Vec<ScatterSpec> {
        let mut specs: Vec<ScatterSpec> = Vec::new();
        for layer in &self.layers {
            let Some(loaded) = &layer.loaded else {
                continue;
            };
            for output in loaded.outputs.iter().filter(|o| wanted(o.range)) {
                let spec = ScatterSpec {
                    model: output.model.clone(),
                    graph: loaded.graph.clone(),
                    output: output.node,
                    graph_key: loaded.graph_key,
                    salt: layer.output_salt(output.node),
                    range: output.range,
                    shadows: output.spawn.shadows,
                    wind: output.spawn.wind.clamp(0.0, 1.0),
                    normals_up: output.spawn.normals_up.clamp(0.0, 1.0),
                };
                // The same graph twice would plant the very same; once.
                if !specs.iter().any(|s| s.same_as(&spec)) {
                    specs.push(spec);
                }
            }
        }
        specs
    }

    pub fn properties(&self) -> Vec<Property> {
        let mut properties: Vec<Property> = self
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| Property::text(format!("{GRAPH_PROPERTY}{}", index + 1), layer.label()).read_only())
            .collect();
        properties.push(Property::text("scatter", self.status()).read_only());
        properties
    }

    /// Index of the layer a `graph N` property belongs to.
    pub fn graph_property_layer(name: &str) -> Option<usize> {
        name.strip_prefix(GRAPH_PROPERTY)?.parse::<usize>().ok()?.checked_sub(1)
    }

    fn status(&self) -> String {
        let mut loaded = 0;
        let mut problems = Vec::new();
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.apply_in.is_some() {
                problems.push(format!("graph {}: loading", index + 1));
            } else if let Some(error) = &layer.error {
                problems.push(format!("graph {}: {error}", index + 1));
            } else if layer.loaded.is_some() {
                loaded += 1;
            }
        }
        if problems.is_empty() {
            match loaded {
                0 => "no graphs".into(),
                1 => "1 graph loaded".into(),
                n => format!("{n} graphs loaded"),
            }
        } else {
            format!("{loaded} loaded; {}", problems.join("; "))
        }
    }
}

/// Property name prefix of a layer; the 1-based layer number follows.
const GRAPH_PROPERTY: &str = "graph ";

/// A placement's position to the millimetre: placements are recomputed
/// identically when a chunk loads again.
fn removal_key(position: Point3<f32>) -> [i32; 3] {
    [
        (position.x * 1000.0).round() as i32,
        (position.y * 1000.0).round() as i32,
        (position.z * 1000.0).round() as i32,
    ]
}

/// A spot `(x, z)` as `removed_spots` holds it.
pub fn spot_key(x: f32, z: f32) -> [i32; 2] {
    let [x, _, z] = removal_key(Point3::new(x, 0.0, z));
    [x, z]
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
