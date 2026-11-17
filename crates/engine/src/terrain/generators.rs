//! The worlds scenes can name (see `source::TerrainGenerator`): the
//! engine's own and those the loaded scripts register.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use ron::Value;

use crate::core::{
    entity::component::{Property, PropertyValue},
    script,
};

use super::{
    hills::Hills,
    source::{TerrainGenerator, TerrainSource},
};

/// The world a terrain gets when its scene names none.
pub const DEFAULT: &str = Hills::NAME;

/// A registered world: how to make it from a seed and saved settings.
#[derive(Clone)]
pub struct GeneratorEntry {
    pub name: &'static str,
    create: fn(u64, &Value) -> Result<Arc<dyn TerrainSource>, String>,
    defaults: fn() -> Value,
    properties: fn(&Value) -> Vec<Property>,
    set_property: fn(&mut Value, &str, PropertyValue) -> bool,
}

/// `settings` as `G`'s settings (`()`: the defaults).
fn settings_of<G: TerrainGenerator>(settings: &Value) -> Result<G::Settings, String> {
    match settings {
        Value::Unit => Ok(G::Settings::default()),
        settings => settings
            .clone()
            .into_rust()
            .map_err(|e| format!("invalid settings for the world '{}': {e}", G::NAME)),
    }
}

impl GeneratorEntry {
    pub fn of<G: TerrainGenerator>() -> Self {
        fn create<G: TerrainGenerator>(seed: u64, settings: &Value) -> Result<Arc<dyn TerrainSource>, String> {
            Ok(G::create(seed, settings_of::<G>(settings)?))
        }
        fn defaults<G: TerrainGenerator>() -> Value {
            script::to_value(&G::Settings::default())
        }
        fn properties<G: TerrainGenerator>(settings: &Value) -> Vec<Property> {
            settings_of::<G>(settings).map(|s| G::properties(&s)).unwrap_or_default()
        }
        fn set_property<G: TerrainGenerator>(settings: &mut Value, name: &str, value: PropertyValue) -> bool {
            let Ok(mut typed) = settings_of::<G>(settings) else {
                return false;
            };
            if !G::set_property(&mut typed, name, value) {
                return false;
            }
            *settings = script::to_value(&typed);
            true
        }
        Self {
            name: G::NAME,
            create: create::<G>,
            defaults: defaults::<G>,
            properties: properties::<G>,
            set_property: set_property::<G>,
        }
    }

    /// The world of `seed` with `settings` (`()`: the defaults).
    pub fn create(&self, seed: u64, settings: &Value) -> Result<Arc<dyn TerrainSource>, String> {
        (self.create)(seed, settings)
    }

    /// Its default settings, as saved.
    pub fn defaults(&self) -> Value {
        (self.defaults)()
    }

    /// `settings` (`()`: the defaults) as the inspector shows them.
    pub fn properties(&self, settings: &Value) -> Vec<Property> {
        (self.properties)(settings)
    }

    /// Set the setting shown as `name` in `settings`; returns whether
    /// there was one.
    pub fn set_property(&self, settings: &mut Value, name: &str, value: PropertyValue) -> bool {
        (self.set_property)(settings, name, value)
    }
}

/// The engine's own worlds.
fn builtin() -> Vec<GeneratorEntry> {
    vec![GeneratorEntry::of::<Hills>()]
}

/// The world named `name`: the scripts' first, then the engine's.
pub fn find(name: &str) -> Option<GeneratorEntry> {
    script::terrain_generator(name).or_else(|| builtin().into_iter().find(|entry| entry.name == name))
}

/// The names of every world there is, the engine's first.
pub fn names() -> Vec<String> {
    let mut names: Vec<String> = builtin().iter().map(|entry| entry.name.to_string()).collect();
    for name in script::terrain_generator_names() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

static VERSION: AtomicU64 = AtomicU64::new(0);

/// Changes whenever the worlds that are registered may have (new scripts
/// were loaded): terrains make their world again then.
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

pub(crate) fn changed() {
    VERSION.fetch_add(1, Ordering::Relaxed);
}
