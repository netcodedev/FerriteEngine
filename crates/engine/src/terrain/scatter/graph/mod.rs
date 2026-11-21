//! Scatter graphs: rules, put together as a graph of nodes, that turn a
//! piece of ground into placements of models (grass, stones, bushes,
//! trees). See docs/scatter-graphs.md.
//!
//! A graph is pure data (a `*.scatter.ron` asset): nodes with stable ids
//! and wires from an output pin to an input pin. Points flow along the
//! wires: sources make them, filters drop them or lower their density,
//! transforms turn, scale and move them, and `SpawnMesh` outputs collect
//! them as placements. `evaluate` (see `eval`) works a graph out for a
//! rectangle of ground; the same graph, seed and ground give the same
//! placements every time and on any thread.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::core::assets::AssetRef;

use super::ScatterLayerData;

mod edit;
mod eval;
#[cfg(test)]
mod tests;

pub use edit::GraphHistory;
pub use eval::*;

/// What a scatter graph file's name ends with.
pub const SCATTER_GRAPH_SUFFIX: &str = ".scatter.ron";

/// Whether `path` names a scatter graph file.
pub fn is_scatter_graph_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().ends_with(SCATTER_GRAPH_SUFFIX))
}

/// The graph's name: its file name without `.scatter.ron`.
pub fn scatter_graph_name(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let cut = name.len().saturating_sub(SCATTER_GRAPH_SUFFIX.len());
    if name.to_lowercase().ends_with(SCATTER_GRAPH_SUFFIX) {
        name[..cut].to_string()
    } else {
        name.to_string()
    }
}

/// A node's id, stable for the life of the graph (wires refer to it).
/// Written as a plain number.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// One pin of a node: `pin` counts the node's inputs (or outputs) from 0.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PinRef {
    pub node: NodeId,
    pub pin: u8,
}

/// Points flow from `from` (an output pin) to `to` (an input pin).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wire {
    pub from: PinRef,
    pub to: PinRef,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Where the node editor shows it (editor-only).
    #[serde(default)]
    pub position: [f32; 2],
}

/// How a `Split` node divides its points.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitBy {
    /// A point's density against the threshold.
    Density,
    /// A random number of the point's own (0..1) against the threshold:
    /// that share of the points goes below.
    Random,
}

/// How far from the camera an output's models are planted.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ScatterRange {
    /// Over every loaded chunk, with colliders, and pictured beyond them
    /// (trees, large rocks).
    Landscape,
    /// Only within `distance` metres of the camera, no colliders, no far
    /// pictures (grass, flowers, pebbles).
    Detail { distance: f32 },
}

/// An output: the model planted at the points that reach it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpawnMesh {
    /// The model asset: all its variants, or the one named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    /// A raw model file below `assets/models/`, when there is no asset.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    pub range: ScatterRange,
    /// Whether the models get colliders (landscape range only).
    #[serde(default)]
    pub collision: bool,
    #[serde(default = "yes")]
    pub shadows: bool,
    /// How much the models sway in the wind, 0..1.
    #[serde(default)]
    pub wind: f32,
    /// How much the models take on the colour of the ground they stand on
    /// (`TerrainSource::scatter_tint`), 0..1.
    #[serde(default)]
    pub tint: f32,
    /// How far the shading normal bends towards the sky, 0..1: grass lit
    /// like the ground rather than as upright cards.
    #[serde(default)]
    pub normals_up: f32,
}

fn yes() -> bool {
    true
}

/// What a node does, with its settings. Angles are in degrees, lengths
/// in metres.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Evenly spread points: one per `spacing` square of a grid laid over
    /// the whole world, moved within it by up to `jitter` (0..1) of the
    /// spacing.
    SurfacePoints { spacing: f32, jitter: f32, seed: u64 },
    /// Random points, `per_hectare` tried per 100 m x 100 m, on the very
    /// spots the terrain's scatter layers use (see `chunk::scatter_spots`),
    /// each with its own turn about the up axis.
    RandomPoints { per_hectare: f32, seed: u64 },
    /// Keeps points where the ground's slope is between `min` and `max`,
    /// fading out over `soft` beyond them.
    Slope { min: f32, max: f32, soft: f32 },
    /// Keeps points whose height is between `min` and `max`, fading out
    /// over `soft` beyond them.
    Height { min: f32, max: f32, soft: f32 },
    /// Multiplies the density by how likely the world lets something
    /// tagged `tag` grow there (`TerrainSource::scatter_chance`: water,
    /// roads, towns, biomes).
    WorldChance { tag: String },
    /// Multiplies the density by a noise mask: patches `wavelength` metres
    /// across, the share above `threshold` (0..1) kept, with an edge
    /// `softness` wide.
    Noise { wavelength: f32, threshold: f32, softness: f32, seed: u64 },
    /// Keeps a point if its random number is under its density: turns soft
    /// values into "there or not".
    DensityCut,
    /// Drops the points (first input) within `radius` of any point of the
    /// second input.
    KeepAwayFrom { radius: f32 },
    /// Turns the points: a random yaw between `yaw_min` and `yaw_max`; up
    /// leaning with the ground by `align` (0 upright .. 1 along the
    /// slope), plus up to `tilt` at random.
    Rotate { yaw_min: f32, yaw_max: f32, align: f32, tilt: f32 },
    /// Scales the points: uniformly by `min..max` at random, upwards by a
    /// further `height_min..height_max`; `by_density` (0..1) shrinks
    /// points of low density.
    Scale { min: f32, max: f32, height_min: f32, height_max: f32, by_density: f32 },
    /// Moves the points up by `min..max` at random (below 0: into the
    /// ground).
    Offset { min: f32, max: f32 },
    /// The points of every wire into its input.
    Merge,
    /// Points at or above `threshold` to the first output, the others to
    /// the second.
    Split { by: SplitBy, threshold: f32 },
    /// An output.
    SpawnMesh(SpawnMesh),
}

/// An input pin of a node kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputPin {
    pub name: &'static str,
    /// Takes any number of wires (otherwise at most one).
    pub multi: bool,
}

const POINTS_IN: &[InputPin] = &[InputPin { name: "points", multi: false }];
const POINTS_OUT: &[&str] = &["points"];

impl NodeKind {
    /// Its name in lists and the node editor.
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::SurfacePoints { .. } => "Surface points",
            NodeKind::RandomPoints { .. } => "Random points",
            NodeKind::Slope { .. } => "Slope",
            NodeKind::Height { .. } => "Height",
            NodeKind::WorldChance { .. } => "World chance",
            NodeKind::Noise { .. } => "Noise",
            NodeKind::DensityCut => "Density cut",
            NodeKind::KeepAwayFrom { .. } => "Keep away from",
            NodeKind::Rotate { .. } => "Rotate",
            NodeKind::Scale { .. } => "Scale",
            NodeKind::Offset { .. } => "Offset",
            NodeKind::Merge => "Merge",
            NodeKind::Split { .. } => "Split",
            NodeKind::SpawnMesh(_) => "Spawn mesh",
        }
    }

    pub fn inputs(&self) -> &'static [InputPin] {
        match self {
            NodeKind::SurfacePoints { .. } | NodeKind::RandomPoints { .. } => &[],
            NodeKind::KeepAwayFrom { .. } => &[
                InputPin { name: "points", multi: false },
                InputPin { name: "away from", multi: false },
            ],
            NodeKind::Merge => &[InputPin { name: "points", multi: true }],
            _ => POINTS_IN,
        }
    }

    pub fn outputs(&self) -> &'static [&'static str] {
        match self {
            NodeKind::SpawnMesh(_) => &[],
            NodeKind::Split { .. } => &["above", "below"],
            _ => POINTS_OUT,
        }
    }
}

/// Something wrong with a graph's structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphProblem {
    DuplicateNode(NodeId),
    /// A wire from or to a node that is not there, or a pin it does not
    /// have.
    BadWire(Wire),
    /// More than one wire into an input that takes one.
    InputWiredTwice(PinRef),
    /// Points would flow in a circle through these nodes.
    Cycle(Vec<NodeId>),
}

impl std::fmt::Display for GraphProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphProblem::DuplicateNode(id) => write!(f, "two nodes have the id {}", id.0),
            GraphProblem::BadWire(w) => write!(
                f,
                "a wire from node {} pin {} to node {} pin {} leads nowhere",
                w.from.node.0, w.from.pin, w.to.node.0, w.to.pin
            ),
            GraphProblem::InputWiredTwice(p) => {
                write!(f, "input {} of node {} has more than one wire", p.pin, p.node.0)
            }
            GraphProblem::Cycle(ids) => {
                let ids: Vec<String> = ids.iter().map(|id| id.0.to_string()).collect();
                write!(f, "points flow in a circle through nodes {}", ids.join(", "))
            }
        }
    }
}

/// A scatter graph, as stored in a `*.scatter.ron` file.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ScatterGraph {
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub wires: Vec<Wire>,
}

impl ScatterGraph {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_ron(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn from_ron(text: &str) -> Result<Self, String> {
        ron::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_ron(&self) -> String {
        let config = ron::ser::PrettyConfig::new().struct_names(false);
        ron::ser::to_string_pretty(self, config).expect("a scatter graph always serialises")
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        fs::write(path, self.to_ron()).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn node(&self, id: NodeId) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// An id no node has yet.
    pub fn next_id(&self) -> NodeId {
        NodeId(self.nodes.iter().map(|n| n.id.0 + 1).max().unwrap_or(0))
    }

    /// Add a node; returns its id.
    pub fn add(&mut self, kind: NodeKind) -> NodeId {
        let id = self.next_id();
        self.nodes.push(GraphNode { id, kind, position: [0.0, 0.0] });
        id
    }

    /// Wire output `from_pin` of `from` to input `to_pin` of `to`.
    pub fn connect(&mut self, from: NodeId, from_pin: u8, to: NodeId, to_pin: u8) {
        self.wires.push(Wire {
            from: PinRef { node: from, pin: from_pin },
            to: PinRef { node: to, pin: to_pin },
        });
    }

    /// A chain: each node's first output into the next one's first input.
    pub fn chain(kinds: impl IntoIterator<Item = NodeKind>) -> Self {
        let mut graph = Self::default();
        let mut last: Option<NodeId> = None;
        for (index, kind) in kinds.into_iter().enumerate() {
            let id = graph.add(kind);
            if let Some(node) = graph.nodes.iter_mut().find(|n| n.id == id) {
                node.position = [index as f32 * 220.0, 0.0];
            }
            if let Some(last) = last {
                graph.connect(last, 0, id, 0);
            }
            last = Some(id);
        }
        graph
    }

    /// The graph a scatter layer of old stands for: its model on the very
    /// spots the layer planted it (`salt`: the layer's, see
    /// `ScatterLayer::salt`).
    pub fn from_layer(layer: &ScatterLayerData, salt: u64) -> Self {
        Self::chain([
            NodeKind::RandomPoints { per_hectare: layer.density.max(0.0), seed: salt },
            NodeKind::WorldChance { tag: layer.tag.clone() },
            NodeKind::DensityCut,
            NodeKind::SpawnMesh(SpawnMesh {
                asset: layer.asset.clone(),
                model: layer.model.clone(),
                range: ScatterRange::Landscape,
                collision: true,
                shadows: true,
                wind: 0.0,
                tint: 0.0,
                normals_up: 0.0,
            }),
        ])
    }

    /// The outputs, in node order.
    pub fn outputs(&self) -> impl Iterator<Item = (NodeId, &SpawnMesh)> {
        self.nodes.iter().filter_map(|n| match &n.kind {
            NodeKind::SpawnMesh(spawn) => Some((n.id, spawn)),
            _ => None,
        })
    }

    /// The wires into input `pin` of `node`.
    pub fn wires_into(&self, node: NodeId, pin: u8) -> impl Iterator<Item = &Wire> {
        self.wires.iter().filter(move |w| w.to.node == node && w.to.pin == pin)
    }

    /// Everything wrong with the graph's structure; empty if it can be
    /// evaluated.
    pub fn problems(&self) -> Vec<GraphProblem> {
        let mut problems = Vec::new();
        let mut kinds: HashMap<NodeId, &NodeKind> = HashMap::new();
        for node in &self.nodes {
            if kinds.insert(node.id, &node.kind).is_some() {
                problems.push(GraphProblem::DuplicateNode(node.id));
            }
        }
        let mut wired: HashSet<PinRef> = HashSet::new();
        for wire in &self.wires {
            let from_ok = kinds
                .get(&wire.from.node)
                .is_some_and(|k| (wire.from.pin as usize) < k.outputs().len());
            let to_pin = kinds
                .get(&wire.to.node)
                .and_then(|k| k.inputs().get(wire.to.pin as usize));
            match (from_ok, to_pin) {
                (true, Some(pin)) => {
                    if !wired.insert(wire.to) && !pin.multi {
                        problems.push(GraphProblem::InputWiredTwice(wire.to));
                    }
                }
                _ => problems.push(GraphProblem::BadWire(*wire)),
            }
        }
        if let Some(cycle) = self.find_cycle() {
            problems.push(GraphProblem::Cycle(cycle));
        }
        problems.dedup();
        problems
    }

    /// Nodes that feed themselves, if any.
    fn find_cycle(&self) -> Option<Vec<NodeId>> {
        // Depth-first along the wires, upstream: 1 = on the path, 2 = done.
        fn visit(
            graph: &ScatterGraph,
            node: NodeId,
            state: &mut HashMap<NodeId, u8>,
            path: &mut Vec<NodeId>,
        ) -> Option<Vec<NodeId>> {
            match state.get(&node) {
                Some(2) => return None,
                Some(1) => {
                    let start = path.iter().position(|&n| n == node).unwrap_or(0);
                    return Some(path[start..].to_vec());
                }
                _ => {}
            }
            state.insert(node, 1);
            path.push(node);
            for wire in graph.wires.iter().filter(|w| w.to.node == node) {
                if let Some(cycle) = visit(graph, wire.from.node, state, path) {
                    return Some(cycle);
                }
            }
            path.pop();
            state.insert(node, 2);
            None
        }
        let mut state = HashMap::new();
        for node in &self.nodes {
            if let Some(cycle) = visit(self, node.id, &mut state, &mut Vec::new()) {
                return Some(cycle);
            }
        }
        None
    }
}
