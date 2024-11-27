use std::collections::HashMap;

use super::Animation;

mod animation_graph;

pub struct AnimationGraph {
    inputs: HashMap<String, f32>,
    default_state: String,
    states: HashMap<String, State>,
    current_state: String,
    previous_state: Option<String>,
    transition_progress: f32,
    transition_speed: f32,
}

pub struct State {
    name: String,
    animations: HashMap<String, Animation>,
    animation_times: HashMap<String, f32>,
    animation_cycled: HashMap<String, bool>,
    sync_animations: bool,
    transitions: Vec<Transition>,
}

pub struct Transition {
    to_state: String,
    condition: Box<dyn Fn(&HashMap<String, f32>) -> bool>,
    transition_time: f32,
}
