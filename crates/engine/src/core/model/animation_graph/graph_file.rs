//! Data description of an animation graph (RON on disk), editable by tools.

use std::{collections::HashMap, error::Error, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::core::{
    assets::{self, AssetDatabase, AssetRef},
    model::Animation,
};

use super::{AnimationGraph, State};

pub const GRAPH_FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AnimGraphFile {
    pub version: u32,
    #[serde(default)]
    pub inputs: Vec<InputDef>,
    pub default_state: String,
    #[serde(default)]
    pub states: Vec<StateDef>,
    /// Evaluated in order: the first transition out of the current state
    /// whose conditions all hold is taken.
    #[serde(default)]
    pub transitions: Vec<TransitionDef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InputDef {
    pub name: String,
    #[serde(default)]
    pub default: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StateDef {
    pub name: String,
    #[serde(default)]
    pub clips: Vec<ClipDef>,
    /// Play all clips at the same normalised time (for blended clips such
    /// as walk + strafe).
    #[serde(default)]
    pub sync: bool,
    /// Node position in the editor canvas.
    #[serde(default)]
    pub position: (f32, f32),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipDef {
    pub name: String,
    /// The animation asset to play; its part names the clip (the first
    /// clip of the file without one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    /// A raw file in `assets/animations/`. Used by graphs from before
    /// assets existed and when `asset` is not set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
}

/// Folder below the asset root that `ClipDef::file` is relative to.
const CLIP_FOLDER: &str = "animations";

impl ClipDef {
    pub fn asset(name: impl Into<String>, asset: AssetRef) -> Self {
        Self {
            name: name.into(),
            asset: Some(asset),
            file: String::new(),
        }
    }

    pub fn file(name: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            asset: None,
            file: file.into(),
        }
    }

    /// What the clip plays, for display: the asset's name or the file's.
    pub fn source_label(&self, database: &AssetDatabase) -> String {
        match &self.asset {
            Some(asset) => database
                .display_name(asset)
                .unwrap_or_else(|| format!("missing asset {}", asset.id)),
            None => self.file.clone(),
        }
    }

    /// Load the clip. Returns it with the key it is cached under, which
    /// changes when the asset is saved again.
    fn load(&self) -> Result<(String, Animation), Box<dyn Error>> {
        let Some(asset) = &self.asset else {
            return Ok((
                format!("file:{}", self.file),
                Animation::from_file(&self.name, &self.file)?,
            ));
        };
        let (source, key) = self.asset_source(asset)?;
        let animation = Animation::load(&self.name, &source, asset.part.as_deref())?;
        Ok((key, animation))
    }

    fn asset_source(&self, asset: &AssetRef) -> Result<(std::path::PathBuf, String), Box<dyn Error>> {
        let database = assets::database();
        let entry = database
            .get(asset.id)
            .ok_or_else(|| format!("animation asset {} does not exist", asset.id))?;
        Ok((entry.source_path(), Self::asset_key(asset, entry.revision)))
    }

    fn asset_key(asset: &AssetRef, revision: u64) -> String {
        format!(
            "asset:{}:{}:{revision}",
            asset.id,
            asset.part.as_deref().unwrap_or("")
        )
    }

    /// The key the clip is cached under right now, without loading it.
    fn cache_key(&self) -> Option<String> {
        match &self.asset {
            None => Some(format!("file:{}", self.file)),
            Some(asset) => assets::database()
                .get(asset.id)
                .map(|entry| Self::asset_key(asset, entry.revision)),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TransitionDef {
    pub from: String,
    pub to: String,
    /// Blend duration in seconds.
    pub blend: f32,
    /// All must hold (logical AND). Empty means "always".
    #[serde(default)]
    pub conditions: Vec<Comparison>,
}

/// `sum(sign * input) <op> value`, e.g. `forward - backward > 0`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Comparison {
    pub terms: Vec<Term>,
    pub op: CompareOp,
    pub value: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Term {
    pub input: String,
    /// +1.0 or -1.0.
    #[serde(default = "plus_one")]
    pub sign: f32,
}

fn plus_one() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Greater,
    GreaterEq,
    Less,
    LessEq,
    Equal,
    NotEqual,
}

impl CompareOp {
    pub const ALL: [CompareOp; 6] = [
        CompareOp::Greater,
        CompareOp::GreaterEq,
        CompareOp::Less,
        CompareOp::LessEq,
        CompareOp::Equal,
        CompareOp::NotEqual,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            CompareOp::Greater => ">",
            CompareOp::GreaterEq => ">=",
            CompareOp::Less => "<",
            CompareOp::LessEq => "<=",
            CompareOp::Equal => "==",
            CompareOp::NotEqual => "!=",
        }
    }
}

const EPSILON: f32 = 1e-4;

impl Comparison {
    pub fn evaluate(&self, inputs: &HashMap<String, f32>) -> bool {
        let sum: f32 = self
            .terms
            .iter()
            .map(|t| t.sign * inputs.get(&t.input).copied().unwrap_or(0.0))
            .sum();
        let diff = sum - self.value;
        match self.op {
            CompareOp::Greater => diff > EPSILON,
            CompareOp::GreaterEq => diff > -EPSILON,
            CompareOp::Less => diff < -EPSILON,
            CompareOp::LessEq => diff < EPSILON,
            CompareOp::Equal => diff.abs() <= EPSILON,
            CompareOp::NotEqual => diff.abs() > EPSILON,
        }
    }

    /// Human-readable form, e.g. `forward - backward > 0`.
    pub fn describe(&self) -> String {
        let mut text = String::new();
        for (i, term) in self.terms.iter().enumerate() {
            match (i, term.sign < 0.0) {
                (0, true) => text.push('-'),
                (0, false) => {}
                (_, true) => text.push_str(" - "),
                (_, false) => text.push_str(" + "),
            }
            text.push_str(&term.input);
        }
        if self.terms.is_empty() {
            text.push('0');
        }
        format!("{text} {} {}", self.op.symbol(), self.value)
    }
}

/// Loaded clips by source (file name, or asset and revision), shared across
/// graph rebuilds so an edited graph does not reload every FBX.
pub type ClipCache = HashMap<String, Animation>;

impl AnimGraphFile {
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        Ok(Self::from_ron(&fs::read_to_string(path)?)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        fs::write(path, self.to_ron()?)?;
        Ok(())
    }

    /// Refer to assets instead of raw files wherever a clip's file is a
    /// registered asset. Returns whether anything changed. Graphs written
    /// before assets existed are brought up to date this way when loaded.
    pub fn use_assets(&mut self, database: &AssetDatabase) -> bool {
        let mut changed = false;
        for clip in self.states.iter_mut().flat_map(|s| s.clips.iter_mut()) {
            if clip.asset.is_some() || clip.file.is_empty() {
                continue;
            }
            let source = database.root().join(CLIP_FOLDER).join(&clip.file);
            if let Some(id) = database.find_by_source(&source) {
                clip.asset = Some(AssetRef::whole(id));
                clip.file.clear();
                changed = true;
            }
        }
        changed
    }

    pub fn state(&self, name: &str) -> Option<&StateDef> {
        self.states.iter().find(|s| s.name == name)
    }

    /// Rename a state everywhere it is referenced.
    pub fn rename_state(&mut self, from: &str, to: &str) {
        for state in self.states.iter_mut().filter(|s| s.name == from) {
            state.name = to.to_owned();
        }
        for t in self.transitions.iter_mut() {
            if t.from == from {
                t.from = to.to_owned();
            }
            if t.to == from {
                t.to = to.to_owned();
            }
        }
        if self.default_state == from {
            self.default_state = to.to_owned();
        }
    }

    /// Remove a state and every transition touching it.
    pub fn remove_state(&mut self, name: &str) {
        self.states.retain(|s| s.name != name);
        self.transitions.retain(|t| t.from != name && t.to != name);
        if self.default_state == name {
            self.default_state = self
                .states
                .first()
                .map(|s| s.name.clone())
                .unwrap_or_default();
        }
    }

    /// Rename an input everywhere it is referenced.
    pub fn rename_input(&mut self, from: &str, to: &str) {
        for input in self.inputs.iter_mut().filter(|i| i.name == from) {
            input.name = to.to_owned();
        }
        for t in self.transitions.iter_mut() {
            for c in t.conditions.iter_mut() {
                for term in c.terms.iter_mut().filter(|term| term.input == from) {
                    term.input = to.to_owned();
                }
            }
        }
    }
}

impl AnimationGraph {
    /// Build a runtime graph from its description. Clips are taken from
    /// `cache` when present and loaded (and cached) otherwise.
    pub fn from_file(file: &AnimGraphFile, cache: &mut ClipCache) -> Result<Self, Box<dyn Error>> {
        if file.version > GRAPH_FILE_VERSION {
            return Err(format!(
                "animation graph version {} is newer than supported version {}",
                file.version, GRAPH_FILE_VERSION
            )
            .into());
        }
        if file.state(&file.default_state).is_none() {
            return Err(format!("default state '{}' does not exist", file.default_state).into());
        }
        let mut graph = AnimationGraph::new();
        for input in &file.inputs {
            graph.add_input(&input.name, input.default);
        }
        for def in &file.states {
            let mut state = State::new(&def.name);
            for clip in &def.clips {
                let cached = clip.cache_key().filter(|key| cache.contains_key(key));
                let key = match cached {
                    Some(key) => key,
                    None => {
                        let (key, animation) = clip.load()?;
                        cache.insert(key.clone(), animation);
                        key
                    }
                };
                let mut animation = cache[&key].clone();
                animation.set_name(&clip.name);
                state.add_animation(animation);
            }
            state.sync_animations(def.sync);
            for t in file.transitions.iter().filter(|t| t.from == def.name) {
                state.add_transition(&t.to, t.conditions.clone(), t.blend.max(0.001));
            }
            if def.name == file.default_state {
                graph.set_default_state(state);
            } else {
                graph.add_state(state);
            }
        }
        Ok(graph)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::assets::{database::tests::TempDir, test_models, AssetId};

    fn graph(clips: Vec<ClipDef>) -> AnimGraphFile {
        AnimGraphFile {
            version: GRAPH_FILE_VERSION,
            inputs: Vec::new(),
            default_state: "idle".into(),
            states: vec![StateDef {
                name: "idle".into(),
                clips,
                sync: false,
                position: (0.0, 0.0),
            }],
            transitions: Vec::new(),
        }
    }

    #[test]
    fn clips_from_before_assets_still_parse() {
        let clip: ClipDef = ron::from_str("(name: \"idle\", file: \"Idle.fbx\")").unwrap();
        assert_eq!(clip, ClipDef::file("idle", "Idle.fbx"));
    }

    #[test]
    fn asset_clips_round_trip_without_a_file() {
        let clip = ClipDef::asset("walk", AssetRef::part(AssetId::from(3), "walk cycle"));
        let text = ron::to_string(&clip).unwrap();
        assert!(!text.contains("file"), "{text}");
        assert_eq!(ron::from_str::<ClipDef>(&text).unwrap(), clip);
    }

    #[test]
    fn registered_files_are_referred_to_as_assets() {
        let project = TempDir::new("graph-assets");
        test_models::write_animation_gltf(&project.0.join("animations").join("Idle.gltf"), "idle");
        let mut database = AssetDatabase::new(&project.0);
        let (added, problems) = database.register_new_files();
        assert_eq!((added, problems.len()), (1, 0));
        let id = database.iter().next().unwrap().meta.id;

        let mut file = graph(vec![
            ClipDef::file("idle", "Idle.gltf"),
            ClipDef::file("walk", "NotImported.fbx"),
        ]);
        assert!(file.use_assets(&database));
        let clips = &file.states[0].clips;
        assert_eq!(clips[0], ClipDef::asset("idle", AssetRef::whole(id)));
        // A file that is no asset keeps its path.
        assert_eq!(clips[1], ClipDef::file("walk", "NotImported.fbx"));
        // Nothing more to do the second time.
        assert!(!file.use_assets(&database));
        assert_eq!(clips_label(&file, &database), vec!["Idle", "NotImported.fbx"]);
    }

    fn clips_label(file: &AnimGraphFile, database: &AssetDatabase) -> Vec<String> {
        file.states[0]
            .clips
            .iter()
            .map(|clip| clip.source_label(database))
            .collect()
    }

    #[test]
    fn a_clip_is_loaded_by_name_from_any_file() {
        let folder = TempDir::new("clip-load");
        let source = folder.0.join("Walk.gltf");
        test_models::write_animation_gltf(&source, "walk cycle");

        let first = Animation::load("a", &source, None).unwrap();
        let named = Animation::load("b", &source, Some("walk cycle")).unwrap();
        assert!(first.duration > 0.0);
        assert_eq!(first.duration, named.duration);
        assert!(Animation::load("c", &source, Some("run")).is_err());
        assert!(Animation::load("d", &folder.0.join("missing.gltf"), None).is_err());
    }
}
