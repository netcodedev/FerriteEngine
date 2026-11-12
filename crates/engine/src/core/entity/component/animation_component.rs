use glfw::{Glfw, WindowEvent};

use crate::{
    core::{
        entity::Entity,
        model::animation_graph::AnimationGraph,
        scene::{scene_file::ComponentData, Scene},
    },
    player::{animation_graph_files, build_animation_graph, load_graph_file},
};

use super::{model_component::ModelComponent, Component, Property, PropertyValue};

pub struct AnimationComponent {
    animation_graph: AnimationGraph,
    /// Graph file the graph was built from; saved with the scene.
    graph_path: Option<String>,
}

impl AnimationComponent {
    pub fn new(animation_graph: AnimationGraph) -> Self {
        AnimationComponent {
            animation_graph,
            graph_path: None,
        }
    }

    pub fn with_path(animation_graph: AnimationGraph, path: &str) -> Self {
        AnimationComponent {
            animation_graph,
            graph_path: Some(path.to_owned()),
        }
    }

    pub fn graph_path(&self) -> Option<&str> {
        self.graph_path.as_deref()
    }

    /// Load `path` and use it, keeping current input values.
    pub fn load_graph(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let file = load_graph_file(path)?;
        let graph = build_animation_graph(&file)?;
        let names: Vec<String> = file.inputs.iter().map(|i| i.name.clone()).collect();
        self.replace_graph(graph, &names);
        self.graph_path = Some(path.to_owned());
        Ok(())
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

    fn display_name(&self) -> String {
        "Animation".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        self.graph_path
            .as_ref()
            .map(|path| ComponentData::AnimationGraph { path: path.clone() })
    }

    fn properties(&self) -> Vec<Property> {
        let current = self.graph_path.clone().unwrap_or_default();
        let mut options = animation_graph_files();
        if !current.is_empty() && !options.contains(&current) {
            options.insert(0, current.clone());
        }
        vec![Property::choice("graph", current, options)]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        if let ("graph", PropertyValue::Text(path)) = (name, value) {
            if self.graph_path.as_deref() == Some(path.as_str()) {
                return;
            }
            if let Err(e) = self.load_graph(&path) {
                log::warn!("could not load animation graph {path}: {e}");
            }
        }
    }
}
