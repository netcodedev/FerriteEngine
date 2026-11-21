//! The scatter graph on the shared node canvas (`graph_canvas`): how its
//! nodes look and how canvas edits reach the graph.

use ferrite::{
    core::scene::Scene,
    terrain::scatter::graph::{self, NodeId, NodeKind, NodeRole, SpawnRange, SplitBy},
};
use ferrite_ui::ui::theme;

use super::{
    super::{
        graph_canvas::{self, CanvasDoc, CanvasHost, CatalogEntry, NodeView, PinRef, Wire},
        Command, EditorState,
    },
    Counts, ScatterEdit,
};

/// The scatter graph workspace's canvas.
pub fn host() -> CanvasHost {
    CanvasHost {
        doc: |s| s.scatter.as_ref().map(|e| e as &dyn CanvasDoc),
        doc_mut: |s| s.scatter.as_mut().map(|e| e as &mut dyn CanvasDoc),
        take_requests: |s| (std::mem::take(&mut s.scatter_open_menu), std::mem::take(&mut s.scatter_fit)),
        tick: tick,
        close: || Command::CloseScatterEditor,
        add_menu: || Command::ScatterAddMenu,
        fit: || Command::ScatterFit,
        undo: || Command::ScatterUndo,
        redo: || Command::ScatterRedo,
        revert: || Command::RevertScatter,
        save: || Command::SaveScatter,
    }
}

fn tick(state: &mut EditorState, scene: &mut Scene) {
    state.scatter_camera = scene.get_active_camera().map(|c| c.get_camera().get_eye_position()).map(|p| [p.x, p.z]);
    if let Some(e) = state.scatter.as_mut() {
        e.follow_file();
        e.update_counts(scene);
        // The world shows the graph as it is being edited.
        e.push_preview(scene);
    }
}

fn id(node: NodeId) -> graph_canvas::NodeId {
    node.0
}

fn pin(p: PinRef) -> graph::PinRef {
    graph::PinRef { node: NodeId(p.node), pin: p.pin }
}

fn wire_of(w: &graph::Wire) -> Wire {
    Wire {
        from: PinRef { node: w.from.node.0, pin: w.from.pin },
        to: PinRef { node: w.to.node.0, pin: w.to.pin },
    }
}

fn entry(kind: &NodeKind) -> CatalogEntry {
    CatalogEntry { label: kind.label().to_string(), description: kind.description().to_string() }
}

impl CanvasDoc for ScatterEdit {
    fn asset(&self) -> ferrite::core::assets::AssetId {
        self.asset
    }

    fn heading(&self) -> String {
        format!("Scatter graph  \u{b7}  {}{}", self.name, if self.dirty() { "  \u{25cf}" } else { "" })
    }

    fn footer(&self) -> (String, theme::Color) {
        if self.changed_on_disk {
            return (
                "The file was changed elsewhere; saving would overwrite that. Revert to take it up.".to_string(),
                theme::WARNING,
            );
        }
        if let Some(problem) = self.graph.problems().first() {
            return (format!("The graph cannot be worked out: {problem}"), theme::DANGER);
        }
        let counts = match &self.counts {
            Counts::None => "no terrain to count points on".to_string(),
            Counts::Working => "counting points…".to_string(),
            Counts::Done(_) => "points counted on the 32 m cell under the camera".to_string(),
            Counts::Failed(e) => format!("counting failed: {e}"),
        };
        (
            format!("{counts}  \u{b7}  right-click or Space: add  \u{b7}  drag from an output to an input to wire  \u{b7}  Shift-drag: select"),
            theme::TEXT_DIM,
        )
    }

    fn structure(&self) -> u64 {
        self.structure
    }

    fn nodes(&self) -> Vec<NodeView> {
        self.graph
            .nodes
            .iter()
            .map(|n| {
                let note = match &self.counts {
                    Counts::Done(counts) => (
                        counts.get(&n.id).map_or("—".to_string(), |c| format!("{c} points")),
                        theme::SUCCESS,
                    ),
                    Counts::Working => ("…".to_string(), theme::TEXT_DIM),
                    Counts::None | Counts::Failed(_) => (String::new(), theme::TEXT_DIM),
                };
                NodeView {
                    id: id(n.id),
                    position: n.position,
                    title: n.kind.label().to_string(),
                    summary: summary(&n.kind),
                    note,
                    color: kind_color(&n.kind),
                    inputs: n.kind.inputs().iter().map(|p| p.name.to_string()).collect(),
                    outputs: n.kind.outputs().iter().map(|s| s.to_string()).collect(),
                    problem: false,
                }
            })
            .collect()
    }

    fn wires(&self) -> Vec<Wire> {
        self.graph.wires.iter().map(wire_of).collect()
    }

    fn selection(&self) -> Vec<graph_canvas::NodeId> {
        self.selection.iter().map(|n| n.0).collect()
    }

    fn selected_wire(&self) -> Option<Wire> {
        self.selected_wire.as_ref().map(wire_of)
    }

    fn can_undo(&self) -> bool {
        ScatterEdit::can_undo(self)
    }

    fn can_redo(&self) -> bool {
        ScatterEdit::can_redo(self)
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        NodeKind::catalog().iter().map(entry).collect()
    }

    fn tip(&self, node: graph_canvas::NodeId) -> Option<CatalogEntry> {
        self.graph.node(NodeId(node)).map(|n| entry(&n.kind))
    }

    fn add(&mut self, index: usize, at: [f32; 2]) {
        let Some(kind) = NodeKind::catalog().into_iter().nth(index) else {
            return;
        };
        let mut added = None;
        self.edit(|graph| added = Some(graph.add_node(kind, at)));
        ScatterEdit::select(self, added.into_iter().collect());
    }

    fn select(&mut self, nodes: Vec<graph_canvas::NodeId>) {
        ScatterEdit::select(self, nodes.into_iter().map(NodeId).collect());
    }

    fn select_wire(&mut self, wire: Option<Wire>) {
        let wire = wire.map(|w| graph::Wire { from: pin(w.from), to: pin(w.to) });
        ScatterEdit::select_wire(self, wire);
    }

    fn begin_move(&mut self) {
        self.record();
    }

    fn move_selection(&mut self, by: [f32; 2]) {
        let selection = self.selection.clone();
        self.graph.move_nodes(&selection, by);
        self.touch();
    }

    fn unwire_into(&mut self, to: PinRef) -> Option<PinRef> {
        let to = pin(to);
        let wire = self.graph.wires_into(to.node, to.pin).last().copied()?;
        self.edit(|graph| graph.unwire(wire));
        Some(PinRef { node: wire.from.node.0, pin: wire.from.pin })
    }

    fn wire(&mut self, from: PinRef, to: PinRef) -> Result<(), String> {
        let (from, to) = (pin(from), pin(to));
        self.graph.can_wire(from, to)?;
        self.edit(|graph| {
            let _ = graph.wire(from, to);
        });
        Ok(())
    }
}

/// The colour of a node's title bar by its role.
fn kind_color(kind: &NodeKind) -> theme::Color {
    match kind.role() {
        NodeRole::Source => (0.20, 0.50, 0.32, 1.0),
        NodeRole::Filter => (0.20, 0.36, 0.60, 1.0),
        NodeRole::Transform => (0.42, 0.30, 0.60, 1.0),
        NodeRole::Flow => (0.36, 0.38, 0.42, 1.0),
        NodeRole::Output => (0.66, 0.42, 0.16, 1.0),
    }
}

/// A node's main settings in a few words.
fn summary(kind: &NodeKind) -> String {
    let n = |v: f32| {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    // A value bound to a graphics setting: from Low to Ultra.
    let p = |param: &ferrite::core::graphics::Param<f32>| match param {
        ferrite::core::graphics::Param::Fixed(v) => n(*v),
        ferrite::core::graphics::Param::ByLevel { values, .. } => format!("{}–{}", n(values.low), n(values.ultra)),
    };
    match kind {
        NodeKind::SurfacePoints { spacing, jitter, .. } => format!("every {} m, jitter {}", p(spacing), n(*jitter)),
        NodeKind::RandomPoints { per_hectare, .. } => format!("{} per hectare", p(per_hectare)),
        NodeKind::Slope { min, max, .. } => format!("{}–{}°", n(*min), n(*max)),
        NodeKind::Height { min, max, .. } => format!("{}–{} m", n(*min), n(*max)),
        NodeKind::Clearance { feature, distance, .. } => format!("{} m from {}", n(*distance), if feature.is_empty() { "?" } else { feature }),
        NodeKind::Biome { biome } if biome.is_empty() => "no biome chosen".into(),
        NodeKind::Biome { biome } => biome.clone(),
        NodeKind::WorldChance { tag } if tag.is_empty() => "untagged".into(),
        NodeKind::WorldChance { tag } => format!("tag \"{tag}\""),
        NodeKind::Noise { wavelength, threshold, .. } => format!("{} m patches, {} kept", n(*wavelength), n(1.0 - threshold)),
        NodeKind::DensityCut { multiplier } if *multiplier == 1.0 => "keep by density".into(),
        NodeKind::DensityCut { multiplier } => format!("keep by density ×{}", n(*multiplier)),
        NodeKind::KeepAwayFrom { radius } => format!("{} m", n(*radius)),
        NodeKind::Rotate { yaw_min, yaw_max, align, tilt } => {
            format!("yaw {}–{}°, align {}, tilt {}°", n(*yaw_min), n(*yaw_max), n(*align), n(*tilt))
        }
        NodeKind::Scale { min, max, .. } => format!("×{}–{}", n(*min), n(*max)),
        NodeKind::Offset { min, max } => format!("{}–{} m up", n(*min), n(*max)),
        NodeKind::Merge => "all inputs".into(),
        NodeKind::Split { by, threshold } => format!(
            "{} at {}",
            match by {
                SplitBy::Density => "density",
                SplitBy::Random => "random",
            },
            n(*threshold)
        ),
        NodeKind::SpawnMesh(spawn) => {
            let model = match (&spawn.asset, spawn.model.trim()) {
                (Some(asset), _) => ferrite::core::assets::database()
                    .display_name(asset)
                    .unwrap_or_else(|| "missing model".into()),
                (None, "") => "no model".into(),
                (None, file) => file.to_string(),
            };
            match &spawn.range {
                SpawnRange::Landscape => format!("{model}, landscape"),
                SpawnRange::Detail { distance } => format!("{model}, {} m", p(distance)),
            }
        }
    }
}

