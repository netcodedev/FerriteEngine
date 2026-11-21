//! A material graph on the shared node canvas (`graph_canvas`).

use std::collections::HashSet;

use ferrite::core::{
    material::{
        edit,
        graph::{self, MaterialGraph, NodeGroup, NodeId, NodeKind},
    },
    scene::Scene,
};
use ferrite_ui::ui::theme;

use super::{
    super::{
        graph_canvas::{self, CanvasDoc, CanvasHost, CatalogEntry, NodeView, PinRef, Wire},
        Command, EditorState,
    },
    MaterialEdit,
};

pub fn host() -> CanvasHost {
    CanvasHost {
        doc: |s| s.material.as_ref().filter(|e| e.is_graph()).map(|e| e as &dyn CanvasDoc),
        doc_mut: |s| s.material.as_mut().filter(|e| e.is_graph()).map(|e| e as &mut dyn CanvasDoc),
        take_requests: |s| (std::mem::take(&mut s.material_open_menu), std::mem::take(&mut s.material_fit)),
        tick,
        close: || Command::CloseMaterialEditor,
        add_menu: || Command::MaterialAddMenu,
        fit: || Command::MaterialFit,
        undo: || Command::MaterialUndo,
        redo: || Command::MaterialRedo,
        revert: || Command::RevertMaterial,
        save: || Command::SaveMaterial,
    }
}

fn tick(state: &mut EditorState, _scene: &mut Scene) {
    if let Some(e) = state.material.as_mut() {
        e.follow_file();
    }
}

fn group_color(group: NodeGroup) -> theme::Color {
    match group {
        NodeGroup::Input => (0.20, 0.50, 0.32, 1.0),
        NodeGroup::Texture => (0.55, 0.30, 0.45, 1.0),
        NodeGroup::Math => (0.20, 0.36, 0.60, 1.0),
        NodeGroup::Effect => (0.42, 0.30, 0.60, 1.0),
        NodeGroup::Output => (0.66, 0.42, 0.16, 1.0),
    }
}

/// A node's settings in a few words.
fn summary(graph: &MaterialGraph, node: &graph::GraphNode) -> String {
    let props = edit::node_properties(graph, node.id);
    let shown: Vec<String> = props
        .iter()
        .filter(|p| !p.name.starts_with("in:") || matches!(node.kind, NodeKind::Output))
        .take(2)
        .map(|p| match &p.value {
            ferrite::core::entity::component::PropertyValue::Text(t) => t.clone(),
            ferrite::core::entity::component::PropertyValue::Float(f) => format!("{f:.2}"),
            ferrite::core::entity::component::PropertyValue::Int(i) => i.to_string(),
            ferrite::core::entity::component::PropertyValue::Bool(b) => b.to_string(),
        })
        .collect();
    match &node.kind {
        NodeKind::Parameter(name) => graph
            .param(name)
            .map(|p| p.ty.label().to_string())
            .unwrap_or_else(|| "missing".into()),
        NodeKind::Output => "the material's channels".into(),
        _ => shown.join(", "),
    }
}

fn entry(kind: &NodeKind) -> CatalogEntry {
    CatalogEntry { label: kind.label(), description: kind.description().to_string() }
}

/// What the add menu offers: the palette, a parameter node per parameter.
fn catalog(graph: &MaterialGraph) -> Vec<NodeKind> {
    let mut kinds: Vec<NodeKind> = graph.params.iter().map(|p| NodeKind::Parameter(p.name.clone())).collect();
    kinds.extend(NodeKind::palette().into_iter().filter(|k| !matches!(k, NodeKind::Parameter(_))));
    kinds
}

fn wire_of(w: &graph::Wire) -> Wire {
    Wire {
        from: PinRef { node: w.from.node.0, pin: w.from.pin },
        to: PinRef { node: w.to.node.0, pin: w.to.pin },
    }
}

fn wire_to(w: Wire) -> graph::Wire {
    graph::Wire {
        from: graph::PinRef { node: NodeId(w.from.node), pin: w.from.pin },
        to: graph::PinRef { node: NodeId(w.to.node), pin: w.to.pin },
    }
}

impl CanvasDoc for MaterialEdit {
    fn asset(&self) -> ferrite::core::assets::AssetId {
        self.asset
    }

    fn heading(&self) -> String {
        format!("Material graph  \u{b7}  {}{}", self.name, if self.dirty() { "  \u{25cf}" } else { "" })
    }

    fn footer(&self) -> (String, theme::Color) {
        if self.changed_on_disk {
            return (
                "The file was changed elsewhere; saving would overwrite that. Revert to take it up.".into(),
                theme::WARNING,
            );
        }
        if let Some(problem) = self.problems().first() {
            return (format!("The graph cannot be compiled (drawn magenta): {problem}"), theme::DANGER);
        }
        (
            "right-click or Space: add  \u{b7}  drag from an output to an input to wire  \u{b7}  Shift-drag: select  \u{b7}  nothing selected: the graph's parameters in the inspector".into(),
            theme::TEXT_DIM,
        )
    }

    fn structure(&self) -> u64 {
        self.structure
    }

    fn nodes(&self) -> Vec<NodeView> {
        let Some(graph) = self.graph() else { return Vec::new() };
        let broken: HashSet<NodeId> = graph.problems().iter().filter_map(|p| p.node).collect();
        graph
            .nodes
            .iter()
            .map(|n| NodeView {
                id: n.id.0,
                position: n.position,
                title: n.kind.label(),
                summary: summary(graph, n),
                note: (String::new(), theme::TEXT_DIM),
                color: group_color(n.kind.group()),
                inputs: n.kind.inputs().iter().map(|p| p.label.to_string()).collect(),
                outputs: n.kind.outputs().iter().map(|s| s.to_string()).collect(),
                problem: broken.contains(&n.id),
            })
            .collect()
    }

    fn wires(&self) -> Vec<Wire> {
        self.graph().map(|g| g.wires.iter().map(wire_of).collect()).unwrap_or_default()
    }

    fn selection(&self) -> Vec<graph_canvas::NodeId> {
        self.selection.iter().map(|n| n.0).collect()
    }

    fn selected_wire(&self) -> Option<Wire> {
        self.selected_wire.as_ref().map(wire_of)
    }

    fn can_undo(&self) -> bool {
        MaterialEdit::can_undo(self)
    }

    fn can_redo(&self) -> bool {
        MaterialEdit::can_redo(self)
    }

    fn catalog(&self) -> Vec<CatalogEntry> {
        self.graph().map(|g| catalog(g).iter().map(entry).collect()).unwrap_or_default()
    }

    fn tip(&self, node: graph_canvas::NodeId) -> Option<CatalogEntry> {
        self.graph()?.node(NodeId(node)).map(|n| entry(&n.kind))
    }

    fn add(&mut self, index: usize, at: [f32; 2]) {
        let Some(kind) = self.graph().and_then(|g| catalog(g).into_iter().nth(index)) else {
            return;
        };
        let mut added = None;
        self.edit_graph(|g| added = Some(g.add_at(kind, at)));
        MaterialEdit::select(self, added.into_iter().collect());
    }

    fn select(&mut self, nodes: Vec<graph_canvas::NodeId>) {
        MaterialEdit::select(self, nodes.into_iter().map(NodeId).collect());
    }

    fn select_wire(&mut self, wire: Option<Wire>) {
        MaterialEdit::select_wire(self, wire.map(wire_to));
    }

    fn begin_move(&mut self) {
        self.record();
    }

    fn move_selection(&mut self, by: [f32; 2]) {
        let selection = self.selection.clone();
        if let Some(g) = self.graph_mut() {
            for id in selection {
                if let Some(node) = g.node_mut(id) {
                    node.position = [node.position[0] + by[0], node.position[1] + by[1]];
                }
            }
        }
        self.touch();
    }

    fn unwire_into(&mut self, to: PinRef) -> Option<PinRef> {
        let wire = *self.graph()?.wire_into(NodeId(to.node), to.pin)?;
        self.edit_graph(|g| g.wires.retain(|w| *w != wire));
        Some(PinRef { node: wire.from.node.0, pin: wire.from.pin })
    }

    fn wire(&mut self, from: PinRef, to: PinRef) -> Result<(), String> {
        let graph = self.graph().ok_or("not a graph")?;
        let mut tried = graph.clone();
        let before = tried.problems();
        tried.connect(NodeId(from.node), from.pin, NodeId(to.node), to.pin);
        if let Some(problem) = tried.problems().into_iter().find(|p| !before.contains(p)) {
            return Err(problem.message);
        }
        self.edit_graph(|g| g.connect(NodeId(from.node), from.pin, NodeId(to.node), to.pin));
        Ok(())
    }
}
