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
            previous_state: None,
            transition_progress: 1.0,
            transition_speed: 1.0,
            delta_progress: 0.0,
            just_finished_transition: false,
            cumulative_translation_error: HashMap::new(),
            root_yaw_delta: 0.0,
        }
    }

    /// How far (radians about the up axis, counter-clockwise seen from
    /// above) the clips of states with root rotation turned the root in the
    /// last update, weighted by how much of the pose they are. Their poses
    /// do not show it: whoever plays the graph turns the body by it, as with
    /// root motion.
    pub fn root_yaw_delta(&self) -> f32 {
        self.root_yaw_delta
    }

    pub fn update(&mut self, delta_time: f32) {
        self.delta_progress = delta_time * self.transition_speed;
        self.transition_progress += self.delta_progress;
        
        self.just_finished_transition = false;
        if self.transition_progress >= 1.0 && self.previous_state.is_some() {
            self.transition_progress = 1.0;
            self.previous_state = None;
            self.just_finished_transition = true;
        }

        if let Some(previous_state) = &self.previous_state {
            if let Some(state) = self.states.get_mut(previous_state) {
                state.update(delta_time);
            }
        }
        let mut transition = false;
        if let Some(state) = self.states.get_mut(&self.current_state) {
            // Also while a blend is running, so the graph follows inputs
            // that change quickly instead of finishing what it started.
            let fired = state
                .transitions
                .iter()
                .find(|t| t.conditions.iter().all(|c| c.evaluate(&self.inputs)))
                .map(|t| (t.to_state.clone(), t.transition_time));
            state.update(delta_time);
            if let Some((to_state, transition_time)) = fired {
                let speed = 1.0 / transition_time.max(1e-3);
                if self.previous_state.as_deref() == Some(to_state.as_str()) {
                    // Back to the state being blended out: the blend runs
                    // backwards from where it is, without a jump.
                    self.previous_state = Some(std::mem::replace(&mut self.current_state, to_state));
                    self.transition_progress = 1.0 - self.transition_progress;
                } else {
                    // The state being blended out (if any) is dropped; the
                    // one being blended in is blended out from here.
                    self.previous_state = Some(std::mem::replace(&mut self.current_state, to_state));
                    self.transition_progress = 0.0;
                    transition = true;
                }
                self.transition_speed = speed;
                self.cumulative_translation_error.clear();
                self.delta_progress = delta_time * self.transition_speed;
            }
        }
        if transition {
            if let Some(state) = self.states.get_mut(&self.current_state) {
                state.reset();
            }
        }
        let current = self
            .states
            .get(&self.current_state)
            .map_or(0.0, |s| s.root_yaw_delta);
        self.root_yaw_delta = match self.previous_state.as_ref().and_then(|p| self.states.get(p)) {
            Some(previous) => {
                current * self.transition_progress
                    + previous.root_yaw_delta * (1.0 - self.transition_progress)
            }
            None => current,
        };
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

    pub fn get_pose(&mut self) -> Option<Pose> {
        let mut final_pose: Option<Pose> = None;
        if let Some(state) = self.states.get(&self.current_state) {
            if let Some(new_pose) = state.get_pose() {
                final_pose = Some(new_pose);
            }
        }
        if let Some(previous_state) = &self.previous_state {
            if let Some(state) = self.states.get(previous_state) {
                if let Some(new_pose) = state.get_pose() {
                    if let Some(pose) = final_pose {
                        let mut interpolated_pose =
                            pose.interpolate(&new_pose, 1.0 - self.transition_progress);
                        
                        if self.transition_progress > 0.0 {
                            for (key, transform_b) in &new_pose.transforms {
                                if let Some(transform_a) = pose.transforms.get(key) {
                                    let error = (transform_a.translation - transform_b.translation) * self.delta_progress;
                                    let e_cum = self.cumulative_translation_error.entry(key.clone()).or_insert(cgmath::Vector3::new(0.0, 0.0, 0.0));
                                    *e_cum += error;
                                }
                            }
                        }
                        interpolated_pose.translation_errors = self.cumulative_translation_error.clone();
                        
                        final_pose = Some(interpolated_pose);
                    } else {
                        final_pose = Some(new_pose);
                    }
                }
            }
        } else if self.just_finished_transition {
            if let Some(pose) = &mut final_pose {
                pose.transition_finished = true;
            }
            self.just_finished_transition = false;
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

    /// Every input and its value.
    pub fn inputs(&self) -> &std::collections::HashMap<String, f32> {
        &self.inputs
    }

    pub fn get_input(&self, name: &str) -> Option<f32> {
        self.inputs.get(name).copied()
    }

    pub fn current_state(&self) -> &str {
        &self.current_state
    }

    /// The state being blended out, while a transition is running.
    pub fn previous_state(&self) -> Option<&str> {
        self.previous_state.as_deref()
    }

    /// 0 at the start of a transition, 1 when it has finished.
    pub fn transition_progress(&self) -> f32 {
        self.transition_progress
    }

    /// Jump to `name` without blending. Returns false if it does not exist.
    pub fn set_state(&mut self, name: &str) -> bool {
        if !self.states.contains_key(name) {
            return false;
        }
        self.current_state = name.to_owned();
        self.previous_state = None;
        self.transition_progress = 1.0;
        self.cumulative_translation_error.clear();
        if let Some(state) = self.states.get_mut(name) {
            state.reset();
        }
        true
    }

    /// Jump back to the default state without blending.
    pub fn reset(&mut self) {
        self.current_state = self.default_state.clone();
        self.previous_state = None;
        self.transition_progress = 1.0;
        self.cumulative_translation_error.clear();
        if let Some(state) = self.states.get_mut(&self.current_state) {
            state.reset();
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
            root_rotation: false,
            root_yaw_delta: 0.0,
            transitions: Vec::new(),
        }
    }

    pub fn update(&mut self, delta_time: f32) {
        let mut turned = 0.0;
        for (name, animation) in &self.animations {
            let time = self.animation_times.entry(name.clone()).or_insert(0.0);
            let cycled = self.animation_cycled.entry(name.clone()).or_insert(false);
            let before = *time;
            *time += delta_time * animation.ticks_per_second;
            if animation.duration <= 0.0 {
                // A single pose: it neither moves nor loops.
                *time = 0.0;
                *cycled = false;
            } else if *time > animation.duration {
                *cycled = true;
                *time = *time % animation.duration;
            } else {
                *cycled = false;
            }
            if self.root_rotation {
                turned += animation.root_yaw_change(before, *time, *cycled);
            }
        }
        // The clips of a state are blended evenly.
        self.root_yaw_delta = turned / self.animations.len().max(1) as f32;
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
                let mut new_pose = if self.root_rotation {
                    animation.sample_without_root_yaw(sample_time)
                } else {
                    animation.sample(sample_time)
                };
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
        self.root_yaw_delta = 0.0;
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

    pub fn add_transition(
        &mut self,
        to_state: &str,
        conditions: Vec<super::graph_file::Comparison>,
        transition_time: f32,
    ) {
        self.transitions.push(Transition {
            to_state: to_state.to_string(),
            conditions,
            transition_time,
        });
    }

    pub fn sync_animations(&mut self, sync: bool) {
        self.sync_animations = sync;
    }

    pub fn root_rotation(&mut self, root_rotation: bool) {
        self.root_rotation = root_rotation;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn a_clip_of_no_length_keeps_its_time_a_number() {
        let mut state = State::new("pose");
        state.root_rotation(true);
        state.add_animation(Animation {
            name: "pose".into(),
            duration: 0.0,
            ticks_per_second: 25.0,
            channels: HashMap::new(),
            root_channel: None,
        });
        state.update(0.1);
        state.update(0.1);
        assert_eq!(state.animation_times["pose"], 0.0);
        assert!(state.root_yaw_delta.is_finite());
        assert!(state.get_pose().is_some());
    }
}
