//! Animation workspace: node canvas, inputs, inspector and live preview for
//! animation graphs (`*.animgraph.ron`).

use std::path::PathBuf;

use ferrite::{
    core::{
        assets::{self, AssetRef},
        model::animation_graph::graph_file::{AnimGraphFile, ClipDef},
    },
    player::{load_graph_file, player_graph_file, PLAYER_GRAPH_PATH},
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

    /// Switch to another graph file (the player graph falls back to the
    /// built-in graph until saved).
    pub fn open(&mut self, path: &str) -> Result<(), String> {
        let file = load_graph_file(path).map_err(|e| e.to_string())?;
        self.file = file;
        self.path = PathBuf::from(path);
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

/// Something a clip of a state can play.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipChoice {
    pub label: String,
    pub asset: Option<AssetRef>,
    /// Raw file in `assets/animations/`, for clips that are no assets.
    pub file: String,
}

impl ClipChoice {
    /// The choice a clip currently stands for.
    pub fn of(clip: &ClipDef) -> Self {
        Self {
            label: clip.source_label(&assets::database()),
            asset: clip.asset.clone(),
            file: clip.file.clone(),
        }
    }

    pub fn is(&self, clip: &ClipDef) -> bool {
        self.asset == clip.asset && self.file == clip.file
    }

    pub fn apply(&self, clip: &mut ClipDef) {
        clip.asset = self.asset.clone();
        clip.file = self.file.clone();
    }
}

/// The clips offered in the clip pickers: every clip of every animation
/// asset, and those model assets bring along.
pub fn clip_choices() -> Vec<ClipChoice> {
    let database = assets::database();
    let mut entries: Vec<_> = database
        .iter()
        .filter(|entry| !entry.meta.clips().is_empty())
        .collect();
    entries.sort_by_key(|entry| entry.meta.name.to_lowercase());
    let mut choices = Vec::new();
    for entry in entries {
        let clips = entry.meta.clips();
        for clip in clips {
            // A file with one clip is just called by its own name.
            let (label, asset) = if clips.len() == 1 {
                (entry.meta.name.clone(), AssetRef::whole(entry.meta.id))
            } else {
                (
                    format!("{} / {}", entry.meta.name, clip.name),
                    AssetRef::part(entry.meta.id, clip.name.clone()),
                )
            };
            choices.push(ClipChoice {
                label,
                asset: Some(asset),
                file: String::new(),
            });
        }
    }
    choices
}
