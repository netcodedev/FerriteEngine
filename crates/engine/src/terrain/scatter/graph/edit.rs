//! Editing a scatter graph: what the node editor and the MCP tools do to
//! one. Every operation leaves a graph `problems` finds nothing wrong with
//! (if it had none): wires only between pins that exist, one wire into an
//! input that takes one, no circles. `GraphHistory` undoes and redoes.

use std::collections::{HashMap, HashSet};

use crate::core::entity::component::{Property, PropertyValue};

use super::{GraphNode, NodeId, NodeKind, PinRef, ScatterGraph, ScatterRange, SpawnMesh, SplitBy, Wire};

impl ScatterGraph {
    /// Add a node at `position` (node editor units); returns its id.
    pub fn add_node(&mut self, kind: NodeKind, position: [f32; 2]) -> NodeId {
        let id = self.next_id();
        self.nodes.push(GraphNode { id, kind, position });
        id
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut GraphNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Remove nodes and every wire from or to them.
    pub fn remove_nodes(&mut self, ids: &[NodeId]) {
        self.nodes.retain(|n| !ids.contains(&n.id));
        self.wires.retain(|w| !ids.contains(&w.from.node) && !ids.contains(&w.to.node));
    }

    /// Move nodes by `delta`.
    pub fn move_nodes(&mut self, ids: &[NodeId], delta: [f32; 2]) {
        for node in self.nodes.iter_mut().filter(|n| ids.contains(&n.id)) {
            node.position = [node.position[0] + delta[0], node.position[1] + delta[1]];
        }
    }

    /// Whether output `from` may be wired to input `to`, and why not.
    pub fn can_wire(&self, from: PinRef, to: PinRef) -> Result<(), String> {
        let source = self.node(from.node).ok_or("no such node")?;
        let target = self.node(to.node).ok_or("no such node")?;
        if from.pin as usize >= source.kind.outputs().len() {
            return Err(format!("{} has no output {}", source.kind.label(), from.pin));
        }
        if to.pin as usize >= target.kind.inputs().len() {
            return Err(format!("{} has no input {}", target.kind.label(), to.pin));
        }
        if from.node == to.node || self.feeds(to.node, from.node) {
            return Err("points would flow in a circle".into());
        }
        Ok(())
    }

    /// Whether points of `upstream` reach `downstream` along the wires.
    pub fn feeds(&self, upstream: NodeId, downstream: NodeId) -> bool {
        let mut seen = HashSet::new();
        let mut open = vec![upstream];
        while let Some(node) = open.pop() {
            if node == downstream {
                return true;
            }
            if seen.insert(node) {
                open.extend(self.wires.iter().filter(|w| w.from.node == node).map(|w| w.to.node));
            }
        }
        false
    }

    /// Wire output `from` to input `to`. A wire already in an input that
    /// takes one is replaced; the same wire twice is one.
    pub fn wire(&mut self, from: PinRef, to: PinRef) -> Result<(), String> {
        self.can_wire(from, to)?;
        let multi = self
            .node(to.node)
            .and_then(|n| n.kind.inputs().get(to.pin as usize))
            .is_some_and(|pin| pin.multi);
        if multi {
            self.wires.retain(|w| !(w.from == from && w.to == to));
        } else {
            self.wires.retain(|w| w.to != to);
        }
        self.wires.push(Wire { from, to });
        Ok(())
    }

    /// Take a wire out.
    pub fn unwire(&mut self, wire: Wire) {
        self.wires.retain(|w| *w != wire);
    }

    /// The nodes `ids` and the wires between them, for the clipboard.
    pub fn copy_nodes(&self, ids: &[NodeId]) -> ScatterGraph {
        ScatterGraph {
            nodes: self.nodes.iter().filter(|n| ids.contains(&n.id)).cloned().collect(),
            wires: self
                .wires
                .iter()
                .filter(|w| ids.contains(&w.from.node) && ids.contains(&w.to.node))
                .copied()
                .collect(),
        }
    }

    /// Add the nodes and wires of `part` with new ids, moved by `offset`;
    /// returns the new ids.
    pub fn paste(&mut self, part: &ScatterGraph, offset: [f32; 2]) -> Vec<NodeId> {
        let mut new_ids: HashMap<NodeId, NodeId> = HashMap::new();
        for node in &part.nodes {
            let position = [node.position[0] + offset[0], node.position[1] + offset[1]];
            let id = self.add_node(node.kind.clone(), position);
            new_ids.insert(node.id, id);
        }
        for wire in &part.wires {
            if let (Some(&from), Some(&to)) = (new_ids.get(&wire.from.node), new_ids.get(&wire.to.node)) {
                self.wires.push(Wire {
                    from: PinRef { node: from, pin: wire.from.pin },
                    to: PinRef { node: to, pin: wire.to.pin },
                });
            }
        }
        part.nodes.iter().filter_map(|n| new_ids.get(&n.id).copied()).collect()
    }
}

impl NodeKind {
    /// One of each kind with its settings to start from, in the order the
    /// node editor's "add" menu lists them.
    pub fn catalog() -> Vec<NodeKind> {
        vec![
            NodeKind::SurfacePoints { spacing: 1.0, jitter: 1.0, seed: 1 },
            NodeKind::RandomPoints { per_hectare: 20.0, seed: 1 },
            NodeKind::Slope { min: 0.0, max: 30.0, soft: 5.0 },
            NodeKind::Height { min: 0.0, max: 500.0, soft: 5.0 },
            NodeKind::WorldChance { tag: String::new() },
            NodeKind::Noise { wavelength: 25.0, threshold: 0.5, softness: 0.2, seed: 1 },
            NodeKind::DensityCut { multiplier: 1.0 },
            NodeKind::KeepAwayFrom { radius: 2.0 },
            NodeKind::Rotate { yaw_min: 0.0, yaw_max: 360.0, align: 0.0, tilt: 0.0 },
            NodeKind::Scale { min: 1.0, max: 1.0, height_min: 1.0, height_max: 1.0, by_density: 0.0 },
            NodeKind::Offset { min: 0.0, max: 0.0 },
            NodeKind::Merge,
            NodeKind::Split { by: SplitBy::Random, threshold: 0.5 },
            NodeKind::SpawnMesh(SpawnMesh {
                asset: None,
                model: String::new(),
                range: ScatterRange::Landscape,
                collision: false,
                shadows: true,
                wind: 0.0,
                tint: 0.0,
                normals_up: 0.0,
            }),
        ]
    }

    /// The node's settings, for the inspector. A `Spawn mesh`'s model is
    /// shown, not typed: it is picked from the assets.
    pub fn properties(&self) -> Vec<Property> {
        let seed = |seed: &u64| Property::int("seed", *seed as i64);
        match self {
            NodeKind::SurfacePoints { spacing, jitter, seed: s } => vec![
                Property::ranged("spacing", *spacing, 0.05, 50.0),
                Property::ranged("jitter", *jitter, 0.0, 1.0),
                seed(s),
            ],
            NodeKind::RandomPoints { per_hectare, seed: s } => {
                vec![Property::ranged("per hectare", *per_hectare, 0.0, 5000.0), seed(s)]
            }
            NodeKind::Slope { min, max, soft } => vec![
                Property::ranged("min", *min, 0.0, 90.0),
                Property::ranged("max", *max, 0.0, 90.0),
                Property::ranged("soft", *soft, 0.0, 45.0),
            ],
            NodeKind::Height { min, max, soft } => vec![
                Property::float("min", *min),
                Property::float("max", *max),
                Property::ranged("soft", *soft, 0.0, 100.0),
            ],
            NodeKind::WorldChance { tag } => vec![Property::text("tag", tag.clone())],
            NodeKind::Noise { wavelength, threshold, softness, seed: s } => vec![
                Property::ranged("wavelength", *wavelength, 0.5, 1000.0),
                Property::ranged("threshold", *threshold, 0.0, 1.0),
                Property::ranged("softness", *softness, 0.0, 1.0),
                seed(s),
            ],
            NodeKind::DensityCut { multiplier } => vec![Property::ranged("multiplier", *multiplier, 0.0, 4.0)],
            NodeKind::Merge => Vec::new(),
            NodeKind::KeepAwayFrom { radius } => vec![Property::ranged("radius", *radius, 0.0, 50.0)],
            NodeKind::Rotate { yaw_min, yaw_max, align, tilt } => vec![
                Property::ranged("yaw min", *yaw_min, 0.0, 360.0),
                Property::ranged("yaw max", *yaw_max, 0.0, 360.0),
                Property::ranged("align", *align, 0.0, 1.0),
                Property::ranged("tilt", *tilt, 0.0, 90.0),
            ],
            NodeKind::Scale { min, max, height_min, height_max, by_density } => vec![
                Property::ranged("min", *min, 0.01, 10.0),
                Property::ranged("max", *max, 0.01, 10.0),
                Property::ranged("height min", *height_min, 0.1, 5.0),
                Property::ranged("height max", *height_max, 0.1, 5.0),
                Property::ranged("by density", *by_density, 0.0, 1.0),
            ],
            NodeKind::Offset { min, max } => vec![Property::ranged("min", *min, -10.0, 10.0), Property::ranged("max", *max, -10.0, 10.0)],
            NodeKind::Split { by, threshold } => vec![
                Property::choice(
                    "by",
                    match by {
                        SplitBy::Density => "density",
                        SplitBy::Random => "random",
                    }
                    .to_string(),
                    vec!["density".into(), "random".into()],
                ),
                Property::ranged("threshold", *threshold, 0.0, 1.0),
            ],
            NodeKind::SpawnMesh(spawn) => {
                let model = match (&spawn.asset, spawn.model.trim()) {
                    (Some(asset), _) => crate::core::assets::database()
                        .display_name(asset)
                        .unwrap_or_else(|| format!("missing asset {}", asset.id)),
                    (None, "") => "(none)".to_string(),
                    (None, file) => file.to_string(),
                };
                let (range, distance) = match spawn.range {
                    ScatterRange::Landscape => ("landscape", None),
                    ScatterRange::Detail { distance } => ("detail", Some(distance)),
                };
                let mut properties = vec![
                    Property::text("model", model).read_only(),
                    Property::choice("range", range.to_string(), vec!["landscape".into(), "detail".into()]),
                ];
                if let Some(distance) = distance {
                    properties.push(Property::ranged("distance", distance, 5.0, 300.0));
                }
                properties.extend([
                    Property::bool("collision", spawn.collision),
                    Property::bool("shadows", spawn.shadows),
                    Property::ranged("wind", spawn.wind, 0.0, 1.0),
                    Property::ranged("tint", spawn.tint, 0.0, 1.0),
                    Property::ranged("normals up", spawn.normals_up, 0.0, 1.0),
                ]);
                properties
            }
        }
    }

    /// Set one of `properties`; returns whether `name` was one of them
    /// (and the value of a fitting kind).
    pub fn set_property(&mut self, name: &str, value: &PropertyValue) -> bool {
        let float = value.as_f32();
        let set = |field: &mut f32| match float {
            Some(v) => {
                *field = v;
                true
            }
            None => false,
        };
        let set_seed = |field: &mut u64| match value {
            PropertyValue::Int(v) => {
                *field = (*v).max(0) as u64;
                true
            }
            _ => false,
        };
        match (self, name) {
            (NodeKind::SurfacePoints { spacing, .. }, "spacing") => set(spacing),
            (NodeKind::SurfacePoints { jitter, .. }, "jitter") => set(jitter),
            (NodeKind::SurfacePoints { seed, .. }, "seed") => set_seed(seed),
            (NodeKind::RandomPoints { per_hectare, .. }, "per hectare") => set(per_hectare),
            (NodeKind::RandomPoints { seed, .. }, "seed") => set_seed(seed),
            (NodeKind::Slope { min, .. } | NodeKind::Height { min, .. }, "min") => set(min),
            (NodeKind::Slope { max, .. } | NodeKind::Height { max, .. }, "max") => set(max),
            (NodeKind::Slope { soft, .. } | NodeKind::Height { soft, .. }, "soft") => set(soft),
            (NodeKind::WorldChance { tag }, "tag") => match value.as_text() {
                Some(text) => {
                    *tag = text.trim().to_string();
                    true
                }
                None => false,
            },
            (NodeKind::Noise { wavelength, .. }, "wavelength") => set(wavelength),
            (NodeKind::Noise { threshold, .. }, "threshold") => set(threshold),
            (NodeKind::Noise { softness, .. }, "softness") => set(softness),
            (NodeKind::Noise { seed, .. }, "seed") => set_seed(seed),
            (NodeKind::KeepAwayFrom { radius }, "radius") => set(radius),
            (NodeKind::DensityCut { multiplier }, "multiplier") => set(multiplier),
            (NodeKind::Rotate { yaw_min, .. }, "yaw min") => set(yaw_min),
            (NodeKind::Rotate { yaw_max, .. }, "yaw max") => set(yaw_max),
            (NodeKind::Rotate { align, .. }, "align") => set(align),
            (NodeKind::Rotate { tilt, .. }, "tilt") => set(tilt),
            (NodeKind::Scale { min, .. } | NodeKind::Offset { min, .. }, "min") => set(min),
            (NodeKind::Scale { max, .. } | NodeKind::Offset { max, .. }, "max") => set(max),
            (NodeKind::Scale { height_min, .. }, "height min") => set(height_min),
            (NodeKind::Scale { height_max, .. }, "height max") => set(height_max),
            (NodeKind::Scale { by_density, .. }, "by density") => set(by_density),
            (NodeKind::Split { by, .. }, "by") => match value.as_text() {
                Some("density") => {
                    *by = SplitBy::Density;
                    true
                }
                Some("random") => {
                    *by = SplitBy::Random;
                    true
                }
                _ => false,
            },
            (NodeKind::Split { threshold, .. }, "threshold") => set(threshold),
            (NodeKind::SpawnMesh(spawn), name) => match (name, value) {
                ("range", PropertyValue::Text(text)) => {
                    spawn.range = match (text.as_str(), spawn.range) {
                        ("landscape", _) => ScatterRange::Landscape,
                        ("detail", ScatterRange::Detail { distance }) => ScatterRange::Detail { distance },
                        ("detail", ScatterRange::Landscape) => ScatterRange::Detail { distance: 60.0 },
                        _ => return false,
                    };
                    true
                }
                ("distance", _) => match (&mut spawn.range, float) {
                    (ScatterRange::Detail { distance }, Some(v)) => {
                        *distance = v.max(1.0);
                        true
                    }
                    _ => false,
                },
                ("collision", PropertyValue::Bool(v)) => {
                    spawn.collision = *v;
                    true
                }
                ("shadows", PropertyValue::Bool(v)) => {
                    spawn.shadows = *v;
                    true
                }
                ("wind", _) => set(&mut spawn.wind),
                ("tint", _) => set(&mut spawn.tint),
                ("normals up", _) => set(&mut spawn.normals_up),
                _ => false,
            },
            _ => false,
        }
    }
}

/// Undo and redo for a graph being edited: whole graphs (they are small).
#[derive(Default)]
pub struct GraphHistory {
    past: Vec<ScatterGraph>,
    future: Vec<ScatterGraph>,
}

/// Steps kept to undo.
const HISTORY_LIMIT: usize = 200;

impl GraphHistory {
    /// Note the graph as it was before an edit.
    pub fn record(&mut self, before: &ScatterGraph) {
        if self.past.last() == Some(before) {
            return;
        }
        self.past.push(before.clone());
        if self.past.len() > HISTORY_LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// Go back one edit; returns whether there was one.
    pub fn undo(&mut self, graph: &mut ScatterGraph) -> bool {
        let Some(before) = self.past.pop() else {
            return false;
        };
        self.future.push(std::mem::replace(graph, before));
        true
    }

    /// Redo an edit undone; returns whether there was one.
    pub fn redo(&mut self, graph: &mut ScatterGraph) -> bool {
        let Some(after) = self.future.pop() else {
            return false;
        };
        self.past.push(std::mem::replace(graph, after));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}
