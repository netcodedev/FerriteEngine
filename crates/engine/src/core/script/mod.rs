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
//! with. The old library's statics stay too: caches in them must be
//! emptied with `on_unload`, or every reload keeps another copy.

mod library;
pub(crate) mod parameters;

use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Mutex, RwLock},
};

use cgmath::{Matrix4, Point3, Vector3};
use glfw::{Glfw, Window, WindowEvent};
use ron::{value::RawValue, Value};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};

use crate::terrain::{generators::GeneratorEntry, source::TerrainGenerator};

use super::{
    entity::{
        component::{Component, Property, PropertyValue},
        Entity, EntityHandle,
    },
    physics::physics_engine::PhysicsEngine,
    scene::{scene_file::ComponentData, Scene},
};

pub use library::{
    clean_cargo_env, engine_workspace, is_inherited_cargo_var, load_library, packaged_library,
    build_dir, target_dir, ScriptCrate, REGISTER_SYMBOL, SCRIPTS_LIBRARY,
};
pub(crate) use library::first_error;

/// What a script does. Every method is optional.
#[allow(unused_variables)]
pub trait Behaviour: 'static {
    /// Every frame of the game (play mode).
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {}
    /// Every frame while a scene is edited and not played.
    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {}
    /// The graphics settings changed (and once before the first update):
    /// read them with `graphics::level("vegetation")`, `graphics::get_bool`
    /// and friends, for a setting of the project's own, say
    /// (docs/graphics-settings.md).
    fn on_graphics_changed(&mut self, scene: &mut Scene, entity: &mut Entity) {}
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
    /// 2D drawing on top of the finished image: a HUD, an inventory, menus
    /// (`PlaneRenderer`, `Text`), in the viewport's pixels.
    fn render_overlay(&self, scene: &Scene, entity: &Entity) {}
    fn handle_event(&mut self, glfw: &mut Glfw, window: &mut Window, event: &WindowEvent) {}
    /// Distance along the ray (`dir` normalized) to the nearest point where
    /// it hits what the script draws, for picking in the editor (see
    /// `Component::raycast`). A script that draws its own models answers
    /// here, or clicking them selects nothing.
    fn raycast(&self, transform: &Matrix4<f32>, origin: Point3<f32>, dir: Vector3<f32>) -> Option<f32> {
        None
    }
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
    /// The graphics settings the script was last told of.
    graphics: crate::core::graphics::Watch,
}

impl<S: Script> ScriptComponent<S> {
    pub fn new(script: S) -> Self {
        Self { script, graphics: Default::default() }
    }

    /// Tell the script of graphics settings it has not seen yet.
    fn follow_graphics(&mut self, scene: &mut Scene, entity: &mut Entity) {
        if self.graphics.changed().is_some() {
            self.script.on_graphics_changed(scene, entity);
        }
    }

    pub fn script(&self) -> &S {
        &self.script
    }

    pub fn script_mut(&mut self) -> &mut S {
        &mut self.script
    }

    fn parameters(&self) -> ScriptData {
        ScriptData::of(&self.script.data())
    }
}

impl<S: Script> Component for ScriptComponent<S> {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.follow_graphics(scene, entity);
        self.script.update(scene, entity, delta_time);
    }

    fn raycast(&self, transform: &Matrix4<f32>, origin: Point3<f32>, dir: Vector3<f32>) -> Option<f32> {
        self.script.raycast(transform, origin, dir)
    }

    fn update_in_editor(&mut self, scene: &mut Scene, entity: &mut Entity, delta_time: f64) {
        self.follow_graphics(scene, entity);
        self.script.update_in_editor(scene, entity, delta_time);
    }

    fn render(
        &self,
        scene: &Scene,
        entity: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // The ground map holds the terrain alone.
        if crate::core::renderer::weather::exposure::in_ground_pass() {
            return;
        }
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

    fn render_overlay(&self, scene: &Scene, entity: &Entity) {
        self.script.render_overlay(scene, entity);
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
        parameters::properties(&to_value(&self.script.data()), S::PARAMETERS)
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let Some(data) = self.parameters().with_parameter(name, value) else {
            return;
        };
        match ron::from_str::<S::Data>(data.as_str()) {
            Ok(data) => self.script.apply(data),
            Err(e) => log::warn!("script {}: cannot set {name}: {e}", S::NAME),
        }
    }
}

impl Entity {
    /// This entity's own script of type `S`, if it has one.
    pub fn script<S: Script>(&self) -> Option<&S> {
        self.get_own_component::<ScriptComponent<S>>()
            .map(ScriptComponent::script)
    }

    /// This entity's own script of type `S`, to change it.
    pub fn script_mut<S: Script>(&mut self) -> Option<&mut S> {
        self.components_mut()
            .iter_mut()
            .find_map(|c| c.as_any_mut().downcast_mut::<ScriptComponent<S>>())
            .map(ScriptComponent::script_mut)
    }
}

impl Scene {
    /// Every script of type `S` in the scene, at any depth, with the entity
    /// it is on. The entity whose update is running at the moment is not
    /// in the scene and is left out: a script sees the others, not itself.
    pub fn scripts<S: Script>(&self) -> Vec<(&Entity, &S)> {
        fn walk<'a, S: Script>(entity: &'a Entity, found: &mut Vec<(&'a Entity, &'a S)>) {
            if let Some(script) = entity.script::<S>() {
                found.push((entity, script));
            }
            for child in entity.get_children() {
                walk(child, found);
            }
        }
        let mut found = Vec::new();
        for entity in self.get_entities() {
            walk(entity, &mut found);
        }
        found
    }

    /// The script of type `S` on the entity `id`, to change it.
    pub fn script_mut<S: Script>(&mut self, id: &EntityHandle) -> Option<&mut S> {
        self.get_entity_mut(id)?.script_mut::<S>()
    }

    /// The first entity named `name`, at any depth.
    pub fn find_entity(&self, name: &str) -> Option<&Entity> {
        fn walk<'a>(entity: &'a Entity, name: &str) -> Option<&'a Entity> {
            if entity.get_name() == name {
                return Some(entity);
            }
            entity.get_children().iter().find_map(|child| walk(child, name))
        }
        self.get_entities().iter().find_map(|entity| walk(entity, name))
    }

    /// The first entity named `name`, at any depth, to change it.
    pub fn find_entity_mut(&mut self, name: &str) -> Option<&mut Entity> {
        fn walk<'a>(entity: &'a mut Entity, name: &str) -> Option<&'a mut Entity> {
            if entity.get_name() == name {
                return Some(entity);
            }
            entity
                .get_children_mut()
                .iter_mut()
                .find_map(|child| walk(child, name))
        }
        self.get_entities_mut()
            .iter_mut()
            .find_map(|entity| walk(entity, name))
    }
}

/// A script's parameters as RON text, as serde writes the script's data
/// (`(speed: 2.0, mode: Fast)`); `()` for its defaults. Kept as text, not
/// as a `ron::Value`: a value forgets the names of enum variants (`Fast`
/// would come back as `()`). In a scene file it is written as it is.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptData(String);

impl ScriptData {
    /// `data` as RON text.
    pub fn of<T: Serialize>(data: &T) -> Self {
        ron::to_string(data).map(Self).unwrap_or_default()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// No parameters given: the script's defaults.
    pub fn is_unit(&self) -> bool {
        matches!(self.0.trim(), "" | "()")
    }

    /// The data with the parameter shown as `name` (see `parameters`) set
    /// to `value`. Every other field is kept as it is written, enum
    /// variants too. `None` if there is no such parameter.
    fn with_parameter(&self, name: &str, value: PropertyValue) -> Option<ScriptData> {
        let mut fields = ron::from_str::<Fields>(&self.0).ok()?.0;
        let changed = fields.iter_mut().any(|(key, raw)| {
            let Ok(old) = raw.into_rust::<Value>() else {
                return false;
            };
            let mut field = Value::Map([(Value::String(key.clone()), old)].into_iter().collect());
            if !parameters::set(&mut field, name, value.clone()) {
                return false;
            }
            let Value::Map(field) = field else {
                return false;
            };
            let Some(new) = field.values().next() else {
                return false;
            };
            // A value writes every list as `[..]`; a tuple or an array
            // (a vector, a colour) is read from `(..)` only.
            let text = match new {
                Value::Seq(items) if raw.get_ron().trim_start().starts_with('(') => items
                    .iter()
                    .map(ron::to_string)
                    .collect::<Result<Vec<_>, _>>()
                    .map(|items| format!("({})", items.join(","))),
                new => ron::to_string(new),
            };
            match text.ok().and_then(|text| RawValue::from_boxed_ron(text.into()).ok()) {
                Some(new) => {
                    *raw = new;
                    true
                }
                None => false,
            }
        });
        if !changed {
            return None;
        }
        let fields: Vec<String> = fields
            .iter()
            .map(|(key, raw)| format!("{key}:{}", raw.get_ron()))
            .collect();
        Some(ScriptData(format!("({})", fields.join(","))))
    }
}

impl Default for ScriptData {
    fn default() -> Self {
        Self("()".to_string())
    }
}

impl Serialize for ScriptData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RawValue::from_ron(&self.0)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ScriptData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        Ok(Self(raw.get_ron().to_string()))
    }
}

/// The fields of a struct written in RON, each as it is written.
struct Fields(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Fields;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a struct")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Fields, A::Error> {
                let mut fields = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    fields.push((key, map.next_value()?));
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

type Create = fn(&ScriptData) -> Result<Box<dyn Component>, String>;

struct Entry {
    create: Create,
    /// The script's type, with its module, to tell two scripts of one name
    /// apart.
    type_name: &'static str,
}

/// The scripts a game offers, by name, and its worlds (see
/// `terrain::source::TerrainGenerator`).
#[derive(Default)]
pub struct ScriptRegistry {
    scripts: BTreeMap<String, Entry>,
    terrains: BTreeMap<String, GeneratorEntry>,
}

impl ScriptRegistry {
    pub fn register<S: Script>(&mut self) {
        fn create<S: Script>(data: &ScriptData) -> Result<Box<dyn Component>, String> {
            // No data at all (`()`): the script's defaults.
            let data: S::Data = if data.is_unit() {
                S::Data::default()
            } else {
                ron::from_str(data.as_str())
                    .or_else(|e| {
                        // Scene files from before wrote the data as a map
                        // (`{"speed": 2.0}`), which only a value reads as
                        // a struct.
                        ron::from_str::<Value>(data.as_str())
                            .ok()
                            .and_then(|value| value.into_rust().ok())
                            .ok_or(e)
                    })
                    .map_err(|e| format!("invalid parameters for script '{}': {e}", S::NAME))?
            };
            Ok(Box::new(ScriptComponent::new(S::from_data(data))))
        }
        let type_name = std::any::type_name::<S>();
        // Scenes save a script by its struct's name alone: a second script
        // of that name would silently take over the first one's entities.
        if let Some(first) = self.scripts.get(S::NAME) {
            if first.type_name != type_name {
                log::error!(
                    "two scripts are named '{}' ({} and {type_name}); only the first is used. \
                     Rename one of them.",
                    S::NAME,
                    first.type_name,
                );
            }
            return;
        }
        self.scripts.insert(
            S::NAME.to_string(),
            Entry {
                create: create::<S>,
                type_name,
            },
        );
    }

    /// Offer the world `G` to scenes (`Terrain(generator: G::NAME)`).
    pub fn register_terrain<G: TerrainGenerator>(&mut self) {
        self.terrains.insert(G::NAME.to_string(), GeneratorEntry::of::<G>());
    }

    /// `register_terrain`, with the hash of the scripts that make the
    /// world (see `GeneratorEntry::fingerprint`).
    pub fn register_terrain_hashed<G: TerrainGenerator>(&mut self, source_hash: u64) {
        self.terrains.insert(G::NAME.to_string(), GeneratorEntry::of_hashed::<G>(source_hash));
    }

    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    /// How many worlds it offers.
    pub fn terrain_count(&self) -> usize {
        self.terrains.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }
}

static REGISTRY: RwLock<ScriptRegistry> = RwLock::new(ScriptRegistry {
    scripts: BTreeMap::new(),
    terrains: BTreeMap::new(),
});

/// Make the scripts of `registry` the ones scenes are built with,
/// replacing all before. What the scripts before registered with
/// `on_unload` is run now.
pub fn install(registry: ScriptRegistry) {
    *REGISTRY.write().unwrap() = registry;
    crate::terrain::generators::changed();
    run_unload();
}

static UNLOAD: Mutex<Vec<fn()>> = Mutex::new(Vec::new());

/// Run `release` when the scripts are reloaded, on the thread that loads
/// them (the GL thread in the editor), for the caches a script keeps in
/// statics or thread-locals: plans of a whole world, models, textures.
///
/// An old script library stays loaded (see the module's docs), and with it
/// every static of it: without this, each reload keeps the caches of the
/// scripts before it for good. Registering the same function again does
/// nothing; one registered again after its run (old code still running)
/// runs on the next reload.
pub fn on_unload(release: fn()) {
    let mut unload = UNLOAD.lock().unwrap_or_else(|e| e.into_inner());
    if !unload.iter().any(|&f| f as usize == release as usize) {
        unload.push(release);
    }
}

fn run_unload() {
    // Taken first: a release may register again (or drop something that
    // does), which locks the list.
    let unload = std::mem::take(&mut *UNLOAD.lock().unwrap_or_else(|e| e.into_inner()));
    for release in unload {
        release();
    }
}

/// The world named `name` the scripts offer.
pub fn terrain_generator(name: &str) -> Option<GeneratorEntry> {
    REGISTRY.read().unwrap().terrains.get(name).cloned()
}

/// The names of the worlds the scripts offer, sorted.
pub fn terrain_generator_names() -> Vec<String> {
    REGISTRY.read().unwrap().terrains.keys().cloned().collect()
}

/// A new component of the script `name`, from its saved parameters.
pub fn create(name: &str, data: &ScriptData) -> Result<Box<dyn Component>, String> {
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

pub fn to_value<T: Serialize>(data: &T) -> Value {
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
        let fresh = create("Spin", &ScriptData::default()).unwrap();
        assert_eq!(
            fresh.to_data(),
            ScriptComponent::new(Spin::default()).to_data()
        );
        assert!(create("Nope", &ScriptData::default()).is_err());
        install(ScriptRegistry::default());
    }

    #[test]
    fn scripts_are_found_on_other_entities_by_type_and_name() {
        // No GL in tests.
        crate::core::headless::enable();
        let mut scene = Scene::new();
        let mut fan = Entity::new("fan");
        fan.add_component(ScriptComponent::new(Spin {
            speed: 1.0,
            ..Default::default()
        }));
        let mut wheel = Entity::new("wheel");
        wheel.add_component(ScriptComponent::new(Spin {
            speed: 2.0,
            ..Default::default()
        }));
        let wheel_id = wheel.id;
        fan.add_child(wheel);
        scene.add_entity(fan);
        scene.add_entity(Entity::new("plain"));

        let speeds: Vec<f32> = scene.scripts::<Spin>().iter().map(|(_, s)| s.speed).collect();
        assert_eq!(speeds, [1.0, 2.0]);
        scene.script_mut::<Spin>(&wheel_id).unwrap().speed = 5.0;
        assert_eq!(scene.find_entity("wheel").unwrap().script::<Spin>().unwrap().speed, 5.0);
        assert!(scene.find_entity("plain").unwrap().script::<Spin>().is_none());
        assert!(scene.find_entity("nobody").is_none());
        scene.find_entity_mut("plain").unwrap().set_name("renamed".into());
        assert!(scene.find_entity("renamed").is_some());
    }

    static RELEASED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn release() {
        RELEASED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn what_the_scripts_registered_to_unload_runs_once_on_the_next_install() {
        use std::sync::atomic::Ordering;
        let _guard = TEST_REGISTRY.lock().unwrap();
        on_unload(release);
        on_unload(release);
        assert_eq!(RELEASED.load(Ordering::SeqCst), 0);
        install(registry());
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1);
        // Not again: the caches it empties were not filled since.
        install(ScriptRegistry::default());
        assert_eq!(RELEASED.load(Ordering::SeqCst), 1);
    }

    #[derive(Clone, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    pub enum Mode {
        #[default]
        Slow,
        Fast,
        Custom(f32),
    }

    #[derive(Script, Default)]
    pub struct Gear {
        pub mode: Mode,
        pub speed: f32,
        pub spare: Mode,
    }

    impl Behaviour for Gear {}

    #[test]
    fn enum_parameters_survive_saving_loading_and_editing() {
        let _guard = TEST_REGISTRY.lock().unwrap();
        let mut registry = ScriptRegistry::default();
        registry.register::<Gear>();
        install(registry);
        let saved = ScriptComponent::new(Gear {
            mode: Mode::Fast,
            speed: 1.0,
            spare: Mode::Custom(2.5),
        })
        .to_data()
        .unwrap();
        // Through a scene file's text, pretty as scenes are written.
        let text = ron::ser::to_string_pretty(&saved, ron::ser::PrettyConfig::default()).unwrap();
        let loaded: ComponentData = ron::from_str(&text).unwrap();
        let ComponentData::Script { name, data } = &loaded else {
            panic!("not script data: {loaded:?}");
        };
        let mut rebuilt = create(name, data).unwrap();
        assert_eq!(rebuilt.to_data(), Some(saved));

        // Editing another parameter keeps the enums.
        rebuilt.set_property("speed", PropertyValue::Float(4.0));
        let gear = rebuilt
            .as_any()
            .downcast_ref::<ScriptComponent<Gear>>()
            .unwrap()
            .script();
        assert_eq!((gear.mode.clone(), gear.speed, gear.spare.clone()), (Mode::Fast, 4.0, Mode::Custom(2.5)));
        install(ScriptRegistry::default());
    }

    #[test]
    fn scene_files_from_before_keep_loading_their_scripts() {
        let _guard = TEST_REGISTRY.lock().unwrap();
        install(registry());
        let text = "Script(name: \"Spin\", data: {\n    \"speed\": 2.0,\n    \"axis\": (0.0, 1.0, 0.0),\n})";
        let data: ComponentData = ron::from_str(text).unwrap();
        let ComponentData::Script { name, data } = &data else {
            panic!("not script data");
        };
        let spin = create(name, data).unwrap();
        let spin = spin.as_any().downcast_ref::<ScriptComponent<Spin>>().unwrap().script();
        assert_eq!((spin.speed, spin.axis), (2.0, [0.0, 1.0, 0.0]));
        // Without data: its defaults.
        let data: ComponentData = ron::from_str("Script(name: \"Spin\")").unwrap();
        let ComponentData::Script { name, data } = &data else {
            panic!("not script data");
        };
        assert!(create(name, data).is_ok());
        install(ScriptRegistry::default());
    }

    mod other {
        use super::super::Behaviour;
        use crate::Script;

        /// A second script named `Spin`, in another module.
        #[derive(Script, Default)]
        pub struct Spin {
            pub other: bool,
        }

        impl Behaviour for Spin {}
    }

    #[test]
    fn a_second_script_of_the_same_name_does_not_replace_the_first() {
        let mut registry = registry();
        registry.register::<other::Spin>();
        assert_eq!(registry.len(), 1);
        let entry = registry.scripts.get("Spin").unwrap();
        assert_eq!(entry.type_name, std::any::type_name::<Spin>());
    }

    /// Tests that install a registry take turns.
    static TEST_REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
