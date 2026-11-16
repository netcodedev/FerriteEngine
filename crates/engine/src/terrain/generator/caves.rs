//! What the height field cannot say: caves under the ground and
//! overhangs on its cliffs. Both are 3D noise, sampled on a 4 m lattice
//! around a chunk (`Underground3`) and blended in between, so they cost
//! little next to the surface's metre-by-metre samples.
//!
//! - **Tunnels** ("spaghetti") run where two noise fields are both near
//!   zero: along the lines where their zero surfaces cross. They wind, fork
//!   and are flattened a little, so most of them run more along than down.
//! - **Caverns** ("cheese") open where a third field is high, only well
//!   below the surface.
//! - **Entrances**: near the surface tunnels close, except in patches
//!   where they break through to the open air.
//! - **Overhangs**: on steep slopes the ground is pushed in and out by a
//!   3D field, so cliffs get ledges and undercuts.
//!
//! Caves stay above the sea (so no water level is needed in them) and
//! under land well above it; none open near the spawn area.

use super::{
    noise::{Fractal, Fractal3},
    world::smoothstep,
    WorldSettings, SEA_LEVEL,
};

/// How deep under the surface caves reach, in metres.
pub const CAVE_DEPTH: f32 = 120.0;
/// How far the overhangs push the ground in or out on a steep slope (at
/// `WorldSettings::overhangs` 1), in metres.
pub const OVERHANG_DEPTH: f32 = 7.0;
/// What `Underground::cave` gives where no cave can be.
pub const NO_CAVE: f32 = -64.0;

const TUNNEL_WAVELENGTH: f64 = 220.0;
const CAVERN_WAVELENGTH: f64 = 120.0;

/// The cave field at a point, in parts: the two tunnel fields (tunnels
/// run where both are zero), the tunnel's radius there and how far into a
/// cavern the point lies. The tunnel fields are smooth, so they blend well
/// between lattice points; the distance from their zero line (the tunnel's
/// axis) is not, so it is worked out after blending.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaveParts {
    pub a: f32,
    pub b: f32,
    pub radius: f32,
    pub cavern: f32,
}

impl CaveParts {
    pub const NONE: CaveParts = CaveParts {
        a: 1.0,
        b: 1.0,
        radius: NO_CAVE,
        cavern: NO_CAVE,
    };

    /// How far into a cave this is, in metres (roughly): positive in the
    /// open.
    pub fn value(&self) -> f32 {
        let distance = (self.a * self.a + self.b * self.b).sqrt() * (TUNNEL_WAVELENGTH / Fractal3::GRADIENT) as f32;
        (self.radius - distance).max(self.cavern).max(NO_CAVE)
    }
}

pub struct Underground {
    caves: f32,
    overhangs: f32,
    spawn_radius: f64,
    tunnel_a: Fractal3,
    tunnel_b: Fractal3,
    tunnel_size: Fractal3,
    /// Where there are tunnels at all: they come in systems.
    tunnel_systems: Fractal3,
    caverns: Fractal3,
    entrances: Fractal,
    overhang: Fractal3,
}

impl Underground {
    pub fn new(seed: u64, settings: &WorldSettings) -> Self {
        Self {
            caves: settings.caves.max(0.0),
            overhangs: settings.overhangs.max(0.0),
            spawn_radius: settings.spawn_radius,
            tunnel_a: Fractal3::new(seed, "tunnel a", 1, TUNNEL_WAVELENGTH),
            tunnel_b: Fractal3::new(seed, "tunnel b", 1, TUNNEL_WAVELENGTH),
            tunnel_size: Fractal3::new(seed, "tunnel size", 1, 90.0),
            tunnel_systems: Fractal3::new(seed, "tunnel systems", 2, 600.0),
            caverns: Fractal3::new(seed, "caverns", 2, CAVERN_WAVELENGTH),
            entrances: Fractal::new(seed, "cave entrances", 2, 700.0),
            overhang: Fractal3::new(seed, "overhangs", 2, 40.0),
        }
    }

    pub fn has_caves(&self) -> bool {
        self.caves > 0.0
    }

    pub fn has_overhangs(&self) -> bool {
        self.overhangs > 0.0
    }

    /// Where caves may be under ground whose surface lies between `low`
    /// and `high`: the lowest and highest height, or `None`.
    pub fn cave_band(&self, low: f32, high: f32) -> Option<(f32, f32)> {
        if !self.has_caves() || high < SEA_LEVEL + 6.0 {
            return None;
        }
        Some(((low - CAVE_DEPTH).max(SEA_LEVEL + 4.0), high))
    }

    /// How far into a cave world `(x, y, z)` lies, in metres (roughly):
    /// positive in the open, negative in the rock. `surface` is the height
    /// of the ground over it, `slope` its steepness (rise per metre).
    pub fn cave(&self, x: f64, y: f64, z: f64, surface: f32, slope: f32) -> f32 {
        self.cave_parts(x, y, z, surface, slope).value()
    }

    /// The cave field at world `(x, y, z)` in parts that blend well
    /// between lattice points (see `CaveParts`).
    pub fn cave_parts(&self, x: f64, y: f64, z: f64, surface: f32, slope: f32) -> CaveParts {
        if !self.has_caves() || surface < SEA_LEVEL + 6.0 {
            return CaveParts::NONE;
        }
        let depth = surface - y as f32;
        let floor = SEA_LEVEL + 4.0;
        if depth < -OVERHANG_DEPTH - 4.0 || depth > CAVE_DEPTH + 20.0 || (y as f32) < floor - 20.0 {
            return CaveParts::NONE;
        }
        // Tunnels: flattened (y counts more), so they wind more along than
        // down.
        let ys = y * 1.8;
        let (a, b) = (self.tunnel_a.at(x, ys, z) as f32, self.tunnel_b.at(x, ys, z) as f32);
        let system = smoothstep(-0.05, 0.2, self.tunnel_systems.at(x, y * 0.5, z)) as f32;
        let radius = (2.0 + 1.3 * (self.tunnel_size.at(x, y, z) as f32 + 1.0)) * self.caves.min(2.0) * system
            - 4.0 * (1.0 - system);
        // Near the surface they close, but where an entrance is: on a
        // hillside, so the tunnel opens as a mouth in the slope instead
        // of a trench along flat ground.
        let entrance = smoothstep(0.3, 0.5, self.entrances.at(x, z))
            * smoothstep(0.3, 0.7, slope as f64)
            * self.away_from_spawn(x, z);
        let radius = radius - (1.0 - entrance as f32) * (14.0 - depth).max(0.0);
        // Caverns, deep down only.
        let c = self.caverns.at(x, y * 1.4, z) as f32;
        let threshold = 0.52 + 0.1 * (1.0 - self.caves.min(2.0));
        let cavern = (c - threshold) * (CAVERN_WAVELENGTH / Fractal3::GRADIENT) as f32 - (30.0 - depth).max(0.0);
        // A floor over the sea (caves flatten out toward it) and one at
        // the band's bottom.
        let bottom = 1.5 * (floor + 6.0 - y as f32).max(0.0) + (depth - CAVE_DEPTH).max(0.0);
        CaveParts {
            a,
            b,
            radius: (radius - bottom).max(NO_CAVE),
            cavern: (cavern - bottom).max(NO_CAVE),
        }
    }

    /// How far (metres) the ground is pushed out at world `(x, y, z)`
    /// where the slope of the height field is `slope` (rise per metre).
    pub fn overhang(&self, x: f64, y: f64, z: f64, slope: f32) -> f32 {
        let amplitude = self.overhang_amplitude(slope);
        if amplitude <= 0.0 {
            return 0.0;
        }
        amplitude * self.overhang.at(x, y * 0.8, z) as f32
    }

    /// How far overhangs may push the ground where the slope is `slope`.
    pub fn overhang_amplitude(&self, slope: f32) -> f32 {
        self.overhangs * OVERHANG_DEPTH * smoothstep(0.9, 1.7, slope as f64) as f32
    }

    fn away_from_spawn(&self, x: f64, z: f64) -> f64 {
        let r = self.spawn_radius;
        if r <= 0.0 {
            return 1.0;
        }
        smoothstep(r * 1.5, r * 3.0, x.hypot(z))
    }
}

/// The underground around one chunk: on a lattice `STEP` metres apart from
/// `ORIGIN` (chunk-local), for each point how far the ground is pushed out
/// and the cave field's parts. The lattice is the world's, so
/// neighbouring chunks blend the same values where they meet.
pub struct Underground3 {
    /// Push, then the cave parts `a`, `b`, `radius`, `cavern`.
    values: Vec<[f32; 5]>,
    /// Per lattice cell, whether anything happens in it: most of the
    /// rock is solid and most of the surface is not pushed, and there the
    /// field is the height field's.
    active: Vec<bool>,
}

impl Underground3 {
    pub const STEP: i32 = 4;
    pub const ORIGIN: i32 = -12;
    pub const SPAN: i32 = (super::CHUNK_SIZE as i32 - 2 * Self::ORIGIN) / Self::STEP + 1;

    pub(super) fn new(values: Vec<[f32; 5]>) -> Self {
        let span = Self::SPAN;
        debug_assert_eq!(values.len(), (span * span * span) as usize);
        // A tunnel's value rises at most about half a cell's diagonal
        // (3.5 m) between corners.
        let busy: Vec<bool> = values
            .iter()
            .map(|v| v[0] != 0.0 || Self::cave(v).value() > -5.0)
            .collect();
        let cells = span - 1;
        let mut active = vec![false; (cells * cells * cells) as usize];
        for k in 0..cells {
            for j in 0..cells {
                for i in 0..cells {
                    active[((k * cells + j) * cells + i) as usize] = (0..8).any(|c| {
                        let (dx, dy, dz) = (c & 1, (c >> 1) & 1, (c >> 2) & 1);
                        busy[(((k + dz) * span + j + dy) * span + i + dx) as usize]
                    });
                }
            }
        }
        Self { values, active }
    }

    fn cave(v: &[f32; 5]) -> CaveParts {
        CaveParts {
            a: v[1],
            b: v[2],
            radius: v[3],
            cavern: v[4],
        }
    }

    pub(super) fn entry(push: f32, cave: CaveParts) -> [f32; 5] {
        [push, cave.a, cave.b, cave.radius, cave.cavern]
    }

    /// `(push out, cave)` at chunk-local whole metres, blended.
    #[inline]
    pub fn at(&self, x: i32, y: i32, z: i32) -> [f32; 2] {
        let cell = |v: i32| {
            let r = v - Self::ORIGIN;
            let i = (r / Self::STEP).clamp(0, Self::SPAN - 2);
            (i, (r - i * Self::STEP) as f32 / Self::STEP as f32)
        };
        let ((ix, tx), (iy, ty), (iz, tz)) = (cell(x), cell(y), cell(z));
        let cells = Self::SPAN - 1;
        if !self.active[((iz * cells + iy) * cells + ix) as usize] {
            return [0.0, NO_CAVE];
        }
        let index = |i: i32, j: i32, k: i32| ((k * Self::SPAN + j) * Self::SPAN + i) as usize;
        let mut out = [0.0f32; 5];
        for corner in 0..8 {
            let (dx, dy, dz) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
            let w = if dx == 1 { tx } else { 1.0 - tx }
                * if dy == 1 { ty } else { 1.0 - ty }
                * if dz == 1 { tz } else { 1.0 - tz };
            if w == 0.0 {
                continue;
            }
            let v = self.values[index(ix + dx, iy + dy, iz + dz)];
            for k in 0..5 {
                out[k] += w * v[k];
            }
        }
        [out[0], Self::cave(&out).value()]
    }
}
