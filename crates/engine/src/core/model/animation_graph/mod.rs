use std::collections::HashMap;

use super::Animation;

mod animation_graph;
pub mod graph_file;

use graph_file::Comparison;

pub struct AnimationGraph {
    inputs: HashMap<String, f32>,
    default_state: String,
    states: HashMap<String, State>,
    current_state: String,
    previous_state: Option<String>,
    transition_progress: f32,
    transition_speed: f32,
    delta_progress: f32,
    just_finished_transition: bool,
    cumulative_translation_error: HashMap<String, cgmath::Vector3<f32>>,
    /// How far the root turned in the last update, see `root_yaw_delta`.
    root_yaw_delta: f32,
}

pub struct State {
    name: String,
    /// In file order: with `sync_animations` the first one leads.
    animations: Vec<Animation>,
    animation_times: HashMap<String, f32>,
    animation_cycled: HashMap<String, bool>,
    sync_animations: bool,
    /// The clips' turn of the root about the up axis is taken out of the
    /// pose and reported instead (`AnimationGraph::root_yaw_delta`).
    root_rotation: bool,
    /// The root's turn in the last update, when `root_rotation`.
    root_yaw_delta: f32,
    transitions: Vec<Transition>,
}

pub struct Transition {
    to_state: String,
    /// All must hold; data instead of a closure so graphs can be saved and
    /// edited (see `graph_file`).
    conditions: Vec<Comparison>,
    transition_time: f32,
}
