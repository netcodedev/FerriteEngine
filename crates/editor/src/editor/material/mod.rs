//! The material workspace: one material asset or material graph asset
//! open for editing. A material's settings and parameters are edited in
//! the inspector with a preview beside them; a graph's nodes on the shared
//! node canvas (`graph_canvas`), the selected node's settings (or the
//! graph's parameters) in the inspector, a preview in the right column.
//! See docs/materials.md, step 6.

pub mod bridge;
pub mod canvas;
pub mod inspector;
pub mod preview;

use std::{path::PathBuf, time::Instant};

use ferrite::core::{
    assets::{self, AssetId, AssetKind},
    entity::component::PropertyValue,
    material::{
        asset::{self as material_asset, MaterialFile},
        edit,
        graph::{MaterialGraph, NodeId, NodeKind, Wire},
        preview::Shape,
        CompiledGraph, Material,
    },
};

/// What is open.
#[derive(Clone, Debug, PartialEq)]
pub enum Doc {
    Material(MaterialFile),
    Graph(MaterialGraph),
}

pub struct MaterialEdit {
    pub asset: AssetId,
    pub name: String,
    pub path: PathBuf,
    pub doc: Doc,
    saved: Doc,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    pub selection: Vec<NodeId>,
    pub selected_wire: Option<Wire>,
    /// Bumped on every change.
    pub revision: u64,
    /// Bumped when nodes or parameters come or go or the selection changes:
    /// panels rebuild then.
    pub structure: u64,
    /// The preview's shape and wind (0: still).
    pub shape: Shape,
    pub wind: f32,
    /// When the preview material was last made, and for which revision.
    preview_made: Option<(u64, Instant)>,
    preview_material: Option<Material>,
    /// A model whose part uses this material, loaded for the preview: its
    /// loader's maps fill `auto` textures. `None` inside: none uses it.
    auto_source: Option<Option<(ferrite::core::model::Model, usize)>>,
    /// Last change and when: changes to the same setting in quick
    /// succession are one undo step.
    last_change: Option<(String, Instant)>,
    file_time: Option<std::time::SystemTime>,
    /// The file changed on disk while this copy had unsaved edits.
    pub changed_on_disk: bool,
}

fn file_time(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Edits to one setting this close together are one undo step.
const ONE_STEP: std::time::Duration = std::time::Duration::from_millis(700);
/// The preview's material is made again this long after the last edit (a
/// new graph compiles its programs).
const PREVIEW_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

fn load(path: &std::path::Path, graph: bool) -> Result<Doc, String> {
    if graph {
        MaterialGraph::load(path).map(Doc::Graph)
    } else {
        MaterialFile::load(path).map(Doc::Material)
    }
}

impl MaterialEdit {
    /// Whether asset `id` is opened here.
    pub fn handles(id: AssetId) -> bool {
        assets::database()
            .get(id)
            .is_some_and(|e| matches!(e.meta.kind, AssetKind::Material(_) | AssetKind::MaterialGraph(_)))
    }

    pub fn open(id: AssetId) -> Result<Self, String> {
        let (name, path, graph) = {
            let database = assets::database();
            let entry = database.get(id).ok_or_else(|| format!("asset {id} does not exist"))?;
            let graph = match entry.meta.kind {
                AssetKind::Material(_) => false,
                AssetKind::MaterialGraph(_) => true,
                _ => return Err(format!("'{}' is not a material", entry.meta.name)),
            };
            (entry.meta.name.clone(), entry.source_path(), graph)
        };
        let doc = load(&path, graph)?;
        Ok(Self {
            asset: id,
            name,
            file_time: file_time(&path),
            path,
            saved: doc.clone(),
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            selection: Vec::new(),
            selected_wire: None,
            revision: 0,
            structure: 0,
            shape: Shape::Sphere,
            wind: 0.0,
            preview_made: None,
            preview_material: None,
            auto_source: None,
            last_change: None,
            changed_on_disk: false,
        })
    }

    pub fn is_graph(&self) -> bool {
        matches!(self.doc, Doc::Graph(_))
    }

    pub fn graph(&self) -> Option<&MaterialGraph> {
        match &self.doc {
            Doc::Graph(g) => Some(g),
            Doc::Material(_) => None,
        }
    }

    pub fn graph_mut(&mut self) -> Option<&mut MaterialGraph> {
        match &mut self.doc {
            Doc::Graph(g) => Some(g),
            Doc::Material(_) => None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.doc != self.saved
    }

    /// Note the document as it is before an edit (one undo step).
    pub fn record(&mut self) {
        self.undo.push(self.doc.clone());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last_change = None;
    }

    /// One undo step for edits of `setting` in quick succession.
    fn record_setting(&mut self, setting: &str) {
        let now = Instant::now();
        let same = self
            .last_change
            .as_ref()
            .is_some_and(|(name, at)| name == setting && now.duration_since(*at) < ONE_STEP);
        if !same {
            self.record();
        }
        self.last_change = Some((setting.to_string(), now));
    }

    pub fn touch(&mut self) {
        self.revision += 1;
    }

    pub fn touch_structure(&mut self) {
        self.revision += 1;
        self.structure += 1;
    }

    /// Change the graph as one undo step.
    pub fn edit_graph(&mut self, change: impl FnOnce(&mut MaterialGraph)) {
        if !self.is_graph() {
            return;
        }
        self.record();
        if let Some(graph) = self.graph_mut() {
            change(graph);
        }
        self.forget_missing();
        self.touch_structure();
    }

    pub fn undo(&mut self) -> bool {
        let Some(doc) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.doc, doc));
        self.forget_missing();
        self.touch_structure();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(doc) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.doc, doc));
        self.forget_missing();
        self.touch_structure();
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

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

    /// Take out what is selected (nodes, or a wire); the output stays.
    pub fn delete_selection(&mut self) {
        if let Some(wire) = self.selected_wire.take() {
            self.edit_graph(|g| g.wires.retain(|w| *w != wire));
            return;
        }
        let nodes = std::mem::take(&mut self.selection);
        if nodes.is_empty() {
            return;
        }
        self.edit_graph(|g| {
            for id in nodes {
                if g.node(id).is_some_and(|n| n.kind != NodeKind::Output) {
                    g.remove(id);
                }
            }
        });
    }

    fn forget_missing(&mut self) {
        let Doc::Graph(graph) = &self.doc else {
            self.selection.clear();
            self.selected_wire = None;
            return;
        };
        self.selection.retain(|id| graph.node(*id).is_some());
        if self.selected_wire.is_some_and(|w| !graph.wires.contains(&w)) {
            self.selected_wire = None;
        }
    }

    /// The graph a material's settings are read against: its graph (the
    /// graph itself when one is open).
    pub fn settings_graph(&self) -> MaterialGraph {
        match &self.doc {
            Doc::Graph(g) => g.clone(),
            Doc::Material(file) => material_asset::graph_for(&file.graph).graph.clone(),
        }
    }

    /// Set a material property (see `material::edit`), one undo step per
    /// setting edited in quick succession.
    pub fn set_material_property(&mut self, name: &str, value: &PropertyValue) -> Result<(), String> {
        let graph = self.settings_graph();
        let Doc::Material(file) = &self.doc else {
            return Err("not a material".into());
        };
        let mut changed = file.clone();
        edit::set_material_property(&mut changed, &graph, name, value)?;
        if Doc::Material(changed.clone()) == self.doc {
            return Ok(());
        }
        self.record_setting(name);
        let graph_changed = matches!(&self.doc, Doc::Material(f) if f.graph != changed.graph);
        self.doc = Doc::Material(changed);
        if graph_changed {
            self.touch_structure();
        } else {
            self.touch();
        }
        Ok(())
    }

    /// Set a node's setting (see `material::edit::node_properties`).
    pub fn set_node_property(&mut self, node: NodeId, name: &str, value: &PropertyValue) -> Result<(), String> {
        let Some(graph) = self.graph() else {
            return Err("not a graph".into());
        };
        let mut changed = graph.clone();
        edit::set_node_property(&mut changed, node, name, value)?;
        if Some(&changed) == self.graph() {
            return Ok(());
        }
        self.record_setting(&format!("{}:{name}", node.0));
        // A constant's width changes its pins' meaning: rebuild.
        let structural = name == "width" || name == "parameter" || name == "operation" || name == "space";
        self.doc = Doc::Graph(changed);
        if structural {
            self.touch_structure();
        } else {
            self.touch();
        }
        Ok(())
    }

    /// Apply tool operations to the graph as one undo step. Returns the
    /// ids of nodes added.
    pub fn apply_ops(&mut self, ops: &[serde_json::Value]) -> Result<Vec<NodeId>, String> {
        let Some(graph) = self.graph() else {
            return Err("not a graph".into());
        };
        let mut changed = graph.clone();
        let mut added = Vec::new();
        for op in ops {
            if let Some(id) = edit::apply_op(&mut changed, op)? {
                added.push(id);
            }
        }
        if Some(&changed) != self.graph() {
            self.record();
            self.doc = Doc::Graph(changed);
            self.forget_missing();
            self.touch_structure();
        }
        Ok(added)
    }

    /// Write the document and take a new revision of the asset, so what
    /// uses it takes it up.
    pub fn save(&mut self) -> Result<(), String> {
        match &self.doc {
            Doc::Material(file) => file.save(&self.path)?,
            Doc::Graph(graph) => graph.save(&self.path)?,
        }
        let meta = assets::database()
            .get(self.asset)
            .map(|entry| entry.meta.clone())
            .ok_or_else(|| format!("asset {} does not exist", self.asset))?;
        assets::database().update(meta).map_err(|e| e.to_string())?;
        self.saved = self.doc.clone();
        self.file_time = file_time(&self.path);
        self.changed_on_disk = false;
        Ok(())
    }

    /// Back to the file (an undo step).
    pub fn revert(&mut self) {
        if let Ok(doc) = load(&self.path, self.is_graph()) {
            self.saved = doc;
            self.file_time = file_time(&self.path);
        }
        self.changed_on_disk = false;
        self.record();
        self.doc = self.saved.clone();
        self.forget_missing();
        self.touch_structure();
    }

    /// Take up a change of the file made elsewhere: an unchanged copy
    /// loads it, one with unsaved edits notes the clash.
    pub fn follow_file(&mut self) {
        let now = file_time(&self.path);
        if now == self.file_time || now.is_none() {
            return;
        }
        self.file_time = now;
        let Ok(doc) = load(&self.path, self.is_graph()) else {
            return;
        };
        if doc == self.saved {
            return;
        }
        if self.dirty() {
            self.changed_on_disk = true;
            return;
        }
        self.saved = doc.clone();
        self.record();
        self.doc = doc;
        self.forget_missing();
        self.touch_structure();
    }

    /// The material the preview draws: made again a moment after edits
    /// (a changed graph compiles new programs). `None` until made.
    pub fn preview_material(&mut self) -> Option<&Material> {
        let due = match self.preview_made {
            None => true,
            Some((revision, at)) => revision != self.revision && at.elapsed() >= PREVIEW_DELAY,
        };
        if due {
            if self.auto_source.is_none() && !self.is_graph() {
                self.auto_source = Some(load_user_of(self.asset));
            }
            let auto = self
                .auto_source
                .as_ref()
                .and_then(|found| found.as_ref())
                .and_then(|(model, index)| model.file_material(*index));
            let material = match &self.doc {
                Doc::Material(file) => material_asset::build(file, &self.name, auto),
                Doc::Graph(graph) => {
                    let mut material = Material::new(CompiledGraph::new(&self.name, graph.clone()));
                    // Cut out where the graph says, so its opacity shows.
                    material.alpha = ferrite::core::material::AlphaMode::Cutout(0.5);
                    material
                }
            };
            self.preview_material = Some(material);
            self.preview_made = Some((self.revision, Instant::now()));
        } else if let Some((revision, _)) = self.preview_made {
            if revision != self.revision {
                // Wait for the edits to settle; keep the timer running from
                // the last change.
            }
        }
        self.preview_material.as_ref()
    }

    /// Where the preview's automatic textures come from.
    pub fn auto_note(&self) -> String {
        match &self.auto_source {
            Some(Some((model, index))) => {
                let maps = model
                    .file_material(*index)
                    .map(|m| ["base_color_map", "normal_map", "roughness_map"].iter().filter(|p| m.texture(p).is_some()).count())
                    .unwrap_or(0);
                format!("Automatic textures from the model that uses it ({maps} maps found)")
            }
            Some(None) => "No model uses it: automatic textures stay empty".into(),
            None => String::new(),
        }
    }

    /// Problems of the open graph, or of the material's graph.
    pub fn problems(&self) -> Vec<String> {
        self.settings_graph().problems().iter().map(|p| p.to_string()).collect()
    }
}

/// The first model asset with a slot drawn with material `id`, loaded (its
/// file's materials), and the slot. Must run on the render thread.
fn load_user_of(id: AssetId) -> Option<(ferrite::core::model::Model, usize)> {
    let (path, scale, index) = {
        let database = assets::database();
        let found = database.iter().find_map(|e| {
            let model = e.meta.as_model()?;
            let index = model.materials.iter().position(|m| m.material == Some(id))?;
            Some((e.source_path(), model.scale, index))
        });
        found?
    };
    let path = path.to_string_lossy().replace('\\', "/");
    let mut model = ferrite::core::model::ModelBuilder::from_file(&path).ok()?.with_scale(scale).build();
    model.init();
    Some((model, index))
}
