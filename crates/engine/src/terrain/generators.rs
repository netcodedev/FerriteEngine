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
    /// Hash of the scripts that make the world (the generator's module and
    /// every script module it uses; see `crates/scripts/build.rs`); 0 for
    /// the engine's own worlds.
    source_hash: u64,
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
            source_hash: 0,
        }
    }

    /// `of`, for a world made by scripts whose sources hash to `hash`.
    pub fn of_hashed<G: TerrainGenerator>(hash: u64) -> Self {
        Self { source_hash: hash, ..Self::of::<G>() }
    }

    /// What the world of `seed` with `settings` is made of: its name, the
    /// seed, the settings, the generator's code (its scripts' hash, this
    /// engine's world version) and the mesher. Equal fingerprints make
    /// equal worlds, so what was made for one can be kept for the other.
    pub fn fingerprint(&self, seed: u64, settings: &Value) -> u64 {
        // FNV-1a: the same on every run and build (unlike std's hasher's
        // randomised keys or its unspecified algorithm).
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut add = |bytes: &[u8]| {
            for b in bytes {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        };
        add(self.name.as_bytes());
        add(&seed.to_le_bytes());
        let settings = match settings {
            Value::Unit => self.defaults(),
            other => other.clone(),
        };
        add(ron::to_string(&settings).unwrap_or_default().as_bytes());
        add(&self.source_hash.to_le_bytes());
        add(&WORLD_VERSION.to_le_bytes());
        add(std::env::var("FERRITE_TERRAIN_MESHER").unwrap_or_default().as_bytes());
        hash
    }

    /// The world of `seed` with `settings` (`()`: the defaults). While it
    /// is made, `world_cache_dir` names the directory kept for this world.
    pub fn create(&self, seed: u64, settings: &Value) -> Result<Arc<dyn TerrainSource>, String> {
        let dir = cache_dir_for(self.fingerprint(seed, settings));
        *CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir;
        let made = (self.create)(seed, settings);
        *CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()) = None;
        made
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

/// Bumped when engine code that worlds are made with changes what they
/// come out as (noise, meshing): every cached world is made again then.
pub const WORLD_VERSION: u64 = 1;

/// Where worlds keep what they made, below the project (the working
/// directory): one directory per fingerprint.
const CACHE_ROOT: &str = ".ferrite-cache/world";
/// Fingerprints kept besides the one in use (the newest).
const CACHE_KEEP: usize = 4;

static CACHE_DIR: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// The directory kept for the world being made: a generator calls this in
/// `TerrainGenerator::create` (not later, nor on another thread) and may
/// keep there whatever spares it work next time (its plans, say). `None`
/// outside `create`, or where there is no place for it.
pub fn world_cache_dir() -> Option<std::path::PathBuf> {
    CACHE_DIR.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The world's directory, made, with the oldest others beyond
/// `CACHE_KEEP` removed.
fn cache_dir_for(fingerprint: u64) -> Option<std::path::PathBuf> {
    if crate::core::headless::is_headless() && std::env::var_os("FERRITE_WORLD_CACHE").is_none() {
        // Servers and tests make their worlds afresh, unless asked.
        return None;
    }
    let root = std::path::Path::new(CACHE_ROOT);
    let dir = root.join(format!("{fingerprint:016x}"));
    std::fs::create_dir_all(&dir).ok()?;
    // Marks it as used now (the pruning keeps the newest).
    let _ = std::fs::write(dir.join("used"), b"");
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut others: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .flatten()
            .filter(|e| e.path() != dir && e.path().is_dir())
            .map(|e| {
                let used = std::fs::metadata(e.path().join("used")).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                (used, e.path())
            })
            .collect();
        others.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, old) in others.into_iter().skip(CACHE_KEEP) {
            let _ = std::fs::remove_dir_all(old);
        }
    }
    Some(dir)
}

/// Forget what was kept for the world of this fingerprint (it is made
/// afresh next time).
pub fn clear_world_cache(fingerprint: u64) {
    let _ = std::fs::remove_dir_all(std::path::Path::new(CACHE_ROOT).join(format!("{fingerprint:016x}")));
}
