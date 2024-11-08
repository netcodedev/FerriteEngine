use std::collections::HashMap;

use crate::core::model::{Animation, Pose};

use super::{AnimationGraph, State, Transition};

impl AnimationGraph {
    pub fn new() -> Self {
        AnimationGraph {
            inputs: HashMap::new(),
            default_state: String::new(),
            states: HashMap::new(),
            current_state: String::new(),
            previous_state: String::new(),
            transition_progress: 1.0,
            transition_speed: 1.0,
        }
    }

    pub fn update(&mut self, delta_time: f32) {
        self.transition_progress += delta_time * self.transition_speed;
        if self.transition_progress > 1.0 {
            self.transition_progress = 1.0;
            self.previous_state = String::new();
        }
        if let Some(state) = self.states.get_mut(&self.previous_state) {
            state.update(delta_time);
        }
        let mut transition = false;
        if let Some(state) = self.states.get_mut(&self.current_state) {
            for transitions in &state.transitions {
                if (transitions.condition)(&self.inputs) {
                    self.previous_state = self.current_state.clone();
                    self.current_state = transitions.to_state.clone();
                    self.transition_progress = 0.0;
                    self.transition_speed = 1.0 / transitions.transition_time;
                    transition = true;
                    break;
                }
            }
            state.update(delta_time);
        }
        if transition {
            if let Some(state) = self.states.get_mut(&self.current_state) {
                state.reset();
            }
        }
    }

    pub fn add_state(&mut self, state: State) {
        self.states.insert(state.name.clone(), state);
    }

    pub fn set_default_state(&mut self, state: State) {
        let name = state.name.clone();
        self.add_state(state);
        self.default_state = name.clone();
        self.current_state = name;
    }

    pub fn get_pose(&self) -> Option<Pose> {
        let mut final_pose: Option<Pose> = None;
        if let Some(state) = self.states.get(&self.current_state) {
            if let Some(new_pose) = state.get_pose() {
                final_pose = Some(new_pose);
            }
        }
        if let Some(state) = self.states.get(&self.previous_state) {
            if let Some(new_pose) = state.get_pose() {
                if let Some(pose) = final_pose {
                    final_pose = Some(pose.interpolate(&new_pose, 1.0 - self.transition_progress));
                } else {
                    final_pose = Some(new_pose);
                }
            }
        }
        final_pose
    }

    pub fn add_input(&mut self, name: &str, value: f32) {
        self.inputs.insert(name.to_string(), value);
    }

    pub fn set_input(&mut self, name: &str, value: f32) {
        if let Some(input) = self.inputs.get_mut(name) {
            *input = value;
        }
    }
}

impl State {
    pub fn new(name: &str) -> Self {
        State {
            name: name.to_string(),
            animations: HashMap::new(),
            animation_times: HashMap::new(),
            animation_cycled: HashMap::new(),
            sync_animations: false,
            transitions: Vec::new(),
        }
    }

    pub fn update(&mut self, delta_time: f32) {
        for (name, animation) in &self.animations {
            let time = self.animation_times.entry(name.clone()).or_insert(0.0);
            let cycled = self.animation_cycled.entry(name.clone()).or_insert(false);
            *time += delta_time * animation.ticks_per_second;
            if *time > animation.duration {
                *cycled = true;
                *time = *time % animation.duration;
            } else {
                *cycled = false;
            }
        }
    }

    pub fn get_pose(&self) -> Option<Pose> {
        let mut final_pose: Option<Pose> = None;
        let mut cycled = false;
        let mut progress = None;
        for (name, animation) in &self.animations {
            if let Some(time) = self.animation_times.get(name) {
                cycled |= *self.animation_cycled.get(name).unwrap();
                if self.sync_animations && progress.is_none() {
                    progress = Some(animation.get_progression(*time));
                }
                let sample_time = if let Some(progress) = progress {
                    progress * animation.duration
                } else {
                    *time
                };
                let mut new_pose = animation.sample(sample_time);
                new_pose.cycle_completed = cycled;
                if let Some(pose) = final_pose {
                    new_pose = pose.interpolate(&new_pose, 0.5);
                    new_pose.cycle_completed = cycled;
                    final_pose = Some(new_pose);
                } else {
                    final_pose = Some(new_pose);
                }
            }
        }
        final_pose
    }

    pub fn reset(&mut self) {
        for time in self.animation_times.values_mut() {
            *time = 0.0;
        }
        for cycled in self.animation_cycled.values_mut() {
            *cycled = false;
        }
    }

    pub fn add_animation(&mut self, animation: Animation) {
        self.animations.insert(animation.name.clone(), animation);
    }

    pub fn add_transition(&mut self, to_state: &str, condition: Box<dyn Fn(&HashMap<String, f32>) -> bool>, transition_time: f32) {
        self.transitions.push(Transition {
            to_state: to_state.to_string(),
            condition,
            transition_time,
        });
    }

    pub fn sync_animations(&mut self, sync: bool) {
        self.sync_animations = sync;
    }
}