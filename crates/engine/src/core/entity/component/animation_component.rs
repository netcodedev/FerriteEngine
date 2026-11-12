use glfw::{Glfw, WindowEvent};

use crate::core::{entity::Entity, model::animation_graph::AnimationGraph, scene::Scene};

use super::{model_component::ModelComponent, Component};

pub struct AnimationComponent {
    animation_graph: AnimationGraph,
}

impl AnimationComponent {
    pub fn new(animation_graph: AnimationGraph) -> Self {
        AnimationComponent { animation_graph }
    }

    pub fn set_input(&mut self, name: &str, value: f32) {
        self.animation_graph.set_input(name, value);
    }

    /// Swap in a rebuilt graph (e.g. after editing it), keeping the current
    /// input values.
    pub fn replace_graph(&mut self, mut graph: AnimationGraph, input_names: &[String]) {
        for name in input_names {
            if let Some(value) = self.animation_graph.get_input(name) {
                graph.set_input(name, value);
            }
        }
        self.animation_graph = graph;
    }

    pub fn graph(&self) -> &AnimationGraph {
        &self.animation_graph
    }
}

impl Component for AnimationComponent {
    fn update(&mut self, _: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.animation_graph.update(delta_time as f32);
        let pose = self.animation_graph.get_pose();
        if let Some(pose) = pose {
            if let Some(model_component) = entity.get_component_mut::<ModelComponent>() {
                model_component.get_model_mut().apply_pose(&pose);
            }
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        // Keep the idle loop playing so posed models look alive in the editor.
        Component::update(self, scene, entity, delta_time);
    }
}
