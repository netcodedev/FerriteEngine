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
    /// A clip held over part of the body for as long as it is wanted
    /// (`hold_layer`): arms carrying something while the legs walk.
    layer: Option<Layer>,
    /// How fast the graph plays (1: as made), e.g. slower under a load.
    speed: f32,
    /// The model's bones as they were before anything moved them (by name
    /// without namespace: full name and transform): bones no clip moves
    /// go back to it every frame. With the `ModelComponent::generation`
    /// of the model it was taken from.
    rest: Option<(u64, HashMap<String, (String, LocalTransform)>)>,
}

/// Seconds a one-shot clip takes to blend in and out.
const ONE_SHOT_FADE: f32 = 0.12;
/// Seconds a held layer takes to blend in and out.
const LAYER_FADE: f32 = 0.2;

/// A clip held (looping) over the graph's pose for the bones from one
/// bone down, blending in while it is wanted and out after.
struct Layer {
    animation: Animation,
    from_bone: String,
    bones: Option<HashSet<String>>,
    /// Ticks into the clip.
    time: f32,
    /// How much it shows now, and where that goes.
    weight: f32,
    wanted: bool,
}

/// `from_bone` (without namespace) and every bone below it in the model of
/// `entity`, by name without namespace.
fn bones_below(entity: &Entity, from_bone: &str) -> Option<HashSet<String>> {
    let skeleton = entity.get_component::<ModelComponent>().and_then(|model| model.get_model().skeleton())?;
    let top = skeleton.joints.iter().position(|joint| base_name(&joint.name) == from_bone)?;
    // Parents come before their children.
    let mut below = vec![false; skeleton.joints.len()];
    below[top] = true;
    for (index, joint) in skeleton.joints.iter().enumerate() {
        if joint.parent.is_some_and(|parent| below[parent]) {
            below[index] = true;
        }
    }
    Some(
        skeleton
            .joints
            .iter()
            .zip(below)
            .filter(|(_, below)| *below)
            .map(|(joint, _)| base_name(&joint.name).to_string())
            .collect(),
    )
}

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
            layer: None,
            speed: 1.0,
            rest: None,
        }
    }

    pub fn with_path(animation_graph: AnimationGraph, path: &str) -> Self {
        AnimationComponent {
            animation_graph,
            graph_path: Some(path.to_owned()),
            one_shot: None,
            layer: None,
            speed: 1.0,
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

    /// Hold `animation` (looping) over what the graph plays, for
    /// `from_bone` and the bones below it, blending in; `None` blends the
    /// held clip out. Called with the same clip every frame it is wanted,
    /// it goes on where it is.
    pub fn hold_layer(&mut self, animation: Option<&Animation>, from_bone: &str) {
        match (animation, &mut self.layer) {
            (Some(animation), Some(layer)) if layer.animation.name() == animation.name() && layer.from_bone == from_bone => {
                layer.wanted = true;
            }
            (Some(animation), layer) => {
                let weight = layer.as_ref().map_or(0.0, |l| l.weight);
                *layer = Some(Layer {
                    animation: animation.clone(),
                    from_bone: from_bone.to_string(),
                    bones: None,
                    time: 0.0,
                    weight,
                    wanted: true,
                });
            }
            (None, Some(layer)) => layer.wanted = false,
            (None, None) => {}
        }
    }

    /// How much the held layer shows (it blends in and out), 0 when none.
    pub fn layer_weight(&self) -> f32 {
        self.layer.as_ref().map_or(0.0, |layer| layer.weight)
    }

    /// Play the graph `speed` times as fast as it was made (1: as made).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.05, 4.0);
    }

    /// Lay the held clip over `pose` and move it on by `delta_time`.
    fn apply_layer(&mut self, pose: &mut crate::core::model::Pose, entity: &Entity, delta_time: f32) {
        let Some(layer) = &mut self.layer else {
            return;
        };
        let step = delta_time / LAYER_FADE;
        layer.weight = if layer.wanted { (layer.weight + step).min(1.0) } else { (layer.weight - step).max(0.0) };
        if layer.weight <= 0.0 && !layer.wanted {
            self.layer = None;
            return;
        }
        if layer.bones.is_none() {
            layer.bones = bones_below(entity, &layer.from_bone);
            if layer.bones.is_none() {
                log::warn!("held animation: the model has no bone {}", layer.from_bone);
                self.layer = None;
                return;
            }
        }
        let ticks = layer.animation.ticks_per_second.max(f32::EPSILON);
        let duration = layer.animation.duration.max(f32::EPSILON);
        let sampled = layer.animation.sample(layer.time % duration);
        if let Some(bones) = &layer.bones {
            pose.overlay(&sampled, layer.weight, bones);
        }
        layer.time = (layer.time + delta_time * ticks) % duration;
        // Wanted again next frame, or it blends out.
        layer.wanted = false;
    }

    /// Lay the one-shot clip over `pose` and move it on by `delta_time`.
    fn apply_one_shot(&mut self, pose: &mut crate::core::model::Pose, entity: &Entity, delta_time: f32) {
        let Some(shot) = &mut self.one_shot else {
            return;
        };
        if shot.bones.is_none() {
            shot.bones = bones_below(entity, &shot.from_bone);
            if shot.bones.is_none() {
                log::warn!("one-shot animation: the model has no bone {}", shot.from_bone);
                self.one_shot = None;
                return;
            }
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
        self.animation_graph.update(delta_time as f32 * self.speed);
        let mut pose = self.animation_graph.get_pose();
        let model = entity.get_component::<ModelComponent>();
        let generation = model.map(|model| model.generation());
        // Taken again from a model loaded in place of the one it was taken
        // from, whose skeleton may differ.
        if self.rest.as_ref().map(|(taken, _)| *taken) != generation {
            self.rest = model.and_then(|model| {
                let skeleton = model.get_model().skeleton()?;
                let rest = skeleton
                    .joints
                    .iter()
                    .map(|joint| {
                        let base = base_name(&joint.name).to_string();
                        (base, (joint.name.clone(), LocalTransform::from_matrix(&joint.local)))
                    })
                    .collect();
                Some((model.generation(), rest))
            });
        }
        if let Some(pose) = &mut pose {
            if let Some((_, rest)) = &self.rest {
                pose.fill_from(rest);
            }
            self.apply_layer(pose, entity, delta_time as f32);
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
