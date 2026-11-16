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
    /// Whether rivers and lakes are planned (see `hydrology`).
    pub rivers: bool,
    /// Whether settlements and the roads between them are planned (see
    /// `roads`).
    pub roads: bool,
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
            rivers: true,
            roads: true,
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
    seed: u64,
    /// The points the gullies run from (see `gullies`), worked out once
    /// for the whole world: a grid of cells for each octave.
    gully_points: Vec<GullyGrid>,
}

/// The gully points of one octave, a cell each, from cell `first` on in x
/// and z, `n` along each.
struct GullyGrid {
    first: i64,
    n: i64,
    points: Vec<GullyPoint>,
}

/// A point gullies run from: where it lies (in cells of its octave), which
/// way across the slope there its stripes' phase runs, and how strongly
/// (0 on flat ground and crests, 1 on steep slopes).
#[derive(Clone, Copy)]
struct GullyPoint {
    at: [f64; 2],
    across: [f64; 2],
    strength: f64,
}

impl WorldShape {
    pub fn new(seed: u64, settings: WorldSettings) -> Self {
        let mut shape = Self {
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
            gully_points: Vec::new(),
            seed,
        };
        // Over the world and the sea around it (as far as the maps reach),
        // on all cores.
        let reach = shape.settings.size * 0.6;
        let mut grids = Vec::new();
        let mut wavelength = GULLY_WAVELENGTH;
        for octave in 0..GULLY_OCTAVES {
            let first = (-reach / wavelength).floor() as i64 - 2;
            let n = (2.0 * reach / wavelength).ceil() as i64 + 5;
            let mut points = vec![GullyPoint { at: [0.0; 2], across: [1.0, 0.0], strength: 0.0 }; (n * n) as usize];
            let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
            let rows = (n as usize).div_ceil(threads) * n as usize;
            let shape = &shape;
            std::thread::scope(|scope| {
                for (k, chunk) in points.chunks_mut(rows).enumerate() {
                    scope.spawn(move || {
                        for (i, p) in chunk.iter_mut().enumerate() {
                            let cell = (k * rows + i) as i64;
                            *p = shape.make_gully_point(octave, first + cell % n, first + cell / n, wavelength);
                        }
                    });
                }
            });
            grids.push(GullyGrid { first, n, points });
            wavelength *= 0.5;
        }
        shape.gully_points = grids;
        shape
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
        let mut c = self.continentalness.at(wx, wz) * 1.6 + 0.55;
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

    /// How mountainous the land is with climate `c`: 0 (not) .. about 1
    /// (in a high range).
    fn mountains(&self, c: &Climate) -> f32 {
        let s = &self.settings;
        // Mountains rise inland (and on the odd island), not out of the sea.
        let inland = smoothstep(-0.12, 0.25, c.continentalness as f64) as f32;
        // The high mountains stand in ranges; elsewhere low erosion only
        // makes rugged uplands.
        let rugged = s.mountains.at(c.erosion);
        (c.range * (0.6 + 0.4 * rugged)).max(rugged * 0.15) * inland * s.mountain_scale
    }

    /// The mountains' height at a point with climate `c`, before the
    /// valleys are cut into them (see `eroded`).
    pub(super) fn uplift(&self, c: &Climate) -> f32 {
        let s = &self.settings;
        (s.ridges.at(c.ridges) + s.massifs) * self.mountains(c)
    }

    /// The large shape at world `(x, z)` (see `Macro`).
    pub fn macro_at(&self, x: f64, z: f64) -> Macro {
        let c = self.climate(x, z);
        let s = &self.settings;
        let base = s.continents.at(c.continentalness);
        let mountains = self.mountains(&c);
        let ridge = self.eroded(x, z, self.uplift(&c));
        // Hills calm down toward the coast, so beaches are gentle.
        let coast = 0.35 + 0.65 * smoothstep(-0.1, 0.15, c.continentalness as f64) as f32;
        Macro {
            height: base + ridge,
            hills: s.hills.at(c.erosion) * s.hill_scale * coast * (1.0 + 0.5 * mountains),
            temperature: c.temperature,
            humidity: c.humidity,
        }
    }

    /// The mountains `uplift` metres high at world `(x, z)` with their
    /// valleys cut in: gullies running down the slopes in several sizes,
    /// so the slopes break into spurs and side valleys that run down from
    /// the crests, as water wears real mountains down. (After the erosion
    /// filter Rune Skovbo Johansen describes for terrain noise.)
    pub(super) fn eroded(&self, x: f64, z: f64, uplift: f32) -> f32 {
        if uplift < 1.0 {
            return uplift;
        }
        let mut h = uplift as f64;
        let mut wavelength = GULLY_WAVELENGTH;
        // Deeper gullies in higher mountains.
        let mut depth = uplift as f64 * GULLY_DEPTH;
        for octave in 0..GULLY_OCTAVES {
            // As much up between the gullies as down in them: cut only
            // down, the few spots that lie between the gullies of every
            // octave would stand as needles.
            h += depth * 0.5 * self.gullies(x, z, octave, wavelength);
            wavelength *= 0.5;
            depth *= GULLY_GAIN;
        }
        h.max(0.0) as f32
    }

    /// Gullies `wavelength` apart at world `(x, z)`: `-1` in a gully, `1`
    /// on the ridge between two, fading to 0 where the land is flat. The
    /// stripes of points scattered on a grid, each running down the slope
    /// where it lies (a plane wave: its gullies do not swing round as the
    /// slope turns, which would bend them into cliffs), blended.
    fn gullies(&self, x: f64, z: f64, octave: usize, wavelength: f64) -> f64 {
        let (gx, gz) = (x / wavelength, z / wavelength);
        let (cx, cz) = (gx.floor() as i64, gz.floor() as i64);
        let tau = std::f64::consts::TAU;
        // Each point's stripes as a turning vector (cos, sin of their
        // phase), blended, then brought back to full length: blended as
        // they are, stripes of different phases would cancel into noise.
        let (mut c, mut s, mut weights) = (0.0, 0.0, 0.0);
        // The four by four cells around: points further off lie a cell and
        // a half away or more, where their weight is down to nothing (so no
        // seams run along the grid).
        let (ox, oz) = (if gx - gx.floor() < 0.5 { -2 } else { -1 }, if gz - gz.floor() < 0.5 { -2 } else { -1 });
        for j in oz..oz + 4 {
            for i in ox..ox + 4 {
                let p = self.gully_point(octave, cx + i, cz + j, wavelength);
                let (vx, vz) = (gx - p.at[0], gz - p.at[1]);
                let w = (1.0 - (vx * vx + vz * vz) / 2.25).max(0.0).powi(3);
                let (sin, cos) = ((vx * p.across[0] + vz * p.across[1]) * tau).sin_cos();
                c += w * p.strength * cos;
                s += w * p.strength * sin;
                weights += w;
            }
        }
        // Where the stripes nearly cancel (or the land is flat) the blend
        // is short: it fades out there rather than turn abruptly.
        c / c.hypot(s).max(weights * 0.5).max(1e-9)
    }

    /// The point gullies of `octave` (`wavelength` apart) run from in cell
    /// `(px, pz)` (see `GullyPoint`).
    fn gully_point(&self, octave: usize, px: i64, pz: i64, wavelength: f64) -> GullyPoint {
        if let Some(grid) = self.gully_points.get(octave) {
            let (i, j) = (px - grid.first, pz - grid.first);
            if (0..grid.n).contains(&i) && (0..grid.n).contains(&j) {
                return grid.points[(j * grid.n + i) as usize];
            }
        }
        // Far out at sea (or while the grids are being made).
        self.make_gully_point(octave, px, pz, wavelength)
    }

    fn make_gully_point(&self, octave: usize, px: i64, pz: i64, wavelength: f64) -> GullyPoint {
        let hash = mix(self.seed ^ mix(octave as u64) ^ mix((px as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ pz as u64));
        let at = [
            px as f64 + (hash & 0xFFFF) as f64 / 65535.0,
            pz as f64 + ((hash >> 16) & 0xFFFF) as f64 / 65535.0,
        ];
        let (x, z) = (at[0] * wavelength, at[1] * wavelength);
        // The slope there, from the uplift some way off: smoothed over the
        // kinks of the ridges' noise.
        let e = 150.0;
        let here = self.uplift(&self.climate(x, z));
        let gx = (self.uplift(&self.climate(x + e, z)) - here) as f64 / e;
        let gz = (self.uplift(&self.climate(x, z + e)) - here) as f64 / e;
        let length = gx.hypot(gz);
        let across = if length > 1e-6 { [-gz / length, gx / length] } else { [1.0, 0.0] };
        GullyPoint { at, across, strength: smoothstep(0.05, 0.35, length) }
    }

    /// The hills on top of the large shape: `-1..1`, times `Macro::hills`.
    pub fn detail(&self, x: f64, z: f64) -> f32 {
        self.detail.at(x, z) as f32
    }
}

/// Wavelength of the coarsest gullies cut into mountains, metres; each
/// finer octave half that.
const GULLY_WAVELENGTH: f64 = 3000.0;
const GULLY_OCTAVES: usize = 5;
/// Depth of the coarsest gullies, as a share of the mountain's height, and
/// of each finer octave's relative to the one before.
const GULLY_DEPTH: f64 = 0.35;
const GULLY_GAIN: f64 = 0.4;

/// Mixes the bits of `z` (splitmix64's finaliser).
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
