//! Weather: rain and snow, kept with the sky light.
//!
//! The weather has a mode (always clear, rain or snow, or changing on its
//! own) and a state that builds up and wears off over time: how wet the
//! ground is, the puddles on it, how much snow covers it and how deep. The
//! state is saved with the scene, so a snowed-in scene stays snowed in.
//!
//! What the shaders see:
//! - `exposure`: a depth map from straight above (`SkyExposure`), so only
//!   surfaces under open sky get wet or snowed on and no rain falls under a
//!   roof or a tree; and one of the terrain alone, so snow buries what lies
//!   on the ground.
//! - `shelter`: from those two, how open the sky is over the ground,
//!   smoothed, for the snow layer's depth under trees.
//! - `snow`: trails pressed into the snow (`SnowTrails`), from footprints
//!   any component hands to `press_snow`.
//! - `snow_layer`: a real layer of snow on the ground near the camera, with
//!   the trails pressed into it.
//! - `precipitation`: the falling drops and flakes around the camera.
//! - `weather.glsl`: wet surfaces, puddles with rain ripples and snow cover,
//!   used by every lit shader.

pub mod exposure;
pub mod precipitation;
pub mod shelter;
pub mod snow;
pub mod snow_layer;

use std::sync::Mutex;
use std::time::Instant;

use cgmath::{Matrix4, Vector2, Vector3};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use super::{light::light::clamp_finite, shader::Shader, texture::Texture};
use exposure::SkyExposure;
use precipitation::PrecipitationRenderer;
use shelter::ShelterMap;
use snow::SnowTrails;
use snow_layer::SnowLayer;

/// The weather's maps and what is read from them (`skyExposure`,
/// `groundHeight`, `snowTrail`, ...), shared by every lit shader and the
/// snow layer's vertex shader. Declare `isShadowPass` before it.
pub const WEATHER_MAPS_GLSL: &str = include_str!("weather_maps.glsl");

/// Texture unit the sky exposure map is bound to while the scene is shaded.
pub const EXPOSURE_UNIT: u32 = 9;
/// Texture unit the snow trail map is bound to while the scene is shaded.
pub const SNOW_TRAIL_UNIT: u32 = 10;
/// Texture unit the ground map (the exposure map's view of the terrain
/// alone) is bound to while the scene is shaded.
pub const GROUND_UNIT: u32 = 8;
/// Texture unit the smoothed open sky (`ShelterMap`) is bound to while the
/// scene is shaded.
pub const SHELTER_UNIT: u32 = 7;
/// Texture units the snow's scan is bound to while the scene is shaded:
/// colour with roughness, and normal with occlusion.
pub const SNOW_COLOUR_UNIT: u32 = 5;
pub const SNOW_SURFACE_UNIT: u32 = 6;

lazy_static! {
    static ref START: Instant = Instant::now();
    static ref FOOTPRINTS: Mutex<Vec<Footprint>> = Mutex::new(Vec::new());
}

/// Seconds since the weather started; drives ripples and falling drops.
pub fn seconds() -> f32 {
    START.elapsed().as_secs_f32()
}

/// A foot pressed into the snow: where, and which way its toes point.
#[derive(Clone, Copy, Debug)]
pub struct Footprint {
    /// World position on the ground (only x and z matter).
    pub position: Vector3<f32>,
    /// Horizontal heel-to-toe direction.
    pub forward: Vector2<f32>,
    /// Length of the print in metres (heel to toe).
    pub length: f32,
}

/// Most footprints pressed in one step of the trail map; must match
/// `MAX_STAMPS` in `snow_update.glsl`. A frame takes up to four times as
/// many, in several steps.
pub const MAX_FOOTPRINTS: usize = 32;

/// Leave a footprint in the snow, if there is any. Anything that walks can
/// call this every frame its foot is on the ground.
pub fn press_snow(footprint: Footprint) {
    let mut footprints = FOOTPRINTS.lock().unwrap();
    if footprints.len() < MAX_FOOTPRINTS * 4 {
        footprints.push(footprint);
    }
}

/// The footprints left since the last call.
pub(crate) fn take_footprints() -> Vec<Footprint> {
    std::mem::take(&mut *FOOTPRINTS.lock().unwrap())
}

/// What the weather is set to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WeatherMode {
    /// Dry: no rain or snow; what fell before dries and melts away.
    #[default]
    Clear,
    Rain,
    Snow,
    /// Spells of clear weather, rain and snow follow each other.
    Changing,
}

impl WeatherMode {
    pub const ALL: [WeatherMode; 4] = [
        WeatherMode::Clear,
        WeatherMode::Rain,
        WeatherMode::Snow,
        WeatherMode::Changing,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WeatherMode::Clear => "Clear",
            WeatherMode::Rain => "Rain",
            WeatherMode::Snow => "Snow",
            WeatherMode::Changing => "Changing",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}

/// What falls from the sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Precipitation {
    #[default]
    None,
    Rain,
    Snow,
}

/// One stretch of weather: what falls, how hard (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spell {
    kind: Precipitation,
    intensity: f32,
}

/// Seconds the clouds take to gather or clear up.
const OVERCAST_SECONDS: f32 = 25.0;
/// Seconds rain or snow takes to start or stop.
const FALL_SECONDS: f32 = 12.0;
/// Seconds of full rain until everything outside is wet.
const WET_SECONDS: f32 = 20.0;
/// Seconds of full rain on wet ground until the puddles are full.
const PUDDLE_SECONDS: f32 = 90.0;
/// Seconds a wet surface takes to dry in full sun (longer under clouds).
const DRY_SECONDS: f32 = 90.0;
/// Seconds of full snowfall until the ground is white.
const COVER_SECONDS: f32 = 40.0;
/// Metres of snow per second of full snowfall.
const SNOW_RATE: f32 = 0.0015;
/// Deepest the snow gets, in metres.
pub const MAX_SNOW_DEPTH: f32 = 0.4;
/// Seconds snow takes to melt away from full cover in full sun.
const MELT_SECONDS: f32 = 240.0;

/// The weather and what it has left on the ground.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct Weather {
    pub mode: WeatherMode,
    /// How hard it rains or snows in the Rain and Snow modes, 0..1.
    pub intensity: f32,
    /// Changing weather: about how many seconds one spell lasts.
    pub spell_seconds: f32,
    /// How wet surfaces under open sky are, 0..1.
    pub wetness: f32,
    /// How full the puddles are, 0..1.
    pub puddles: f32,
    /// How much of the ground under open sky snow covers, 0..1.
    pub snow_cover: f32,
    /// How deep the snow is, in metres.
    pub snow_depth: f32,
    #[serde(skip)]
    now: Now,
}

/// The GPU side of the weather, kept by the scene.
pub struct WeatherTargets {
    pub exposure: SkyExposure,
    pub shelter: ShelterMap,
    pub trails: SnowTrails,
    pub precipitation: PrecipitationRenderer,
    pub layer: SnowLayer,
    /// The snow's scan from the project's `assets/textures/terrain/snow/`.
    pub snow_textures: Option<[Texture; 2]>,
}

impl WeatherTargets {
    pub fn new() -> Self {
        Self {
            exposure: SkyExposure::new(),
            shelter: ShelterMap::new(),
            trails: SnowTrails::new(),
            precipitation: PrecipitationRenderer::new(),
            layer: SnowLayer::new(),
            snow_textures: crate::terrain::materials::load_snow(),
        }
    }

    /// Bind the snow's scan, if there is one, to its units.
    pub fn bind_snow_textures(&self) {
        if let Some([colour, surface]) = &self.snow_textures {
            colour.bind_to(SNOW_COLOUR_UNIT);
            surface.bind_to(SNOW_SURFACE_UNIT);
        }
    }
}

/// What the sky is doing at the moment (not saved: a loaded scene starts
/// in the steady state of its mode).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Now {
    /// What is falling.
    kind: Precipitation,
    /// How hard it falls, 0..1.
    amount: f32,
    /// How far the clouds have closed in for the weather, 0..1.
    overcast: f32,
    /// How stormy the clouds are, 0..1 (follows the spell's intensity).
    storm: f32,
    /// The changing weather's spell, and the seconds it has left.
    spell: Spell,
    spell_left: f32,
    random: u32,
}

impl Default for Now {
    fn default() -> Self {
        Self {
            kind: Precipitation::None,
            amount: 0.0,
            overcast: 0.0,
            storm: 0.0,
            spell: Spell { kind: Precipitation::None, intensity: 0.0 },
            spell_left: 0.0,
            random: 0x2545_F491,
        }
    }
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            mode: WeatherMode::Clear,
            intensity: 0.7,
            spell_seconds: 150.0,
            wetness: 0.0,
            puddles: 0.0,
            snow_cover: 0.0,
            snow_depth: 0.0,
            now: Now::default(),
        }
    }
}

/// What the lit shaders need to know about the weather.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherShading {
    pub rain: f32,
    pub snow: f32,
    pub wetness: f32,
    pub puddles: f32,
    pub snow_cover: f32,
    pub snow_depth: f32,
}

/// Where the weather maps lie while the scene is shaded.
#[derive(Clone, Copy, Debug)]
pub struct WeatherMaps {
    /// World to the exposure map's clip space, and the metres its depth
    /// range spans.
    pub exposure: Option<(Matrix4<f32>, f32)>,
    /// World x, z of the trail map's corner and the metres it spans.
    pub trails: Option<[f32; 3]>,
    /// Whether the snow's scan (`SNOW_COLOUR_UNIT`, `SNOW_SURFACE_UNIT`) is
    /// bound; without it snow is a plain colour with a fine grain.
    pub snow_textures: bool,
}

fn approach(value: f32, target: f32, step: f32) -> f32 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}

impl Weather {
    pub const INTENSITY_RANGE: (f32, f32) = (0.0, 1.0);
    pub const SPELL_RANGE: (f32, f32) = (20.0, 1200.0);

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = clamp_finite(intensity, Self::INTENSITY_RANGE, 0.7);
    }

    pub fn set_spell_seconds(&mut self, seconds: f32) {
        self.spell_seconds = clamp_finite(seconds, Self::SPELL_RANGE, 150.0);
    }

    pub fn set_wetness(&mut self, wetness: f32) {
        self.wetness = clamp_finite(wetness, (0.0, 1.0), 0.0);
    }

    pub fn set_puddles(&mut self, puddles: f32) {
        self.puddles = clamp_finite(puddles, (0.0, 1.0), 0.0);
    }

    pub fn set_snow_cover(&mut self, cover: f32) {
        self.snow_cover = clamp_finite(cover, (0.0, 1.0), 0.0);
    }

    pub fn set_snow_depth(&mut self, depth: f32) {
        self.snow_depth = clamp_finite(depth, (0.0, MAX_SNOW_DEPTH), 0.0);
    }

    /// Copy every field through its setter, so out of range values (from a
    /// hand edited scene file) are clamped; the sky starts in the steady
    /// state of the mode.
    pub fn sanitized(self) -> Self {
        let mut weather = self;
        weather.set_intensity(self.intensity);
        weather.set_spell_seconds(self.spell_seconds);
        weather.set_wetness(self.wetness);
        weather.set_puddles(self.puddles);
        weather.set_snow_cover(self.snow_cover);
        weather.set_snow_depth(self.snow_depth);
        weather.now = Now::default();
        weather.settle();
        weather
    }

    /// What should be falling now.
    fn wanted(&self) -> Spell {
        match self.mode {
            WeatherMode::Clear => Spell { kind: Precipitation::None, intensity: 0.0 },
            WeatherMode::Rain => Spell { kind: Precipitation::Rain, intensity: self.intensity },
            WeatherMode::Snow => Spell { kind: Precipitation::Snow, intensity: self.intensity },
            WeatherMode::Changing => self.now.spell,
        }
    }

    /// Jump the sky straight to what the mode wants, without waiting for
    /// the clouds to gather (on load, and in the editor). What lies on the
    /// ground stays.
    pub fn settle(&mut self) {
        let spell = self.wanted();
        let falling = spell.kind != Precipitation::None && spell.intensity > 0.0;
        self.now.kind = spell.kind;
        self.now.amount = if falling { spell.intensity } else { 0.0 };
        self.now.overcast = if falling { 1.0 } else { 0.0 };
        self.now.storm = spell.intensity;
    }

    fn random(&mut self) -> f32 {
        // xorshift32
        let mut x = self.now.random;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.now.random = x;
        x as f32 / u32::MAX as f32
    }

    fn next_spell(&mut self) {
        let roll = self.random();
        let kind = if roll < 0.45 {
            Precipitation::None
        } else if roll < 0.78 {
            Precipitation::Rain
        } else {
            Precipitation::Snow
        };
        let intensity = 0.35 + 0.65 * self.random();
        self.now.spell = Spell { kind, intensity };
        self.now.spell_left = self.spell_seconds * (0.5 + self.random());
    }

    /// Move the weather on by `dt` seconds. `sun` (0..1) is how strongly
    /// the sun shines (it dries and melts faster).
    pub fn update(&mut self, dt: f32, sun: f32) {
        if dt <= 0.0 {
            return;
        }
        if self.mode == WeatherMode::Changing {
            self.now.spell_left -= dt;
            if self.now.spell_left <= 0.0 {
                self.next_spell();
            }
        }
        let spell = self.wanted();
        let falling = spell.kind != Precipitation::None && spell.intensity > 0.0;

        // The clouds gather first; rain and snow start once they have.
        let now = &mut self.now;
        now.overcast = approach(now.overcast, falling as u8 as f32, dt / OVERCAST_SECONDS);
        if falling {
            now.storm = approach(now.storm, spell.intensity, dt / OVERCAST_SECONDS);
        }
        let ready = ((now.overcast - 0.5) / 0.4).clamp(0.0, 1.0);
        if now.kind != spell.kind {
            // One kind stops before another starts.
            now.amount = approach(now.amount, 0.0, dt / FALL_SECONDS);
            if now.amount <= 0.0 {
                now.kind = spell.kind;
            }
        } else {
            let wanted = if falling { spell.intensity * ready } else { 0.0 };
            now.amount = approach(now.amount, wanted, dt / FALL_SECONDS);
        }

        let rain = if now.kind == Precipitation::Rain { now.amount } else { 0.0 };
        let snow = if now.kind == Precipitation::Snow { now.amount } else { 0.0 };
        let sun = sun.clamp(0.0, 1.0) * (1.0 - 0.8 * now.overcast);

        // Rain wets everything, then fills the puddles; without it things dry.
        if rain > 0.0 {
            self.wetness += rain * dt / WET_SECONDS;
            self.puddles += rain * ((self.wetness - 0.5) * 2.0).max(0.0) * dt / PUDDLE_SECONDS;
        } else {
            let dry = (0.25 + 0.75 * sun) * dt / DRY_SECONDS;
            self.wetness -= dry;
            self.puddles -= dry * 0.6;
            self.puddles = self.puddles.min(self.wetness + 0.2);
        }

        // Snow covers the ground, then piles up; without it, it melts (and
        // rain melts it faster), wetting what is under it.
        if snow > 0.0 {
            self.snow_cover += snow * dt / COVER_SECONDS;
            self.snow_depth += snow * SNOW_RATE * dt * self.snow_cover;
            // Falling snow dries nothing, but stops the wet look growing.
            self.wetness -= self.snow_cover * dt / DRY_SECONDS;
        } else if self.snow_cover > 0.0 {
            let melt = (0.3 + 0.7 * sun + 3.0 * rain) * dt / MELT_SECONDS;
            self.snow_depth -= melt * MAX_SNOW_DEPTH;
            // The cover thins once the snow is shallow.
            if self.snow_depth < 0.03 {
                self.snow_cover -= melt * 1.5;
            }
            self.wetness = self.wetness.max(self.snow_cover.min(1.0) * 0.8);
        }
        self.set_wetness(self.wetness);
        self.set_puddles(self.puddles);
        self.set_snow_cover(self.snow_cover);
        self.set_snow_depth(self.snow_depth);
    }

    /// What is falling and how hard, 0..1.
    pub fn precipitation(&self) -> (Precipitation, f32) {
        (self.now.kind, self.now.amount)
    }

    /// The low clouds' coverage, from `fair` (the sky light's own setting)
    /// toward overcast as the weather turns.
    pub fn cloud_coverage(&self, fair: f32) -> f32 {
        let overcast = 0.82 + 0.18 * self.now.storm;
        fair + (overcast.max(fair) - fair) * self.now.overcast
    }

    /// The low clouds' shape (`cloudType` uniform, 0 flat .. 1 towering),
    /// from `fair` toward the weather's: a flat grey sheet for snow and
    /// steady rain, towering storm clouds for downpours.
    pub fn cloud_type(&self, fair: f32) -> f32 {
        let spell = self.wanted();
        let target = match spell.kind {
            Precipitation::Rain => smoothstep(0.8, 0.95, self.now.storm),
            _ => 0.0,
        };
        fair + (target - fair) * self.now.overcast
    }

    pub fn shading(&self) -> WeatherShading {
        let (kind, amount) = self.precipitation();
        WeatherShading {
            rain: if kind == Precipitation::Rain { amount } else { 0.0 },
            snow: if kind == Precipitation::Snow { amount } else { 0.0 },
            wetness: self.wetness,
            puddles: self.puddles,
            snow_cover: self.snow_cover,
            snow_depth: self.snow_depth,
        }
    }

    /// Whether anything shows: something falls or lies on the ground.
    pub fn is_active(&self) -> bool {
        let s = self.shading();
        s.rain + s.snow + s.wetness + s.puddles + s.snow_cover > 0.0
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl WeatherShading {
    /// Set the weather uniforms of `weather.glsl` on the bound `shader`.
    pub fn apply(&self, shader: &Shader, maps: Option<&WeatherMaps>) {
        shader.set_uniform_1f("rainAmount", self.rain);
        shader.set_uniform_1f("snowAmount", self.snow);
        shader.set_uniform_1f("wetness", self.wetness);
        shader.set_uniform_1f("puddleAmount", self.puddles);
        shader.set_uniform_1f("snowCover", self.snow_cover);
        shader.set_uniform_1f("snowDepth", self.snow_depth);
        shader.set_uniform_1f("weatherTime", seconds());
        shader.set_uniform_1i("exposureMap", EXPOSURE_UNIT as i32);
        shader.set_uniform_1i("snowTrailMap", SNOW_TRAIL_UNIT as i32);
        shader.set_uniform_1i("groundMap", GROUND_UNIT as i32);
        shader.set_uniform_1i("shelterMap", SHELTER_UNIT as i32);
        shader.set_uniform_1i("snowColourMap", SNOW_COLOUR_UNIT as i32);
        shader.set_uniform_1i("snowSurfaceMap", SNOW_SURFACE_UNIT as i32);
        shader.set_uniform_1i(
            "hasSnowTextures",
            maps.is_some_and(|maps| maps.snow_textures) as i32,
        );
        let layer = maps.is_some_and(|maps| maps.exposure.is_some());
        shader.set_uniform_1f(
            "snowLayerReach",
            if layer { snow_layer::full_reach(self.snow_depth, self.snow_cover) } else { 0.0 },
        );
        match maps.and_then(|maps| maps.exposure) {
            Some((matrix, range)) => {
                shader.set_uniform_1i("useExposure", 1);
                shader.set_uniform_mat4("exposureMatrix", &matrix);
                shader.set_uniform_1f("exposureRange", range);
            }
            None => shader.set_uniform_1i("useExposure", 0),
        }
        match maps.and_then(|maps| maps.trails) {
            Some([x, z, size]) => {
                shader.set_uniform_1i("useSnowTrails", 1);
                shader.set_uniform_3f("trailArea", x, z, size);
            }
            None => shader.set_uniform_1i("useSnowTrails", 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_settings_are_not_kept() {
        let weather = Weather {
            intensity: f32::NAN,
            spell_seconds: f32::NAN,
            wetness: f32::NAN,
            puddles: f32::NAN,
            snow_cover: f32::NAN,
            snow_depth: f32::NAN,
            ..Default::default()
        }
        .sanitized();
        let values = [
            weather.intensity,
            weather.spell_seconds,
            weather.wetness,
            weather.puddles,
            weather.snow_cover,
            weather.snow_depth,
        ];
        assert!(values.iter().all(|v| v.is_finite()), "{values:?}");
    }

    fn run(weather: &mut Weather, seconds: f32, sun: f32) {
        let mut t = 0.0;
        while t < seconds {
            weather.update(0.1, sun);
            t += 0.1;
        }
    }

    #[test]
    fn modes_round_trip_through_their_names() {
        for mode in WeatherMode::ALL {
            assert_eq!(WeatherMode::from_name(mode.name()), Some(mode));
        }
        assert_eq!(WeatherMode::from_name("Hail"), None);
    }

    #[test]
    fn rain_waits_for_the_clouds_then_wets_and_fills_puddles() {
        let mut weather = Weather { mode: WeatherMode::Rain, intensity: 1.0, ..Default::default() };
        run(&mut weather, 5.0, 1.0);
        assert_eq!(weather.precipitation().1, 0.0, "clouds gather first");
        assert!(weather.cloud_coverage(0.2) > 0.2);
        run(&mut weather, 60.0, 1.0);
        assert_eq!(weather.precipitation(), (Precipitation::Rain, 1.0));
        assert_eq!(weather.wetness, 1.0);
        assert!(weather.puddles > 0.0);
        assert!(weather.cloud_coverage(0.2) > 0.95);
        // Heavy rain comes from storm clouds.
        assert!(weather.cloud_type(0.68) > 0.9);
    }

    #[test]
    fn clear_weather_dries_the_ground() {
        let mut weather = Weather { mode: WeatherMode::Clear, wetness: 1.0, puddles: 1.0, ..Default::default() };
        run(&mut weather, 200.0, 1.0);
        assert_eq!(weather.wetness, 0.0);
        assert_eq!(weather.puddles, 0.0);
        assert_eq!(weather.cloud_coverage(0.2), 0.2);
    }

    #[test]
    fn snow_covers_then_piles_up_then_melts() {
        let mut weather = Weather { mode: WeatherMode::Snow, intensity: 1.0, ..Default::default() };
        weather.settle();
        run(&mut weather, 30.0, 0.0);
        let early = (weather.snow_cover, weather.snow_depth);
        assert!(early.0 > 0.5);
        run(&mut weather, 300.0, 0.0);
        assert_eq!(weather.snow_cover, 1.0);
        assert!(weather.snow_depth > early.1 * 4.0);
        assert!(weather.snow_depth <= MAX_SNOW_DEPTH);
        assert_eq!(weather.wetness, 0.0, "snow is not wet");
        // Snow clouds are a flat sheet.
        assert!(weather.cloud_type(0.68) < 0.05);

        weather.mode = WeatherMode::Clear;
        run(&mut weather, 30.0, 1.0);
        assert!(weather.snow_cover > 0.9, "snow does not vanish at once");
        assert!(weather.wetness > 0.5, "melting snow wets the ground");
        run(&mut weather, 600.0, 1.0);
        assert_eq!(weather.snow_cover, 0.0);
        assert_eq!(weather.snow_depth, 0.0);
    }

    #[test]
    fn rain_stops_before_snow_starts() {
        let mut weather = Weather { mode: WeatherMode::Rain, intensity: 1.0, ..Default::default() };
        weather.settle();
        weather.mode = WeatherMode::Snow;
        weather.update(1.0, 0.0);
        assert_eq!(weather.precipitation().0, Precipitation::Rain);
        run(&mut weather, 30.0, 0.0);
        assert_eq!(weather.precipitation().0, Precipitation::Snow);
        assert!(weather.precipitation().1 > 0.5);
    }

    #[test]
    fn changing_weather_brings_rain_and_snow() {
        let mut weather = Weather { mode: WeatherMode::Changing, spell_seconds: 30.0, ..Default::default() };
        let (mut rained, mut snowed, mut cleared) = (false, false, false);
        for _ in 0..40_000 {
            weather.update(0.1, 0.5);
            match weather.precipitation() {
                (Precipitation::Rain, a) if a > 0.3 => rained = true,
                (Precipitation::Snow, a) if a > 0.3 => snowed = true,
                (_, a) if a == 0.0 => cleared = true,
                _ => {}
            }
        }
        assert!(rained && snowed && cleared);
    }

    #[test]
    fn scenes_without_weather_are_clear_and_dry() {
        let weather: Weather = ron::from_str("()").unwrap();
        assert_eq!(weather.mode, WeatherMode::Clear);
        assert!(!weather.sanitized().is_active());
        let clamped = Weather { wetness: 4.0, snow_depth: 9.0, ..Default::default() }.sanitized();
        assert_eq!((clamped.wetness, clamped.snow_depth), (1.0, MAX_SNOW_DEPTH));
    }
}
