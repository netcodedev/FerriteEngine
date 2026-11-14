use std::collections::{HashMap, HashSet};

use glfw::{Glfw, WindowEvent};

use crate::{
    core::{
        entity::Entity,
        model::{animation_graph::AnimationGraph, ik::base_name, Animation, LocalTransform},
        scene::{scene_file::ComponentData, Scene},
    },
    player::{animation_graph_files, build_animation_graph, load_graph_file},
};

use super::{model_component::ModelComponent, Component, Property, PropertyValue};

pub struct AnimationComponent {
    animation_graph: AnimationGraph,
    /// Graph file the graph was built from; saved with the scene.
    graph_path: Option<String>,
    /// A clip played once over part of the body (`play_once`).
    one_shot: Option<OneShot>,
    /// The model's bones as they were before anything moved them (by name
    /// without namespace: full name and transform): bones no clip moves
    /// go back to it every frame.
    rest: Option<HashMap<String, (String, LocalTransform)>>,
}

/// Seconds a one-shot clip takes to blend in and out.
const ONE_SHOT_FADE: f32 = 0.12;

/// A clip played once over the graph's pose, for the bones from one bone
/// down: a swing of the arms while the legs walk on.
struct OneShot {
    animation: Animation,
    /// The topmost bone it moves (without namespace), e.g. `Spine`.
    from_bone: String,
    /// Names (without namespace) of `from_bone` and every bone below it,
    /// found on the first update.
    bones: Option<HashSet<String>>,
    /// Ticks into the clip.
    time: f32,
    speed: f32,
}

impl OneShot {
    /// How much it shows now: blending in, fully, blending out.
    fn weight(&self) -> f32 {
        let ticks = self.animation.ticks_per_second.max(f32::EPSILON);
        let seconds = self.time / ticks;
        let left = (self.animation.duration - self.time) / ticks;
        (seconds / ONE_SHOT_FADE).min(left / ONE_SHOT_FADE).clamp(0.0, 1.0)
    }
}

impl AnimationComponent {
    pub fn new(animation_graph: AnimationGraph) -> Self {
        AnimationComponent {
            animation_graph,
            graph_path: None,
            one_shot: None,
            rest: None,
        }
    }

    pub fn with_path(animation_graph: AnimationGraph, path: &str) -> Self {
        AnimationComponent {
            animation_graph,
            graph_path: Some(path.to_owned()),
            one_shot: None,
            rest: None,
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

    /// Play `animation` once, `speed` times as fast as it was made, over
    /// what the graph plays: only for `from_bone` (its name without
    /// namespace, e.g. `Spine`) and the bones below it. Blends in and out;
    /// replaces a one-shot that is still playing.
    pub fn play_once(&mut self, animation: Animation, from_bone: &str, speed: f32) {
        self.one_shot = Some(OneShot {
            animation,
            from_bone: from_bone.to_string(),
            bones: None,
            time: 0.0,
            speed: speed.max(0.01),
        });
    }

    /// Seconds (of the clip as made) the one-shot clip has played, while
    /// it plays.
    pub fn one_shot_time(&self) -> Option<f32> {
        self.one_shot
            .as_ref()
            .map(|shot| shot.time / shot.animation.ticks_per_second.max(f32::EPSILON))
    }

    /// How much the one-shot clip shows (it blends in and out), 0 when
    /// none plays.
    pub fn one_shot_weight(&self) -> f32 {
        self.one_shot.as_ref().map_or(0.0, |shot| shot.weight())
    }

    pub fn stop_one_shot(&mut self) {
        self.one_shot = None;
    }

    /// Lay the one-shot clip over `pose` and move it on by `delta_time`.
    fn apply_one_shot(&mut self, pose: &mut crate::core::model::Pose, entity: &Entity, delta_time: f32) {
        let Some(shot) = &mut self.one_shot else {
            return;
        };
        if shot.bones.is_none() {
            let skeleton = entity
                .get_component::<ModelComponent>()
                .and_then(|model| model.get_model().skeleton());
            let Some(skeleton) = skeleton else {
                self.one_shot = None;
                return;
            };
            let Some(top) = skeleton
                .joints
                .iter()
                .position(|joint| base_name(&joint.name) == shot.from_bone)
            else {
                log::warn!("one-shot animation: the model has no bone {}", shot.from_bone);
                self.one_shot = None;
                return;
            };
            // Parents come before their children.
            let mut below = vec![false; skeleton.joints.len()];
            below[top] = true;
            for (index, joint) in skeleton.joints.iter().enumerate() {
                if joint.parent.is_some_and(|parent| below[parent]) {
                    below[index] = true;
                }
            }
            shot.bones = Some(
                skeleton
                    .joints
                    .iter()
                    .zip(below)
                    .filter(|(_, below)| *below)
                    .map(|(joint, _)| base_name(&joint.name).to_string())
                    .collect(),
            );
        }
        let ticks = shot.animation.ticks_per_second.max(f32::EPSILON);
        let duration = shot.animation.duration;
        let weight = shot.weight();
        let sampled = shot.animation.sample(shot.time.min(duration));
        if let Some(bones) = &shot.bones {
            pose.overlay(&sampled, weight, bones);
        }
        shot.time += delta_time * shot.speed * ticks;
        if shot.time >= duration {
            self.one_shot = None;
        }
    }
}

impl Component for AnimationComponent {
    fn update(&mut self, _: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.animation_graph.update(delta_time as f32);
        let mut pose = self.animation_graph.get_pose();
        if self.rest.is_none() {
            self.rest = entity
                .get_component::<ModelComponent>()
                .and_then(|model| model.get_model().skeleton())
                .map(|skeleton| {
                    skeleton
                        .joints
                        .iter()
                        .map(|joint| {
                            let base = base_name(&joint.name).to_string();
                            (base, (joint.name.clone(), LocalTransform::from_matrix(&joint.local)))
                        })
                        .collect()
                });
        }
        if let Some(pose) = &mut pose {
            if let Some(rest) = &self.rest {
                pose.fill_from(rest);
            }
            self.apply_one_shot(pose, entity, delta_time as f32);
        }
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
