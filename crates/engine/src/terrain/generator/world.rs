//! The world's large shape, from climate maps the way Minecraft builds
//! its terrain: smooth noise fields (continentalness, erosion, ridges)
//! that splines turn into metres.
//!
//! - **Continentalness** decides between deep sea, shelf, coast and
//!   inland, and how high the land lies. Toward the edge of the world it
//!   falls, so the world is an island (or several) in an endless ocean.
//! - **Erosion** decides how rugged the land is: low erosion means
//!   mountains, high erosion wide plains.
//! - **Ridges** draw the crest lines of the mountain ranges where erosion
//!   lets them rise.
//!
//! Everything here is large: it changes over hundreds of metres to
//! kilometres. It is sampled on a 4 m grid (`MacroTile`) and smoothly
//! interpolated; only the hills on top are sampled at every metre.

use serde::{Deserialize, Serialize};

use super::{noise::Fractal, spline::Spline};

/// Height of the sea's surface, in metres.
pub const SEA_LEVEL: f32 = 0.0;

/// What shapes a world, as stored with the terrain in the scene. Every
/// field may be left out of a scene file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldSettings {
    /// Edge length of the square world in metres, centred on the origin.
    /// Beyond it lies open sea.
    pub size: f64,
    /// Continentalness (-1..1) to the land's (or sea floor's) base height.
    pub continents: Spline,
    /// Erosion (-1..1) to how much the ridges may rise (0..1).
    pub mountains: Spline,
    /// Ridge value (0..1) to the height a ridge adds, in metres.
    pub ridges: Spline,
    /// Metres mountainous land rises as a whole, under its ridges, so a
    /// range stands on a broad base instead of rising from the plain.
    pub massifs: f32,
    /// Erosion (-1..1) to the height of the hills on top, in metres.
    pub hills: Spline,
    /// Scales the mountains (1: as the splines say).
    pub mountain_scale: f32,
    /// Scales the hills.
    pub hill_scale: f32,
    /// Metres around the origin kept as low, gentle land, so a scene's
    /// player and camera stand on the ground.
    pub spawn_radius: f64,
    /// How many and how wide caves are (0: none, 1: as designed, up to 2).
    pub caves: f32,
    /// How far cliffs overhang (0: not at all, 1: as designed).
    pub overhangs: f32,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            size: 32768.0,
            continents: Spline::new(&[
                (-1.0, -420.0),
                (-0.6, -380.0),
                (-0.38, -220.0),
                (-0.24, -45.0),
                (-0.14, -12.0),
                (-0.07, -1.0),
                (-0.02, 3.0),
                (0.12, 18.0),
                (0.35, 50.0),
                (0.65, 110.0),
                (1.0, 190.0),
            ]),
            mountains: Spline::new(&[
                (-1.0, 1.0),
                (-0.5, 1.0),
                (-0.2, 0.65),
                (0.1, 0.25),
                (0.4, 0.04),
                (1.0, 0.0),
            ]),
            ridges: Spline::new(&[
                (0.0, 0.0),
                (0.2, 30.0),
                (0.4, 150.0),
                (0.6, 380.0),
                (0.8, 680.0),
                (1.0, 900.0),
            ]),
            massifs: 600.0,
            hills: Spline::new(&[
                (-1.0, 24.0),
                (-0.3, 18.0),
                (0.2, 11.0),
                (0.6, 5.0),
                (1.0, 2.5),
            ]),
            mountain_scale: 1.0,
            hill_scale: 1.0,
            spawn_radius: 700.0,
            caves: 1.0,
            overhangs: 1.0,
        }
    }
}

impl WorldSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The climate maps at one point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Climate {
    /// -1 (deep sea) .. 1 (far inland).
    pub continentalness: f32,
    /// -1 (rugged) .. 1 (flat).
    pub erosion: f32,
    /// 0..1, 1 on a crest line.
    pub ridges: f32,
    /// 0..1: how far into a mountain range a point lies.
    pub range: f32,
    /// Mean temperature at sea level, °C; it drops with height (see
    /// `biome::temperature_at`). Colder toward the north (−z).
    pub temperature: f32,
    /// 0 (dry) .. 1 (wet); wetter near the coast.
    pub humidity: f32,
}

/// The large shape at one point: what the 4 m grid holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Macro {
    /// Height without the hills, in metres.
    pub height: f32,
    /// How high the hills rise here, in metres.
    pub hills: f32,
    /// Temperature at sea level, °C (see `Climate`).
    pub temperature: f32,
    /// Humidity, 0..1 (see `Climate`).
    pub humidity: f32,
}

/// The noise fields of a world (see the module).
pub struct WorldShape {
    pub settings: WorldSettings,
    warp_x: Fractal,
    warp_z: Fractal,
    continentalness: Fractal,
    erosion: Fractal,
    /// Very large ridges: the lines along which mountain ranges run.
    ranges: Fractal,
    ridges: Fractal,
    /// The hills on top, at every metre.
    detail: Fractal,
    temperature: Fractal,
    humidity: Fractal,
}

impl WorldShape {
    pub fn new(seed: u64, settings: WorldSettings) -> Self {
        Self {
            settings,
            warp_x: Fractal::new(seed, "warp x", 3, 7000.0),
            warp_z: Fractal::new(seed, "warp z", 3, 7000.0),
            continentalness: Fractal::new(seed, "continentalness", 5, 12000.0),
            erosion: Fractal::new(seed, "erosion", 4, 6000.0),
            ranges: Fractal::with_gain(seed, "ranges", 2, 16000.0, 0.3),
            ridges: Fractal::with_gain(seed, "ridges", 4, 5000.0, 0.35),
            detail: Fractal::new(seed, "hills", 6, 340.0),
            temperature: Fractal::new(seed, "temperature", 3, 10000.0),
            humidity: Fractal::new(seed, "humidity", 3, 7000.0),
        }
    }

    /// Distance from the origin, in metres, scaled into how far a point is
    /// from the spawn area (0 inside it, 1 well outside).
    fn away_from_spawn(&self, x: f64, z: f64) -> f32 {
        let r = self.settings.spawn_radius;
        if r <= 0.0 {
            return 1.0;
        }
        smoothstep(r, r * 4.0, x.hypot(z)) as f32
    }

    /// The climate maps at world `(x, z)`.
    pub fn climate(&self, x: f64, z: f64) -> Climate {
        // Bend the coordinates so coasts and ranges do not follow the
        // noise's grid.
        let (wx, wz) = (
            x + 1200.0 * self.warp_x.at(x, z),
            z + 1200.0 * self.warp_z.at(x, z),
        );
        let mut c = self.continentalness.at(wx, wz) * 1.6 + 0.22;
        // The world's edge sinks into the deep sea over its last 3 km.
        // Its corners are rounded (a superellipse) and its line bent like
        // the coasts, so the world does not look square.
        let half = self.settings.size * 0.5;
        let (ex, ez) = ((wx / half).abs(), (wz / half).abs());
        let edge = half * (1.0 - (ex.powi(4) + ez.powi(4)).powf(0.25));
        c -= (1.0 - smoothstep(0.0, 3500.0_f64.min(half), edge)) * 2.2;
        let erosion = self.erosion.at(wx * 1.1, wz * 0.9) * 1.6;
        let range = smoothstep(0.42, 0.9, self.ranges.ridges(wx, wz));
        // Ridges on coordinates bent their own way. Not much: bending
        // squeezes space in places, and squeezed ridges become walls.
        let ridges = self.ridges.ridges(
            wx + 450.0 * self.warp_z.at(z * 1.3, x * 1.3),
            wz + 450.0 * self.warp_x.at(z * 1.3, x * 1.3),
        );
        let mut climate = Climate {
            continentalness: c.clamp(-1.0, 1.0) as f32,
            erosion: erosion.clamp(-1.0, 1.0) as f32,
            ridges: ridges as f32,
            range: range as f32,
            temperature: 0.0,
            humidity: 0.0,
        };
        // Toward the north (−z) colder, by up to 7 °C at the edge.
        let north = (wz / half).clamp(-1.0, 1.0) * 7.0;
        climate.temperature = (13.0 + 14.0 * self.temperature.at(wx, wz) * 1.4 + north) as f32;
        let coast = 1.0 - smoothstep(-0.05, 0.35, climate.continentalness as f64);
        climate.humidity =
            (0.5 + 0.5 * self.humidity.at(wx, wz) * 1.5 + 0.18 * coast).clamp(0.0, 1.0) as f32;
        // Around the origin: low, gentle land 10 to 40 m above the sea,
        // where scenes made for the old terrain have their players and
        // cameras (bent like the rest, so it does not show as a disc).
        let away = self.away_from_spawn(wx, wz);
        if away < 1.0 {
            climate.continentalness = lerp(0.1, climate.continentalness, away);
            climate.range *= away;
            climate.erosion = lerp(0.35, climate.erosion, away);
            climate.temperature = lerp(14.0, climate.temperature, away);
            climate.humidity = lerp(0.55, climate.humidity, away);
        }
        climate
    }

    /// The large shape at world `(x, z)` (see `Macro`).
    pub fn macro_at(&self, x: f64, z: f64) -> Macro {
        let c = self.climate(x, z);
        let s = &self.settings;
        let base = s.continents.at(c.continentalness);
        // Mountains rise inland (and on the odd island), not out of the sea.
        let inland = smoothstep(-0.12, 0.25, c.continentalness as f64) as f32;
        // The high mountains stand in ranges; elsewhere low erosion only
        // makes rugged uplands.
        let rugged = s.mountains.at(c.erosion);
        let mountains = (c.range * (0.6 + 0.4 * rugged)).max(rugged * 0.15) * inland * s.mountain_scale;
        let ridge = (s.ridges.at(c.ridges) + s.massifs) * mountains;
        // Hills calm down toward the coast, so beaches are gentle.
        let coast = 0.35 + 0.65 * smoothstep(-0.1, 0.15, c.continentalness as f64) as f32;
        Macro {
            height: base + ridge,
            hills: s.hills.at(c.erosion) * s.hill_scale * coast * (1.0 + 0.5 * mountains),
            temperature: c.temperature,
            humidity: c.humidity,
        }
    }

    /// The hills on top of the large shape: `-1..1`, times `Macro::hills`.
    pub fn detail(&self, x: f64, z: f64) -> f32 {
        self.detail.at(x, z) as f32
    }
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
