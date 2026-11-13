//! Game scripts: components written as `.rs` files in a project's `scripts`
//! folder, compiled into a library the editor loads (and reloads when they
//! change), found by name when a scene is loaded.
//!
//! A script is a struct with `#[derive(Script, Default)]` and an
//! `impl Behaviour`. Its public fields are its parameters: saved in scenes
//! and edited in the inspector. The engine wraps it in a `ScriptComponent`,
//! which does the saving and the editing.
//!
//! On a reload the old library stays loaded: components made by it (a
//! falling tree, say) may still be alive. Script components are rebuilt
//! from their parameters with the new code (`Scene::rebuild_scripts`);
//! a component that is not a script keeps running the code it was made
//! with.

mod library;
mod parameters;

use std::{borrow::Cow, collections::BTreeMap, sync::RwLock};

use cgmath::Matrix4;
use glfw::{Glfw, Window, WindowEvent};
use ron::Value;
use serde::{de::DeserializeOwned, Serialize};

use super::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity,
    },
    physics::physics_engine::PhysicsEngine,
    scene::{scene_file::ComponentData, Scene},
};

pub use library::{engine_workspace, load_library, ScriptCrate, REGISTER_SYMBOL};

/// What a script does. Every method is optional.
#[allow(unused_variables)]
pub trait Behaviour: 'static {
    /// Every frame of the game (play mode).
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {}
    /// Every frame while a scene is edited and not played.
    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {}
    fn render(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
    }
    /// After every opaque thing is drawn; for overlays and transparency.
    fn render_transparent(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
    }
    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &WindowEvent) {}
    /// When its entity leaves the scene: release what it owns outside of
    /// itself (physics bodies, say).
    fn on_remove(&mut self, physics: &mut PhysicsEngine) {}
}

/// A game script. Implemented by `#[derive(Script)]`.
pub trait Script: Behaviour + Default + Sized {
    /// Name it is saved under in scenes: the struct's name.
    const NAME: &'static str;
    /// Its parameters, in declaration order.
    const PARAMETERS: &'static [&'static str];
    /// Its parameters as one value.
    type Data: Serialize + DeserializeOwned + Default;

    fn data(&self) -> Self::Data;
    /// Take over `data` as its parameters, keeping its own state.
    fn apply(&mut self, data: Self::Data);

    fn from_data(data: Self::Data) -> Self {
        let mut script = Self::default();
        script.apply(data);
        script
    }
}

/// The component a script lives in on an entity.
pub struct ScriptComponent<S: Script> {
    script: S,
}

impl<S: Script> ScriptComponent<S> {
    pub fn new(script: S) -> Self {
        Self { script }
    }

    pub fn script(&self) -> &S {
        &self.script
    }

    pub fn script_mut(&mut self) -> &mut S {
        &mut self.script
    }

    fn parameters(&self) -> Value {
        to_value(&self.script.data())
    }
}

impl<S: Script> Component for ScriptComponent<S> {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.script.update(scene, entity, delta_time);
    }

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.script.update_in_editor(scene, entity, delta_time);
    }

    fn render(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        self.script.render(scene, entity, view_projection, parent_transform);
    }

    fn render_transparent(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        self.script
            .render_transparent(scene, entity, view_projection, parent_transform);
    }

    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &WindowEvent) {
        self.script.handle_event(glfw, window, event);
    }

    fn on_remove(&mut self, physics: &mut PhysicsEngine) {
        self.script.on_remove(physics);
    }

    fn display_name(&self) -> String {
        format!("Script: {}", S::NAME)
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::Script {
            name: S::NAME.to_string(),
            data: self.parameters(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        parameters::properties(&self.parameters(), S::PARAMETERS)
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let mut data = self.parameters();
        if !parameters::set(&mut data, name, value) {
            return;
        }
        match data.into_rust::<S::Data>() {
            Ok(data) => self.script.apply(data),
            Err(e) => log::warn!("script {}: cannot set {name}: {e}", S::NAME),
        }
    }
}

type Create = fn(&Value) -> Result<Box<dyn Component>, String>;

struct Entry {
    create: Create,
}

/// The scripts a game offers, by name.
#[derive(Default)]
pub struct ScriptRegistry {
    scripts: BTreeMap<String, Entry>,
}

impl ScriptRegistry {
    pub fn register<S: Script>(&mut self) {
        fn create<S: Script>(data: &Value) -> Result<Box<dyn Component>, String> {
            // No data at all (`()`): the script's defaults.
            let data: S::Data = match data {
                Value::Unit => S::Data::default(),
                data => data
                    .clone()
                    .into_rust()
                    .map_err(|e| format!("invalid parameters for script '{}': {e}", S::NAME))?,
            };
            Ok(Box::new(ScriptComponent::new(S::from_data(data))))
        }
        self.scripts.insert(
            S::NAME.to_string(),
            Entry {
                create: create::<S>,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }
}

static REGISTRY: RwLock<ScriptRegistry> = RwLock::new(ScriptRegistry {
    scripts: BTreeMap::new(),
});

/// Make the scripts of `registry` the ones scenes are built with,
/// replacing all before.
pub fn install(registry: ScriptRegistry) {
    *REGISTRY.write().unwrap() = registry;
}

/// A new component of the script `name`, from its saved parameters.
pub fn create(name: &str, data: &Value) -> Result<Box<dyn Component>, String> {
    let registry = REGISTRY.read().unwrap();
    let entry = registry
        .scripts
        .get(name)
        .ok_or_else(|| format!("no script '{name}' is loaded"))?;
    (entry.create)(data)
}

/// The names of every script loaded, sorted.
pub fn names() -> Vec<String> {
    REGISTRY.read().unwrap().scripts.keys().cloned().collect()
}

/// Export `$register: fn(&mut ScriptRegistry)` from a script library.
#[macro_export]
macro_rules! export_scripts {
    ($register:path) => {
        #[no_mangle]
        pub fn ferrite_register_scripts(registry: &mut $crate::core::script::ScriptRegistry) {
            $register(registry)
        }
    };
}

pub(crate) fn to_value<T: Serialize>(data: &T) -> Value {
    ron::to_string(data)
        .ok()
        .and_then(|text| ron::from_str(&text).ok())
        .unwrap_or(Value::Unit)
}

/// A parameter's name as shown: `move_speed` → `move speed`.
pub(crate) fn label(name: &str) -> Cow<'static, str> {
    Cow::Owned(name.replace('_', " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Script;

    #[derive(Script, Default)]
    pub struct Spin {
        /// Turns per second.
        pub speed: f32,
        pub enabled: bool,
        pub axis: [f32; 3],
        pub label: String,
        pub steps: u32,
        angle: f32,
    }

    impl Behaviour for Spin {
        fn update(&mut self, _: &mut Scene, _: &mut Entity, delta_time: f64) {
            self.angle += self.speed * delta_time as f32;
        }
    }

    fn registry() -> ScriptRegistry {
        let mut registry = ScriptRegistry::default();
        registry.register::<Spin>();
        registry
    }

    #[test]
    fn public_fields_are_the_parameters() {
        assert_eq!(Spin::PARAMETERS, &["speed", "enabled", "axis", "label", "steps"]);
        let component = ScriptComponent::new(Spin {
            speed: 2.5,
            ..Default::default()
        });
        let names: Vec<String> = component
            .properties()
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        assert_eq!(
            names,
            ["speed", "enabled", "axis x", "axis y", "axis z", "label", "steps"]
        );
    }

    #[test]
    fn parameters_are_edited_in_place() {
        let mut component = ScriptComponent::new(Spin::default());
        component.script.angle = 1.0;
        component.set_property("speed", PropertyValue::Float(3.0));
        component.set_property("axis y", PropertyValue::Float(1.0));
        component.set_property("steps", PropertyValue::Int(4));
        component.set_property("enabled", PropertyValue::Bool(true));
        component.set_property("label", PropertyValue::Text("wheel".into()));
        let spin = component.script();
        assert_eq!(spin.speed, 3.0);
        assert_eq!(spin.axis, [0.0, 1.0, 0.0]);
        assert_eq!(spin.steps, 4);
        assert!(spin.enabled);
        assert_eq!(spin.label, "wheel");
        // Its own state is kept.
        assert_eq!(spin.angle, 1.0);
    }

    #[test]
    fn a_script_is_rebuilt_from_its_saved_parameters() {
        let _guard = TEST_REGISTRY.lock().unwrap();
        install(registry());
        let saved = ScriptComponent::new(Spin {
            speed: 2.5,
            label: "fan".into(),
            ..Default::default()
        })
        .to_data()
        .unwrap();
        let ComponentData::Script { name, data } = &saved else {
            panic!("not script data: {saved:?}");
        };
        let rebuilt = create(name, data).unwrap();
        assert_eq!(rebuilt.to_data(), Some(saved.clone()));

        // Through a scene file's text and back.
        let text = ron::to_string(&saved).unwrap();
        assert_eq!(ron::from_str::<ComponentData>(&text).unwrap(), saved);

        // Saved without parameters: built with its defaults.
        let fresh = create("Spin", &Value::Unit).unwrap();
        assert_eq!(
            fresh.to_data(),
            ScriptComponent::new(Spin::default()).to_data()
        );
        assert!(create("Nope", &Value::Unit).is_err());
        install(ScriptRegistry::default());
    }

    /// Tests that install a registry take turns.
    static TEST_REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
