//! Data description of an animation graph (RON on disk), editable by tools.

use std::{collections::HashMap, error::Error, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::core::model::Animation;

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
    /// File in `assets/animations/`.
    pub file: String,
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

/// Loaded clips by file name, shared across graph rebuilds so an edited
/// graph does not reload every FBX.
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
                if !cache.contains_key(&clip.file) {
                    let animation = Animation::from_file(&clip.name, &clip.file)?;
                    cache.insert(clip.file.clone(), animation);
                }
                let mut animation = cache[&clip.file].clone();
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
