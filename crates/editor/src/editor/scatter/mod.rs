//! The scatter graph workspace: one scatter graph asset open for editing,
//! drawn as nodes and wires (`canvas`), the selected node's settings in
//! `inspector`. See docs/scatter-graphs.md, step 6.

pub mod canvas;
pub mod inspector;
pub mod preview;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

use ferrite::{
    core::{
        assets::{self, AssetId, AssetKind},
        scene::Scene,
    },
    terrain::{
        scatter::{
            detail::{cell_rect, CellKey, DETAIL_CELL},
            graph::{evaluate_traced, GraphHistory, NodeId, NodeKind, NodeRole, ScatterGraph, Wire},
        },
        Terrain, TerrainSource,
    },
};

/// What the point counts were worked out for: the rules as they were and
/// the cell under the camera.
#[derive(Clone, PartialEq)]
struct CountedFor {
    rules: u64,
    cell: CellKey,
}

/// The point counts of the nodes on the cell under the camera.
#[derive(Clone, Debug, Default)]
pub enum Counts {
    /// No terrain to count on, or not asked yet.
    #[default]
    None,
    Working,
    Done(HashMap<NodeId, usize>),
    Failed(String),
}

pub struct ScatterEdit {
    pub asset: AssetId,
    pub name: String,
    pub path: PathBuf,
    pub graph: ScatterGraph,
    /// As saved; the graph is unsaved while it differs.
    saved: ScatterGraph,
    history: GraphHistory,
    pub selection: Vec<NodeId>,
    pub selected_wire: Option<Wire>,
    /// Bumped on every change of the graph.
    pub revision: u64,
    /// Bumped when what the graph places may have changed (not when a node
    /// only moved on the canvas).
    rules: u64,
    /// Bumped when nodes come or go or the selection changes; panels
    /// rebuild their widgets then.
    pub structure: u64,
    pub clipboard: Option<ScatterGraph>,
    pub counts: Counts,
    /// Every node's points on the cell counted, per output pin (see
    /// `shown_points`).
    pub trace: Option<(CellKey, Trace)>,
    counted_for: Option<CountedFor>,
    counting: Option<(CountedFor, Receiver<Result<Worked, String>>)>,
    /// The rules last handed to the terrain to plant (see `push_preview`).
    pushed_rules: Option<u64>,
    /// When the file was last written, as this copy knows it.
    file_time: Option<std::time::SystemTime>,
    /// The file changed on disk while this copy had unsaved edits: saving
    /// would overwrite that change.
    pub changed_on_disk: bool,
}

fn file_time(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Per node, per output pin: where its points stand and their density,
/// `[x, y, z, density]`.
pub type Trace = HashMap<NodeId, Vec<Vec<[f32; 4]>>>;

/// What the counting thread worked out: per node its count and its points.
type Worked = (HashMap<NodeId, usize>, Trace);

/// Points to show in the preview, of one colour.
pub struct PointGroup {
    pub label: String,
    pub color: (f32, f32, f32),
    pub points: Vec<[f32; 3]>,
}

/// Colours of groups that are just one of several (outputs, inputs).
pub const PALETTE: [(f32, f32, f32); 6] = [
    (0.45, 0.85, 0.35),
    (0.95, 0.70, 0.25),
    (0.40, 0.70, 1.00),
    (0.95, 0.45, 0.55),
    (0.75, 0.55, 0.95),
    (0.90, 0.90, 0.90),
];
const KEPT: (f32, f32, f32) = (0.35, 0.90, 0.35);
const DROPPED: (f32, f32, f32) = (0.95, 0.30, 0.25);
const AWAY_FROM: (f32, f32, f32) = (0.35, 0.60, 1.00);

/// How much a fading filter let through, in steps: (above, up to, colour);
/// the first is what it dropped.
const FADE: [(Option<f32>, f32, (f32, f32, f32)); 5] = [
    (None, 0.0, DROPPED),
    (Some(0.0), 0.25, (0.95, 0.50, 0.25)),
    (Some(0.25), 0.5, (0.95, 0.75, 0.25)),
    (Some(0.5), 0.75, (0.85, 0.90, 0.30)),
    (Some(0.75), 1.0, KEPT),
];

impl ScatterEdit {
    /// Open the scatter graph asset `id`.
    pub fn open(id: AssetId) -> Result<Self, String> {
        let (name, path) = {
            let database = assets::database();
            let entry = database.get(id).ok_or_else(|| format!("asset {id} does not exist"))?;
            if !matches!(entry.meta.kind, AssetKind::ScatterGraph(_)) {
                return Err(format!("'{}' is not a scatter graph", entry.meta.name));
            }
            (entry.meta.name.clone(), entry.source_path())
        };
        let graph = ScatterGraph::load(&path)?;
        let written = file_time(&path);
        Ok(Self {
            asset: id,
            name,
            path,
            saved: graph.clone(),
            file_time: written,
            changed_on_disk: false,
            graph,
            history: GraphHistory::default(),
            selection: Vec::new(),
            selected_wire: None,
            revision: 0,
            rules: 0,
            structure: 0,
            clipboard: None,
            counts: Counts::None,
            trace: None,
            counted_for: None,
            counting: None,
            pushed_rules: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.graph != self.saved
    }

    /// Note the graph as it is before an edit (one undo step).
    pub fn record(&mut self) {
        self.history.record(&self.graph);
    }

    /// Nodes moved on the canvas (after `record`).
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// A node's settings changed (after `record`).
    pub fn touch_rules(&mut self) {
        self.revision += 1;
        self.rules += 1;
    }

    /// Nodes or wires came or went.
    pub fn touch_structure(&mut self) {
        self.revision += 1;
        self.rules += 1;
        self.structure += 1;
    }

    /// Change the graph as one undo step.
    pub fn edit(&mut self, change: impl FnOnce(&mut ScatterGraph)) {
        self.record();
        change(&mut self.graph);
        self.forget_missing();
        self.touch_structure();
    }

    pub fn undo(&mut self) -> bool {
        let undone = self.history.undo(&mut self.graph);
        if undone {
            self.forget_missing();
            self.touch_structure();
        }
        undone
    }

    pub fn redo(&mut self) -> bool {
        let redone = self.history.redo(&mut self.graph);
        if redone {
            self.forget_missing();
            self.touch_structure();
        }
        redone
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Select `nodes` alone (or none).
    pub fn select(&mut self, nodes: Vec<NodeId>) {
        if self.selection != nodes || self.selected_wire.is_some() {
            self.selection = nodes;
            self.selected_wire = None;
            self.structure += 1;
        }
    }

    pub fn select_wire(&mut self, wire: Option<Wire>) {
        if self.selected_wire != wire || !self.selection.is_empty() {
            self.selected_wire = wire;
            self.selection.clear();
            self.structure += 1;
        }
    }

    /// Take out what is selected (nodes, or a wire).
    pub fn delete_selection(&mut self) {
        if let Some(wire) = self.selected_wire {
            self.edit(|graph| graph.unwire(wire));
        } else if !self.selection.is_empty() {
            let nodes = std::mem::take(&mut self.selection);
            self.edit(|graph| graph.remove_nodes(&nodes));
        }
    }

    pub fn copy(&mut self) {
        if !self.selection.is_empty() {
            self.clipboard = Some(self.graph.copy_nodes(&self.selection));
        }
    }

    /// Paste the clipboard a little down and right of where it came from.
    pub fn paste(&mut self) {
        let Some(part) = self.clipboard.clone() else {
            return;
        };
        let mut pasted = Vec::new();
        self.edit(|graph| pasted = graph.paste(&part, [40.0, 40.0]));
        self.selection = pasted;
        // The next paste lands further on.
        if let Some(clipboard) = &mut self.clipboard {
            for node in &mut clipboard.nodes {
                node.position = [node.position[0] + 40.0, node.position[1] + 40.0];
            }
        }
    }

    /// Write the graph and take a new revision of the asset, so terrains
    /// using it plant again.
    pub fn save(&mut self) -> Result<(), String> {
        self.graph.save(&self.path)?;
        let meta = assets::database()
            .get(self.asset)
            .map(|entry| entry.meta.clone())
            .ok_or_else(|| format!("asset {} does not exist", self.asset))?;
        assets::database().update(meta).map_err(|e| e.to_string())?;
        self.saved = self.graph.clone();
        self.file_time = file_time(&self.path);
        self.changed_on_disk = false;
        Ok(())
    }

    /// Back to the graph in the file (an undo step, so it can be undone).
    pub fn revert(&mut self) {
        if let Ok(graph) = ScatterGraph::load(&self.path) {
            self.saved = graph;
            self.file_time = file_time(&self.path);
        }
        self.changed_on_disk = false;
        let saved = self.saved.clone();
        self.edit(|graph| *graph = saved);
    }

    /// Drop selected nodes and wires that are gone.
    fn forget_missing(&mut self) {
        let graph = &self.graph;
        self.selection.retain(|id| graph.node(*id).is_some());
        if self.selected_wire.is_some_and(|w| !graph.wires.contains(&w)) {
            self.selected_wire = None;
        }
    }

    /// Keep the point counts up to date for the cell under the editor
    /// camera: picked up when worked out, asked for again (on a thread of
    /// their own) when the graph or the cell changed.
    /// Take up a change of the file made elsewhere (another tool, by hand):
    /// an unchanged copy loads it (as an undo step); a copy with unsaved
    /// edits keeps them and notes the clash (`changed_on_disk`).
    pub fn follow_file(&mut self) {
        let now = file_time(&self.path);
        if now == self.file_time || now.is_none() {
            return;
        }
        self.file_time = now;
        let Ok(graph) = ScatterGraph::load(&self.path) else {
            return;
        };
        if graph == self.saved {
            return;
        }
        if self.dirty() {
            self.changed_on_disk = true;
            return;
        }
        self.saved = graph.clone();
        self.edit(|g| *g = graph);
    }

    pub fn update_counts(&mut self, scene: &Scene) {
        if let Some((_, receiver)) = &self.counting {
            match receiver.try_recv() {
                Ok(result) => {
                    let (counted_for, _) = self.counting.take().expect("counting");
                    self.counts = match result {
                        Ok((counts, trace)) => {
                            self.trace = Some((counted_for.cell, trace));
                            Counts::Done(counts)
                        }
                        Err(e) => Counts::Failed(e),
                    };
                    self.counted_for = Some(counted_for);
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.counting = None;
                    self.counts = Counts::Failed("working out the points failed".into());
                }
            }
        }
        let Some(source) = scene.get_component::<Terrain>().and_then(|t| t.source().cloned()) else {
            self.counts = Counts::None;
            return;
        };
        let Some(eye) = scene.get_active_camera().map(|c| c.get_camera().get_eye_position()) else {
            return;
        };
        let cell = ((eye.x / DETAIL_CELL).floor() as i32, (eye.z / DETAIL_CELL).floor() as i32);
        let wanted = CountedFor { rules: self.rules, cell };
        if self.counted_for.as_ref() == Some(&wanted) {
            return;
        }
        let graph = self.graph.clone();
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("scatter-counts".into()).spawn(move || {
            let result = evaluate_traced(&graph, &*source as &dyn TerrainSource, cell_rect(cell)).map(|e| (e.counts, e.trace));
            let _ = sender.send(result);
        });
        if spawned.is_ok() {
            if !matches!(self.counts, Counts::Done(_)) {
                self.counts = Counts::Working;
            }
            self.counting = Some((wanted, receiver));
        }
    }

    /// Hand the graph as it is now to the scene's terrain to plant in place
    /// of the saved one, whenever its rules changed (a broken graph is not
    /// handed over; the last good one stays).
    pub fn push_preview(&mut self, scene: &mut Scene) {
        if self.pushed_rules == Some(self.rules) || !self.graph.problems().is_empty() {
            return;
        }
        if let Some(terrain) = scene.get_component_mut::<Terrain>() {
            terrain.scatter_mut().preview_graph(self.asset, Some(self.graph.clone()));
            self.pushed_rules = Some(self.rules);
        }
    }

    /// The terrain plants the saved graph again.
    pub fn clear_preview(&mut self, scene: &mut Scene) {
        if let Some(terrain) = scene.get_component_mut::<Terrain>() {
            terrain.scatter_mut().preview_graph(self.asset, None);
        }
        self.pushed_rules = None;
    }

    /// The points the preview shows, in coloured groups: with nothing (or
    /// several nodes) selected, each output's; with one node selected, that
    /// stage's: a filter's kept and dropped (and, keeping away, what from),
    /// a merge's by input, a split's by output, anything else's points.
    pub fn shown_points(&self) -> Vec<PointGroup> {
        let Some((_, trace)) = &self.trace else {
            return Vec::new();
        };
        let traced = |node: NodeId, pin: u8| -> Vec<[f32; 4]> {
            trace.get(&node).and_then(|pins| pins.get(pin as usize)).cloned().unwrap_or_default()
        };
        let into_traced = |node: NodeId, input: u8| -> Vec<[f32; 4]> {
            self.graph.wires_into(node, input).flat_map(|w| traced(w.from.node, w.from.pin)).collect()
        };
        let at = |points: Vec<[f32; 4]>| -> Vec<[f32; 3]> { points.into_iter().map(|[x, y, z, _]| [x, y, z]).collect() };
        let pin = |node: NodeId, pin: u8| at(traced(node, pin));
        let into = |node: NodeId, input: u8| at(into_traced(node, input));
        let group = |label: String, color, points| PointGroup { label, color, points };
        let selected = match self.selection.as_slice() {
            [id] => self.graph.node(*id),
            _ => None,
        };
        let Some(node) = selected else {
            return self
                .graph
                .outputs()
                .enumerate()
                .map(|(i, (id, spawn))| {
                    let model = spawn
                        .asset
                        .as_ref()
                        .and_then(|a| assets::database().display_name(a))
                        .unwrap_or_else(|| "no model".into());
                    group(format!("#{} {model}", id.0), PALETTE[i % PALETTE.len()], pin(id, 0))
                })
                .collect();
        };
        let id = node.id;
        match (&node.kind, node.kind.role()) {
            // Filters that fade density: each point coloured by how much of
            // it they let through (the world's chance there, the slope's
            // soft edge, the noise), dropped ones red.
            (
                NodeKind::Slope { .. }
                | NodeKind::Height { .. }
                | NodeKind::WorldChance { .. }
                | NodeKind::Noise { .. }
                | NodeKind::Biome { .. }
                | NodeKind::Clearance { .. },
                _,
            ) => {
                let factor_of: HashMap<[u32; 2], f32> = traced(id, 0)
                    .into_iter()
                    .map(|[x, _, z, density]| ([x.to_bits(), z.to_bits()], density))
                    .collect();
                let what = if matches!(node.kind, NodeKind::WorldChance { .. }) { "chance" } else { "kept" };
                let mut groups: Vec<PointGroup> = FADE
                    .iter()
                    .map(|(low, high, color)| match low {
                        None => group(format!("{what} 0 (dropped)"), *color, Vec::new()),
                        Some(low) => group(format!("{what} {low:.2}–{high:.2}"), *color, Vec::new()),
                    })
                    .collect();
                for [x, y, z, density] in into_traced(id, 0) {
                    let out = factor_of.get(&[x.to_bits(), z.to_bits()]).copied().unwrap_or(0.0);
                    let factor = if density > 0.0 { (out / density).clamp(0.0, 1.0) } else { 0.0 };
                    let bucket = if out <= 0.0 {
                        0
                    } else {
                        FADE.iter().position(|(low, high, _)| low.is_some_and(|low| factor > low && factor <= *high)).unwrap_or(FADE.len() - 1)
                    };
                    groups[bucket].points.push([x, y, z]);
                }
                groups
            }
            (kind, NodeRole::Filter) => {
                let kept = pin(id, 0);
                let kept_at: std::collections::HashSet<[u32; 2]> =
                    kept.iter().map(|p| [p[0].to_bits(), p[2].to_bits()]).collect();
                let dropped: Vec<[f32; 3]> = into(id, 0)
                    .into_iter()
                    .filter(|p| !kept_at.contains(&[p[0].to_bits(), p[2].to_bits()]))
                    .collect();
                let mut groups = vec![group("kept".into(), KEPT, kept), group("dropped".into(), DROPPED, dropped)];
                if matches!(kind, NodeKind::KeepAwayFrom { .. }) {
                    groups.push(group("kept away from".into(), AWAY_FROM, into(id, 1)));
                }
                groups
            }
            (NodeKind::Merge, _) => self
                .graph
                .wires_into(id, 0)
                .enumerate()
                .map(|(i, w)| {
                    let from = self.graph.node(w.from.node).map_or("?", |n| n.kind.label());
                    group(format!("from #{} {from}", w.from.node.0), PALETTE[i % PALETTE.len()], pin(w.from.node, w.from.pin))
                })
                .collect(),
            (NodeKind::Split { .. }, _) => {
                vec![group("above".into(), KEPT, pin(id, 0)), group("below".into(), PALETTE[1], pin(id, 1))]
            }
            (kind, _) => vec![group(kind.label().into(), PALETTE[0], pin(id, 0))],
        }
    }
}
