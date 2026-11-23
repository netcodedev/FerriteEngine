//! Graphics settings: what a frame may cost on this machine (view
//! distance, shadow resolution, MSAA, ...), apart from how a scene should
//! look (which stays in the scene). See docs/graphics-settings.md.
//!
//! There is no fixed list: a registry of [`SettingDef`]s, the engine's own
//! (`engine_definitions`) plus the project's (its `graphics` section).
//! Most user-facing settings are a [`Level`] per group ("Schatten: Hoch");
//! a preset sets them all. The engine's internal values ("knobs": shadow
//! map size, LOD factors, ...) and values in assets are [`Param`]s chosen
//! by a level setting, so what "Low" means is data the project can change.
//!
//! The settings are resolved into an [`Resolved`] snapshot with a
//! generation that goes up on every change; systems compare it once per
//! frame and apply what changed.

mod defs;
mod param;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    cell::Cell,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, RwLock,
    },
};

use serde::{Deserialize, Serialize};

pub use defs::{engine_definitions, engine_knobs};
pub use param::{Param, ParamScalar};

/// A quality level, and a preset (which puts every level setting there).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Level {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Low, Level::Medium, Level::High, Level::Ultra];

    pub fn name(self) -> &'static str {
        match self {
            Level::Low => "low",
            Level::Medium => "medium",
            Level::High => "high",
            Level::Ultra => "ultra",
        }
    }

    /// The German label menus show.
    pub fn label(self) -> &'static str {
        match self {
            Level::Low => "Niedrig",
            Level::Medium => "Mittel",
            Level::High => "Hoch",
            Level::Ultra => "Ultra",
        }
    }

    pub fn from_name(name: &str) -> Option<Level> {
        let name = name.trim().to_lowercase();
        Level::ALL
            .into_iter()
            .find(|level| level.name() == name || level.label().to_lowercase() == name)
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One value per level.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PerLevel<T> {
    pub low: T,
    pub medium: T,
    pub high: T,
    pub ultra: T,
}

impl<T> PerLevel<T> {
    pub const fn new(low: T, medium: T, high: T, ultra: T) -> Self {
        Self { low, medium, high, ultra }
    }

    pub fn get(&self, level: Level) -> &T {
        match level {
            Level::Low => &self.low,
            Level::Medium => &self.medium,
            Level::High => &self.high,
            Level::Ultra => &self.ultra,
        }
    }

    pub fn get_mut(&mut self, level: Level) -> &mut T {
        match level {
            Level::Low => &mut self.low,
            Level::Medium => &mut self.medium,
            Level::High => &mut self.high,
            Level::Ultra => &mut self.ultra,
        }
    }
}

impl<T: Clone> PerLevel<T> {
    pub fn all(value: T) -> Self {
        Self { low: value.clone(), medium: value.clone(), high: value.clone(), ultra: value }
    }
}

/// What kind of value a setting holds, with its value per preset.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SettingKind {
    /// Low / Medium / High / Ultra: at a preset's level.
    Level,
    Toggle { per_preset: PerLevel<bool> },
    Number {
        min: f32,
        max: f32,
        #[serde(default)]
        step: f32,
        #[serde(default)]
        unit: String,
        per_preset: PerLevel<f32>,
    },
    Choice { options: Vec<String>, per_preset: PerLevel<String> },
}

/// A setting's value.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SettingValue {
    Level(Level),
    Toggle(bool),
    Number(f32),
    Choice(String),
}

impl fmt::Display for SettingValue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SettingValue::Level(level) => write!(f, "{level}"),
            SettingValue::Toggle(on) => f.write_str(if *on { "an" } else { "aus" }),
            SettingValue::Number(n) => write!(f, "{n}"),
            SettingValue::Choice(c) => f.write_str(c),
        }
    }
}

impl SettingValue {
    /// From JSON as MCP sends it: a level or option name, a bool, a number.
    pub fn from_json(kind: &SettingKind, value: &serde_json::Value) -> Option<SettingValue> {
        match kind {
            SettingKind::Level => value.as_str().and_then(Level::from_name).map(SettingValue::Level),
            SettingKind::Toggle { .. } => value.as_bool().map(SettingValue::Toggle),
            SettingKind::Number { .. } => value.as_f64().map(|n| SettingValue::Number(n as f32)),
            SettingKind::Choice { .. } => value.as_str().map(|s| SettingValue::Choice(s.to_string())),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            SettingValue::Level(level) => level.name().into(),
            SettingValue::Toggle(on) => (*on).into(),
            SettingValue::Number(n) => (*n as f64).into(),
            SettingValue::Choice(c) => c.clone().into(),
        }
    }
}

/// A setting players can choose.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SettingDef {
    /// Its name in files, scripts and MCP: `"shadows"`, `"demo.blood"`.
    pub key: String,
    pub label: String,
    /// The heading it is listed under in menus.
    pub group: String,
    pub kind: SettingKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl SettingDef {
    /// Its value at preset `preset`.
    pub fn at_preset(&self, preset: Level) -> SettingValue {
        match &self.kind {
            SettingKind::Level => SettingValue::Level(preset),
            SettingKind::Toggle { per_preset } => SettingValue::Toggle(*per_preset.get(preset)),
            SettingKind::Number { per_preset, .. } => SettingValue::Number(*per_preset.get(preset)),
            SettingKind::Choice { per_preset, .. } => SettingValue::Choice(per_preset.get(preset).clone()),
        }
    }

    /// `value` made valid for this setting (a number clamped into its
    /// range), or why it cannot be.
    pub fn check(&self, value: SettingValue) -> Result<SettingValue, String> {
        match (&self.kind, value) {
            (SettingKind::Level, value @ SettingValue::Level(_)) => Ok(value),
            (SettingKind::Toggle { .. }, value @ SettingValue::Toggle(_)) => Ok(value),
            (SettingKind::Number { min, max, .. }, SettingValue::Number(n)) if n.is_finite() => {
                Ok(SettingValue::Number(n.clamp(*min, *max)))
            }
            (SettingKind::Choice { options, .. }, SettingValue::Choice(c)) => {
                if options.contains(&c) {
                    Ok(SettingValue::Choice(c))
                } else {
                    Err(format!("{}: no option {c:?} (one of {})", self.key, options.join(", ")))
                }
            }
            (_, value) => Err(format!("{}: {value:?} is not a value for it", self.key)),
        }
    }
}

/// The project's part (its `.fp` file's `graphics` section).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ProjectGraphics {
    /// The preset a player starts with.
    pub default_preset: Level,
    /// The project's own settings; one with an engine setting's key
    /// replaces it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingDef>,
    /// Engine settings the project does not offer (at their value for
    /// the preset; not in menus).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    /// Engine knobs with other values per level.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub knobs: BTreeMap<String, Param<f32>>,
}

impl ProjectGraphics {
    pub fn is_default(&self) -> bool {
        *self == ProjectGraphics::default()
    }
}

/// What a player chose (their `graphics.ron`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct PlayerSettings {
    /// The last preset picked.
    pub preset: Option<Level>,
    /// Settings changed by hand since, by key. Keys this build does not
    /// know are kept.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub changed: BTreeMap<String, SettingValue>,
}

/// The engine's knobs, resolved, for code that reads them every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineKnobs {
    pub msaa: u32,
    pub resolution_scale: f32,
    pub vsync: bool,
    /// Frames per second at most; 0: no cap.
    pub fps_cap: f32,
    pub anisotropy: u8,
    pub load_distance: f32,
    pub detail_distance: f32,
    pub terrain_lod_factor: f32,
    pub shadow_map_size: u32,
    pub shadow_distance: f32,
    pub shadow_cascades: usize,
    pub stagger_second_cascade: bool,
    pub point_shadow_resolution: u32,
    pub point_shadow_lights: usize,
    pub scatter_lod_factor: f32,
    pub detail_distance_factor: f32,
    pub detail_density: f32,
    pub ssao_samples: u32,
    /// Temporal anti-aliasing instead of MSAA (`renderer::taa`).
    pub taa: bool,
    /// Sun shadows ray traced instead of the cascades, where the device
    /// can (`renderer::raytracing`).
    pub ray_traced_shadows: bool,
    /// Detail scatter (grass) near the camera casts ray traced shadows too.
    pub ray_traced_grass: bool,
    /// The leaves' ray traced shadows follow the wind (`renderer::raytracing`).
    pub ray_traced_wind: bool,
    /// Shadows of point and spot lights ray traced (ReSTIR) instead of
    /// their cube maps, where the device can (`renderer::raytracing`).
    pub ray_traced_lights: bool,
    /// Which ambient occlusion (`effects.ao`).
    pub ambient_occlusion: AoMode,
    /// Which reflections (`effects.reflections`).
    pub reflections: ReflectionMode,
    pub bloom: bool,
    pub cloud_downscale: i32,
    pub precipitation: f32,
}

/// The ambient occlusion the settings ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AoMode {
    Off,
    /// Screen space (`renderer::ssao`).
    Ssao,
    /// Ray traced where the device can (`renderer::raytracing`), SSAO
    /// elsewhere.
    RayTraced,
}

/// The reflections the settings ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReflectionMode {
    /// Every surface reflects the sky alone.
    Sky,
    /// Water marches the scene in screen space (as before settings).
    Ssr,
    /// Glossy surfaces and water trace the scene where the device can
    /// (`renderer::raytracing`), SSR elsewhere.
    RayTraced,
}

/// The settings, worked out: every setting's value and every knob.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// Goes up on every change.
    pub generation: u64,
    /// The preset in use (the last picked, or the project's).
    pub preset: Level,
    /// Some settings differ from the preset.
    pub custom: bool,
    /// Every setting is at this level's preset value (editor preview).
    pub preview: Option<Level>,
    values: HashMap<String, SettingValue>,
    knobs: HashMap<String, f32>,
    pub engine: EngineKnobs,
}

impl Resolved {
    pub fn value(&self, key: &str) -> Option<&SettingValue> {
        self.values.get(key)
    }

    /// The level of setting `key`; High when it is no level setting.
    pub fn level(&self, key: &str) -> Level {
        match self.values.get(key) {
            Some(SettingValue::Level(level)) => *level,
            _ => Level::High,
        }
    }

    pub fn toggle(&self, key: &str) -> Option<bool> {
        match self.values.get(key) {
            Some(SettingValue::Toggle(on)) => Some(*on),
            _ => None,
        }
    }

    pub fn number(&self, key: &str) -> Option<f32> {
        match self.values.get(key) {
            Some(SettingValue::Number(n)) => Some(*n),
            _ => None,
        }
    }

    pub fn choice(&self, key: &str) -> Option<&str> {
        match self.values.get(key) {
            Some(SettingValue::Choice(c)) => Some(c),
            _ => None,
        }
    }

    pub fn knob(&self, key: &str) -> Option<f32> {
        self.knobs.get(key).copied()
    }
}

/// The settings of one game: definitions, knobs and the player's choice.
/// The engine keeps one (see [`current`]); tests make their own.
#[derive(Clone, Debug)]
pub struct Graphics {
    defs: Vec<SettingDef>,
    hidden: Vec<String>,
    knobs: BTreeMap<String, Param<f32>>,
    default_preset: Level,
    player: PlayerSettings,
    preview: Option<Level>,
    generation: u64,
    resolved: Arc<Resolved>,
    /// Changed since it was last saved.
    unsaved: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Graphics::new(&ProjectGraphics::default(), PlayerSettings::default())
    }
}

impl Graphics {
    pub fn new(project: &ProjectGraphics, player: PlayerSettings) -> Self {
        let mut defs = engine_definitions();
        for def in &project.settings {
            match defs.iter_mut().find(|d| d.key == def.key) {
                Some(existing) => *existing = def.clone(),
                None => defs.push(def.clone()),
            }
        }
        let mut knobs = engine_knobs();
        for (key, param) in &project.knobs {
            if !knobs.contains_key(key) {
                log::warn!("graphics: the project sets knob {key:?}, which the engine does not have");
            }
            knobs.insert(key.clone(), param.clone());
        }
        let mut graphics = Graphics {
            defs,
            hidden: project.hidden.clone(),
            knobs,
            default_preset: project.default_preset,
            player,
            preview: None,
            generation: 0,
            resolved: Arc::new(placeholder()),
            unsaved: false,
        };
        graphics.check_player();
        graphics.resolve();
        graphics
    }

    /// Drop (with a warning) what the player's file holds that is no
    /// longer valid; unknown keys stay.
    fn check_player(&mut self) {
        let mut fixed = Vec::new();
        for (key, value) in &self.player.changed {
            let Some(def) = self.defs.iter().find(|d| &d.key == key) else { continue };
            match def.check(value.clone()) {
                Ok(checked) if checked != *value => {
                    log::warn!("graphics: {key} = {value} is out of range, using {checked}");
                    fixed.push((key.clone(), Some(checked)));
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!("graphics: {e}; using the preset's value");
                    fixed.push((key.clone(), None));
                }
            }
        }
        for (key, value) in fixed {
            match value {
                Some(value) => self.player.changed.insert(key, value),
                None => self.player.changed.remove(&key),
            };
        }
    }

    pub fn preset(&self) -> Level {
        self.player.preset.unwrap_or(self.default_preset)
    }

    pub fn resolved(&self) -> Arc<Resolved> {
        self.resolved.clone()
    }

    pub fn player(&self) -> &PlayerSettings {
        &self.player
    }

    pub fn unsaved(&self) -> bool {
        self.unsaved
    }

    /// Every setting, the hidden ones included.
    pub fn all_definitions(&self) -> &[SettingDef] {
        &self.defs
    }

    /// The settings menus offer.
    pub fn definitions(&self) -> Vec<SettingDef> {
        self.defs.iter().filter(|d| !self.hidden.contains(&d.key)).cloned().collect()
    }

    pub fn definition(&self, key: &str) -> Option<&SettingDef> {
        self.defs.iter().find(|d| d.key == key)
    }

    pub fn knob_params(&self) -> &BTreeMap<String, Param<f32>> {
        &self.knobs
    }

    /// Pick a preset: every setting goes to its value there.
    pub fn set_preset(&mut self, preset: Level) {
        self.player.preset = Some(preset);
        self.player.changed.retain(|key, _| !self.defs.iter().any(|d| &d.key == key));
        self.changed();
    }

    /// Set one setting (the preset becomes custom unless it is the
    /// preset's own value).
    pub fn set(&mut self, key: &str, value: SettingValue) -> Result<(), String> {
        let def = self.definition(key).ok_or_else(|| format!("no graphics setting {key:?}"))?;
        let value = def.check(value)?;
        if def.at_preset(self.preset()) == value {
            self.player.changed.remove(key);
        } else {
            self.player.changed.insert(key.to_string(), value);
        }
        self.changed();
        Ok(())
    }

    /// Back to the preset's value.
    pub fn reset(&mut self, key: &str) {
        if self.player.changed.remove(key).is_some() {
            self.changed();
        }
    }

    /// Show everything at `level`'s preset without changing what the
    /// player chose (the editor's preview); `None` ends it.
    pub fn set_preview(&mut self, level: Option<Level>) {
        if self.preview != level {
            self.preview = level;
            self.generation += 1;
            self.resolve();
        }
    }

    fn changed(&mut self) {
        self.unsaved = true;
        self.generation += 1;
        self.resolve();
    }

    pub fn mark_saved(&mut self) {
        self.unsaved = false;
    }

    fn resolve(&mut self) {
        let preset = self.preview.unwrap_or_else(|| self.preset());
        let mut values = HashMap::new();
        for def in &self.defs {
            let changed = match self.preview {
                Some(_) => None,
                None => self.player.changed.get(&def.key).cloned(),
            };
            values.insert(def.key.clone(), changed.unwrap_or_else(|| def.at_preset(preset)));
        }
        let mut resolved = Resolved {
            generation: self.generation,
            preset: self.preset(),
            custom: self.defs.iter().any(|d| self.player.changed.contains_key(&d.key)),
            preview: self.preview,
            values,
            knobs: HashMap::new(),
            engine: placeholder().engine,
        };
        let knobs = self.knobs.iter().map(|(key, param)| (key.clone(), param.resolve(&resolved))).collect();
        resolved.knobs = knobs;
        resolved.engine = defs::engine_values(&resolved);
        self.resolved = Arc::new(resolved);
    }
}

fn placeholder() -> Resolved {
    Resolved {
        generation: 0,
        preset: Level::High,
        custom: false,
        preview: None,
        values: HashMap::new(),
        knobs: HashMap::new(),
        engine: defs::TODAY,
    }
}

// ---------------------------------------------------------------------
// The engine's settings.

struct Global {
    graphics: Graphics,
    /// Where the player's choice is saved; none in tests and the server.
    file: Option<PathBuf>,
    /// The project's part, as given (the editor edits it).
    project: ProjectGraphics,
}

static GLOBAL: LazyLock<RwLock<Global>> = LazyLock::new(|| {
    RwLock::new(Global { graphics: Graphics::default(), file: None, project: ProjectGraphics::default() })
});

fn read<R>(f: impl FnOnce(&Global) -> R) -> R {
    f(&GLOBAL.read().unwrap_or_else(|e| e.into_inner()))
}

fn write<R>(f: impl FnOnce(&mut Global) -> R) -> R {
    let mut global = GLOBAL.write().unwrap_or_else(|e| e.into_inner());
    let result = f(&mut global);
    PUBLISHED.store(global.graphics.generation, Ordering::Release);
    result
}

/// The generation [`engine`] compares its per-thread copy with.
static PUBLISHED: AtomicU64 = AtomicU64::new(u64::MAX);

thread_local! {
    static ENGINE: Cell<Option<(u64, EngineKnobs)>> = const { Cell::new(None) };
}

/// The engine's knobs now: cheap enough to call per chunk or per draw (a
/// copy per thread, renewed when the settings change).
pub fn engine() -> EngineKnobs {
    let published = PUBLISHED.load(Ordering::Acquire);
    ENGINE.with(|cache| match cache.get() {
        Some((generation, knobs)) if generation == published => knobs,
        _ => {
            let resolved = current();
            cache.set(Some((resolved.generation, resolved.engine)));
            resolved.engine
        }
    })
}

/// Where a game's player settings live: `$XDG_CONFIG_HOME/ferrite/<game>/
/// graphics.ron` (`~/.config` without it; `%APPDATA%` on Windows).
pub fn player_file_path(project_name: &str) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    let name: String = project_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let name = if name.trim().is_empty() { "game".to_string() } else { name };
    Some(base.join("ferrite").join(name).join("graphics.ron"))
}

/// Read a player's settings file; a missing file is the defaults, a
/// broken one too (with a warning).
pub fn read_player_file(path: &Path) -> PlayerSettings {
    match std::fs::read_to_string(path) {
        Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
            log::warn!("graphics: {}: {e}; using the defaults", path.display());
            PlayerSettings::default()
        }),
        Err(_) => PlayerSettings::default(),
    }
}

/// Start with the project's part and the player's file at `file`
/// (`None`: nothing is saved).
pub fn init(project: &ProjectGraphics, file: Option<PathBuf>) {
    let player = file.as_deref().map(read_player_file).unwrap_or_default();
    write(|global| {
        let generation = global.graphics.generation + 1;
        global.graphics = Graphics::new(project, player);
        // Systems compare generations: a new start must count as a change.
        global.graphics.generation = generation;
        global.graphics.resolve();
        global.file = file;
        global.project = project.clone();
    });
}

/// Start for the project named `name` with its `graphics` section; the
/// player's file in the user's config folder.
pub fn init_for_project(name: &str, project: &ProjectGraphics) {
    init(project, player_file_path(name));
}

/// The settings now.
pub fn current() -> Arc<Resolved> {
    read(|g| g.graphics.resolved())
}

/// Goes up on every change.
pub fn generation() -> u64 {
    read(|g| g.graphics.generation)
}

/// The settings menus offer.
pub fn definitions() -> Vec<SettingDef> {
    read(|g| g.graphics.definitions())
}

pub fn definition(key: &str) -> Option<SettingDef> {
    read(|g| g.graphics.definition(key).cloned())
}

pub fn preset() -> Level {
    read(|g| g.graphics.preset())
}

pub fn set_preset(preset: Level) {
    write(|g| g.graphics.set_preset(preset));
}

pub fn set(key: &str, value: SettingValue) -> Result<(), String> {
    write(|g| g.graphics.set(key, value))
}

pub fn reset(key: &str) {
    write(|g| g.graphics.reset(key));
}

pub fn set_preview(level: Option<Level>) {
    write(|g| g.graphics.set_preview(level));
}

pub fn preview() -> Option<Level> {
    read(|g| g.graphics.preview)
}

/// The level of setting `key` (High when it is no level setting).
pub fn level(key: &str) -> Level {
    current().level(key)
}

pub fn get_bool(key: &str) -> Option<bool> {
    current().toggle(key)
}

pub fn get_f32(key: &str) -> Option<f32> {
    current().number(key)
}

pub fn get_choice(key: &str) -> Option<String> {
    current().choice(key).map(str::to_string)
}

/// Whether there are changes not yet saved.
pub fn unsaved() -> bool {
    read(|g| g.graphics.unsaved())
}

/// Write the player's choice to their file.
pub fn save() -> Result<(), String> {
    let (file, player) = read(|g| (g.file.clone(), g.graphics.player().clone()));
    let Some(file) = file else { return Ok(()) };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = ron::ser::to_string_pretty(&player, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    super::utils::write_atomically(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    write(|g| g.graphics.mark_saved());
    Ok(())
}

/// The project's part, as the settings were started with.
pub fn project_settings() -> ProjectGraphics {
    read(|g| g.project.clone())
}

/// Use another project part (the editor changed it), keeping the
/// player's choice.
pub fn set_project_settings(project: &ProjectGraphics) {
    write(|global| {
        let player = global.graphics.player().clone();
        let preview = global.graphics.preview;
        let generation = global.graphics.generation + 1;
        let unsaved = global.graphics.unsaved;
        global.graphics = Graphics::new(project, player);
        global.graphics.preview = preview;
        global.graphics.generation = generation;
        global.graphics.unsaved = unsaved;
        global.graphics.resolve();
        global.project = project.clone();
    });
}

/// The knobs as they stand (for the editor's project tab).
pub fn knob_params() -> BTreeMap<String, Param<f32>> {
    read(|g| g.graphics.knob_params().clone())
}

/// Every setting as JSON (MCP): definitions, values, preset, generation.
pub fn to_json() -> serde_json::Value {
    read(|g| {
        let resolved = g.graphics.resolved();
        let settings: Vec<serde_json::Value> = g
            .graphics
            .definitions()
            .into_iter()
            .map(|def| {
                let mut entry = serde_json::to_value(&def).unwrap_or_default();
                if let Some(value) = resolved.value(&def.key) {
                    entry["value"] = value.to_json();
                }
                entry["changed"] = g.graphics.player().changed.contains_key(&def.key).into();
                entry
            })
            .collect();
        let knobs: serde_json::Map<String, serde_json::Value> = resolved
            .knobs
            .iter()
            .map(|(k, v)| (k.clone(), (*v as f64).into()))
            .collect();
        serde_json::json!({
            "preset": g.graphics.preset().name(),
            "custom": resolved.custom,
            "preview": resolved.preview.map(Level::name),
            "generation": resolved.generation,
            "settings": settings,
            "knobs": knobs,
        })
    })
}

/// Carry out an MCP `graphics` request (see `BridgeRequest::Graphics`);
/// returns every setting, as `to_json`.
pub fn apply_request(
    preset: Option<&str>,
    set: Option<&serde_json::Map<String, serde_json::Value>>,
    reset: Option<&[String]>,
    preview: Option<&str>,
    project: Option<&serde_json::Value>,
    save_now: bool,
) -> Result<serde_json::Value, String> {
    if let Some(preset) = preset {
        let level = Level::from_name(preset).ok_or_else(|| format!("no preset {preset:?} (low, medium, high, ultra)"))?;
        set_preset(level);
    }
    for (key, value) in set.into_iter().flatten() {
        let def = definition(key).ok_or_else(|| format!("no graphics setting {key:?}"))?;
        let value = SettingValue::from_json(&def.kind, value)
            .ok_or_else(|| format!("{key}: {value} is not a value for it"))?;
        self::set(key, value)?;
    }
    for key in reset.into_iter().flatten() {
        self::reset(key);
    }
    if let Some(preview) = preview {
        match preview.trim().to_lowercase().as_str() {
            "off" | "none" | "" => set_preview(None),
            name => set_preview(Some(Level::from_name(name).ok_or_else(|| format!("no level {name:?}"))?)),
        }
    }
    if let Some(project) = project {
        let project: ProjectGraphics = serde_json::from_value(project.clone()).map_err(|e| format!("project: {e}"))?;
        set_project_settings(&project);
    }
    if save_now {
        save()?;
    }
    let mut json = to_json();
    json["project"] = serde_json::to_value(project_settings()).unwrap_or_default();
    json["unsaved"] = unsaved().into();
    Ok(json)
}

/// What an inspector's `<name> follows` shows for a value not bound to a
/// setting (see `Param`).
pub const FIXED: &str = "fixed";

/// The settings a `Param` can follow: those with levels.
pub fn level_settings() -> Vec<String> {
    definitions()
        .into_iter()
        .filter(|def| matches!(def.kind, SettingKind::Level))
        .map(|def| def.key)
        .collect()
}

/// Remembers which generation of the settings a system applied last.
#[derive(Debug, Default)]
pub struct Watch {
    seen: Option<u64>,
}

impl Watch {
    /// The settings, when they changed since the last call (always on the
    /// first).
    pub fn changed(&mut self) -> Option<Arc<Resolved>> {
        let resolved = current();
        if self.seen == Some(resolved.generation) {
            return None;
        }
        self.seen = Some(resolved.generation);
        Some(resolved)
    }
}
