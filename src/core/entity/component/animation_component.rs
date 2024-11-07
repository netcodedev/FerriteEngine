use std::collections::HashMap;

use glfw::{Glfw, WindowEvent};

use crate::core::{
    entity::Entity,
    model::{Animation, Pose},
    scene::Scene,
};

use super::{model_component::ModelComponent, Component};

pub struct AnimationComponent {
    animations: HashMap<String, Animation>,
    animation_weights: Vec<f32>,
    current_animations: Vec<String>,
    current_time: Vec<f32>,
    sync_animations: bool,
}

impl AnimationComponent {
    pub fn new() -> Self {
        AnimationComponent {
            animations: HashMap::new(),
            animation_weights: Vec::new(),
            current_animations: Vec::new(),
            current_time: Vec::new(),
            sync_animations: false,
        }
    }

    pub fn add_animation(&mut self, name: &str, mut animation: Animation) {
        animation.set_name(name);
        self.animations.insert(name.to_string(), animation);
    }

    pub fn play_animation(&mut self, name: &str) {
        self.current_animations = vec![name.to_string()];
        self.current_time = vec![0.0];
        self.sync_animations = false;
    }

    pub fn blend_animations(&mut self, name1: &str, name2: &str, weight: f32, sync: bool) {
        self.current_animations = vec![name1.to_string(), name2.to_string()];
        self.animation_weights = vec![weight];
        self.current_time = vec![0.0, 0.0];
        self.sync_animations = sync;
    }
}

impl Component for AnimationComponent {
    fn update(&mut self, _: &mut Scene, entity: &mut Entity, delta_time: f64) {
        let mut final_pose: Option<Pose> = None;
        let mut progression = None;
        let mut cycle_completed = false;
        for i in 0..self.current_animations.len() {
            if let Some(animation) = self.animations.get(&self.current_animations[i]) {
                self.current_time[i] += delta_time as f32 * animation.ticks_per_second;
                if self.current_time[i] > animation.duration {
                    self.current_time[i] = self.current_time[i] % animation.duration;
                    cycle_completed = true;
                }
                if self.sync_animations && progression.is_none() {
                    progression = Some(animation.get_progression(self.current_time[i]));
                }
                let new_pose = if let Some(progression) = progression {
                    animation.sample(progression * animation.duration)
                } else {
                    animation.sample(self.current_time[i])
                };
                if let Some(pose) = final_pose {
                    final_pose = Some(pose.interpolate(&new_pose, self.animation_weights[0]));
                } else {
                    final_pose = Some(new_pose);
                }
            }
        }
        if let Some(mut pose) = final_pose {
            pose.cycle_completed = cycle_completed;
            if let Some(model_component) = entity.get_component_mut::<ModelComponent>() {
                model_component.get_model_mut().apply_pose(&pose);
            }
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}
