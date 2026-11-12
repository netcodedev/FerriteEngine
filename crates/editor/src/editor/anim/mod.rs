//! Animation workspace: node canvas, inputs, inspector and live preview for
//! animation graphs (`*.animgraph.ron`).

use std::path::{Path, PathBuf};

use ferrite::{
    core::model::animation_graph::graph_file::AnimGraphFile,
    player::{player_graph_file, PLAYER_GRAPH_PATH},
};

pub mod canvas;
pub mod graph_inspector;
pub mod inputs;
pub mod preview;

#[derive(Clone, Debug, PartialEq)]
pub enum AnimSelection {
    None,
    State(String),
    /// Index into `AnimGraphFile::transitions`.
    Transition(usize),
}

/// What the live preview is doing, for highlighting in the canvas.
#[derive(Clone, Debug, Default)]
pub struct PreviewStatus {
    pub current: String,
    pub previous: Option<String>,
    pub progress: f32,
    pub error: Option<String>,
}

pub struct AnimEditor {
    pub file: AnimGraphFile,
    pub path: PathBuf,
    pub dirty: bool,
    pub selection: AnimSelection,
    /// Bumped on every edit; the preview rebuilds its graph when it changes.
    pub revision: u64,
    /// Bumped when states, transitions, clips, conditions or inputs are
    /// added, removed or renamed; panels rebuild their widgets then.
    pub structure: u64,
    /// Bumped when inputs are renamed, so pickers listing them refresh.
    pub labels: u64,
    pub preview: PreviewStatus,
}

impl AnimEditor {
    pub fn new() -> Self {
        Self {
            file: player_graph_file(),
            path: PathBuf::from(PLAYER_GRAPH_PATH),
            dirty: false,
            selection: AnimSelection::None,
            revision: 0,
            structure: 0,
            labels: 0,
            preview: PreviewStatus::default(),
        }
    }

    /// A value changed (position, blend time, condition value, …).
    pub fn touch(&mut self) {
        self.dirty = true;
        self.revision += 1;
    }

    /// Something was added, removed or renamed.
    pub fn touch_structure(&mut self) {
        self.touch();
        self.structure += 1;
    }

    pub fn file_name(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "graph".into());
        if self.dirty {
            format!("{name} *")
        } else {
            name
        }
    }

    /// A state name not used yet, based on `base`.
    pub fn unique_state_name(&self, base: &str) -> String {
        unique_name(base, |n| self.file.state(n).is_some())
    }

    pub fn unique_input_name(&self, base: &str) -> String {
        unique_name(base, |n| self.file.inputs.iter().any(|i| i.name == n))
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let file = if self.path.exists() {
            AnimGraphFile::load(&self.path).map_err(|e| e.to_string())?
        } else {
            player_graph_file()
        };
        self.file = file;
        self.dirty = false;
        self.selection = AnimSelection::None;
        self.touch_structure();
        self.dirty = false;
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.file.save(&self.path).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }
}

fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|n| !taken(n))
        .expect("unbounded range")
}

/// Clip files offered in the clip pickers.
pub fn animation_files() -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(Path::new("assets/animations"))
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| {
                    let lower = n.to_lowercase();
                    lower.ends_with(".fbx") || lower.ends_with(".gltf") || lower.ends_with(".glb")
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}
