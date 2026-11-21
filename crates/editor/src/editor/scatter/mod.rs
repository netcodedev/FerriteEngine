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
            graph::{evaluate, GraphHistory, NodeId, ScatterGraph, Wire},
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
    /// The outputs' placements (x, z) on the cell counted, for the map.
    pub map: Option<(CellKey, Vec<(NodeId, Vec<[f32; 2]>)>)>,
    counted_for: Option<CountedFor>,
    counting: Option<(CountedFor, Receiver<Result<Worked, String>>)>,
    /// The rules last handed to the terrain to plant (see `push_preview`).
    pushed_rules: Option<u64>,
}

/// What the counting thread worked out: per node its points, per output
/// where its models stand.
type Worked = (HashMap<NodeId, usize>, Vec<(NodeId, Vec<[f32; 2]>)>);

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
        Ok(Self {
            asset: id,
            name,
            path,
            saved: graph.clone(),
            graph,
            history: GraphHistory::default(),
            selection: Vec::new(),
            selected_wire: None,
            revision: 0,
            rules: 0,
            structure: 0,
            clipboard: None,
            counts: Counts::None,
            map: None,
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
        Ok(())
    }

    /// Back to the saved graph (an undo step, so it can be undone).
    pub fn revert(&mut self) {
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
    pub fn update_counts(&mut self, scene: &Scene) {
        if let Some((_, receiver)) = &self.counting {
            match receiver.try_recv() {
                Ok(result) => {
                    let (counted_for, _) = self.counting.take().expect("counting");
                    self.counts = match result {
                        Ok((counts, outputs)) => {
                            self.map = Some((counted_for.cell, outputs));
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
            let result = evaluate(&graph, &*source as &dyn TerrainSource, cell_rect(cell)).map(|e| {
                let outputs = e
                    .outputs
                    .iter()
                    .map(|(id, placements)| (*id, placements.iter().map(|p| [p.position[0], p.position[2]]).collect()))
                    .collect();
                (e.counts, outputs)
            });
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
}
