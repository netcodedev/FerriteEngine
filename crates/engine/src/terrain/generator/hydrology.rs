//! Rivers and lakes, planned over the whole world at once.
//!
//! The large shape is sampled on a coarse grid (`CELL` metres) over the
//! world and its sea. Then:
//!
//! 1. **Priority flood** from the sea inwards: every cell is reached from
//!    its lowest neighbour already reached, and is raised (a little more
//!    than) to that neighbour's height where it lies lower. Raised cells
//!    are depressions: they fill to lakes. The neighbour a cell was reached
//!    from is where its water flows.
//! 2. **Flow**: in the reverse of that order (from the top down), each cell
//!    passes the rain that fell on it and all that flowed into it to its
//!    downstream neighbour. Rain grows with humidity.
//! 3. **Rivers** run where enough water has gathered: from cell to
//!    downstream cell, wider the more water, until they reach a lake or
//!    the sea. Cell centres are shifted a little at random so rivers do not
//!    follow the grid.
//!
//! The ground then gets the rivers' beds and banks carved in (see
//! `carve`), and the water mesh their surfaces (see `water_level`).

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    sync::{Arc, Mutex},
};

use super::{
    noise::{field_seed, Fractal},
    world::{smoothstep, WorldShape},
    WorldSettings, SEA_LEVEL,
};
use crate::terrain::CHUNK_SIZE;

/// Metres between the plan's samples.
pub const CELL: f64 = 64.0;
/// Water (in km² of rain catchment, weighted by humidity) a cell needs
/// before a river runs from it.
const RIVER_FLOW: f32 = 1.5;
/// How much deeper than its surroundings a filled cell must be to count
/// as lake.
const LAKE_DEPTH: f32 = 0.75;
/// Smallest lake, in cells, and how deep its deepest cell must be.
const MIN_LAKE_CELLS: usize = 25;
const MIN_LAKE_DEPTH: f32 = 6.0;
/// How far rivers wind off the straight line between the plan's cells,
/// at most, in metres, and over what length.
const MEANDER: f64 = 18.0;
const MEANDER_WAVELENGTH: f64 = 170.0;
/// Narrowest and widest river, metres.
const MIN_WIDTH: f32 = 3.5;
const MAX_WIDTH: f32 = 70.0;
/// Where `Hydrology::lake_at` reaches from 0 (no lake near) to 1 (well
/// inside one): the water starts at `LAKE_WATER`, the shore lies at about
/// `LAKE_SHORE`.
pub const LAKE_WATER: f32 = 0.25;
const LAKE_SHORE: f32 = 0.45;

/// A piece of river between two points, with its water level and width at
/// each end.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub level: [f32; 2],
    pub width: [f32; 2],
}

impl Segment {
    /// Distance from `(x, z)` to the segment, and how far along it (0..1)
    /// the nearest point lies.
    fn distance(&self, x: f64, z: f64) -> (f64, f64) {
        let (dx, dz) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        let length2 = dx * dx + dz * dz;
        let t = if length2 > 0.0 {
            (((x - self.a[0]) * dx + (z - self.a[1]) * dz) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (px, pz) = (self.a[0] + dx * t, self.a[1] + dz * t);
        ((x - px).hypot(z - pz), t)
    }

    /// How far beyond its centre line the river reshapes the ground.
    fn reach(&self) -> f64 {
        let w = self.width[0].max(self.width[1]) as f64;
        w * 0.5 + bank_width(w as f32) as f64
    }
}

/// Width of the banks beside a river `width` wide, over which the ground
/// rises from the water back to the land's own height.
fn bank_width(width: f32) -> f32 {
    16.0 + 2.5 * width
}

/// Depth of a river `width` wide in its middle.
fn depth(width: f32) -> f32 {
    1.2 + 0.07 * width
}

/// The nearest river to a point, as far as the ground cares.
#[derive(Clone, Copy, Debug)]
pub struct NearestRiver {
    pub distance: f32,
    pub level: f32,
    pub width: f32,
}

pub struct Hydrology {
    /// Bend the rivers off their straight pieces (see `MEANDER`).
    meander_x: Fractal,
    meander_z: Fractal,
    origin: f64,
    n: usize,
    /// Height of the large shape at each cell.
    ground: Vec<f32>,
    /// Water level of each cell's lake, where it lies in one.
    lake: Vec<Option<f32>>,
    /// Water gathered at each cell (see `RIVER_FLOW`).
    flow: Vec<f32>,
    /// River pieces by the chunk squares they come near.
    segments: HashMap<(i32, i32), Vec<Segment>>,
}

impl Hydrology {
    /// The plan of the world `shape` describes.
    pub fn new(seed: u64, shape: &WorldShape) -> Self {
        let size = shape.settings.size;
        let n = (size * 1.1 / CELL).ceil() as usize;
        let origin = -(n as f64) * CELL * 0.5;
        let centre = |i: usize| origin + (i as f64 + 0.5) * CELL;
        // The large shape and the humidity at every cell, on all cores.
        let mut samples = vec![(0.0f32, 0.0f32); n * n];
        let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
        let rows = n.div_ceil(threads);
        std::thread::scope(|scope| {
            for (k, chunk) in samples.chunks_mut(rows * n).enumerate() {
                scope.spawn(move || {
                    for (i, s) in chunk.iter_mut().enumerate() {
                        let (row, col) = (k * rows + i / n, i % n);
                        let (x, z) = (centre(col), centre(row));
                        // The ground with its hills, so rivers follow the
                        // valleys between them.
                        let m = shape.macro_at(x, z);
                        *s = (m.height + m.hills * shape.detail(x, z), m.humidity);
                    }
                });
            }
        });
        let ground: Vec<f32> = samples.iter().map(|s| s.0).collect();

        // 1. Priority flood from the sea and the grid's edge.
        const NONE: u32 = u32::MAX;
        let mut filled = ground.clone();
        let mut down = vec![NONE; n * n];
        let mut done = vec![false; n * n];
        let mut order = Vec::with_capacity(n * n);
        let mut heap = BinaryHeap::new();
        let key = |h: f32| Reverse((h * 1000.0) as i64);
        for i in 0..n * n {
            let (x, z) = (i % n, i / n);
            if ground[i] < SEA_LEVEL || x == 0 || z == 0 || x == n - 1 || z == n - 1 {
                done[i] = true;
                heap.push((key(filled[i]), i as u32));
            }
        }
        let neighbours = |i: usize| {
            let (x, z) = ((i % n) as i64, (i / n) as i64);
            [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)]
                .into_iter()
                .map(move |(dx, dz)| (x + dx, z + dz))
                .filter(move |&(x, z)| x >= 0 && z >= 0 && x < n as i64 && z < n as i64)
                .map(move |(x, z)| z as usize * n + x as usize)
        };
        while let Some((_, i)) = heap.pop() {
            let i = i as usize;
            order.push(i);
            for j in neighbours(i) {
                if done[j] {
                    continue;
                }
                done[j] = true;
                // Water from `j` flows to `i`; where `j` lies lower, it
                // fills up (a lake), a little above `i` so it still drains.
                filled[j] = filled[j].max(filled[i] + 0.01);
                down[j] = i as u32;
                heap.push((key(filled[j]), j as u32));
            }
        }

        // 2. Flow, from the top down.
        let cell_km2 = (CELL / 1000.0).powi(2) as f32;
        let mut flow: Vec<f32> = samples.iter().map(|s| cell_km2 * (0.25 + 1.5 * s.1)).collect();
        for &i in order.iter().rev() {
            if down[i] != NONE {
                let f = flow[i];
                flow[down[i] as usize] += f;
            }
        }
        // Lakes: filled well above the ground, on land, away from the spawn,
        // and only where the basin is large and deep enough; the hills'
        // small hollows are just filled (water runs over them).
        let spawn = shape.settings.spawn_radius;
        let candidate: Vec<bool> = (0..n * n)
            .map(|i| {
                let (x, z) = (centre(i % n), centre(i / n));
                filled[i] - ground[i] > LAKE_DEPTH && filled[i] > SEA_LEVEL + 0.5 && x.hypot(z) > spawn
            })
            .collect();
        let mut lake: Vec<Option<f32>> = vec![None; n * n];
        let mut seen = vec![false; n * n];
        for start in 0..n * n {
            if !candidate[start] || seen[start] {
                continue;
            }
            // The basin: the cells of the same water level around `start`.
            let mut basin = vec![start];
            seen[start] = true;
            let mut k = 0;
            let mut deepest = 0.0f32;
            while k < basin.len() {
                let i = basin[k];
                k += 1;
                deepest = deepest.max(filled[i] - ground[i]);
                for j in neighbours(i) {
                    if candidate[j] && !seen[j] && (filled[j] - filled[i]).abs() < 0.5 {
                        seen[j] = true;
                        basin.push(j);
                    }
                }
            }
            if basin.len() >= MIN_LAKE_CELLS && deepest >= MIN_LAKE_DEPTH {
                for &i in &basin {
                    lake[i] = Some(filled[i]);
                }
            }
        }

        // 3. Rivers.
        let point = |i: usize| -> [f64; 2] {
            // Shifted up to a third of a cell, the same for every river
            // through the cell.
            let h = field_seed(seed, "river point") ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
            let h = field_seed(h, "jitter");
            let u = |shift: u32| ((h >> shift) & 0xFFFF) as f64 / 65535.0 - 0.5;
            [centre(i % n) + u(0) * CELL * 0.66, centre(i / n) + u(16) * CELL * 0.66]
        };
        let width_of = |i: usize, at: [f64; 2]| -> f32 {
            let w = (MIN_WIDTH + 5.0 * (flow[i] - RIVER_FLOW).max(0.0).sqrt()).min(MAX_WIDTH);
            // Rivers thin out and end near the spawn area.
            w * smoothstep(spawn * 0.6, spawn * 1.1, at[0].hypot(at[1])) as f32
        };
        let is_river = |i: usize| {
            flow[i] >= RIVER_FLOW && down[i] != NONE && ground[i] >= SEA_LEVEL && lake[i].is_none()
        };
        // The water's level at each river cell: the plan's, but never above
        // the ground beside the river down to the next cell (the plan only
        // knows the ground at cell centres, and a river standing above its
        // banks would need dykes), and never above the level upstream.
        let natural = |x: f64, z: f64| {
            let m = shape.macro_at(x, z);
            m.height + m.hills * shape.detail(x, z)
        };
        let mut level = vec![f32::INFINITY; n * n];
        for &i in order.iter().rev() {
            if !is_river(i) {
                continue;
            }
            let j = down[i] as usize;
            let (a, b) = (point(i), point(j));
            let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
            let length = dx.hypot(dz).max(1e-6);
            let (nx, nz) = (-dz / length, dx / length);
            let side = width_of(i, a).max(width_of(i, b)) as f64 * 0.5 + 3.0 + MEANDER;
            let steps = (length / 8.0).ceil() as usize;
            let mut lowest = f32::INFINITY;
            for k in 0..steps {
                let t = k as f64 / steps as f64;
                let (x, z) = (a[0] + dx * t, a[1] + dz * t);
                for o in [-side, 0.0, side] {
                    lowest = lowest.min(natural(x + nx * o, z + nz * o));
                }
            }
            level[i] = level[i].min(filled[i]).min(lowest - 0.4).max(SEA_LEVEL);
            if is_river(j) {
                level[j] = level[j].min(level[i]);
            }
        }
        let mut segments: HashMap<(i32, i32), Vec<Segment>> = HashMap::new();
        for i in 0..n * n {
            if !is_river(i) {
                continue;
            }
            let j = down[i] as usize;
            let (a, b) = (point(i), point(j));
            let (wa, wb) = (width_of(i, a), width_of(i, b));
            if wa <= 0.5 && wb <= 0.5 {
                continue;
            }
            // Into the sea or a lake: down to its level.
            let end = if ground[j] < SEA_LEVEL {
                SEA_LEVEL
            } else if let Some(lake) = lake[j] {
                lake
            } else {
                level[j]
            };
            let segment = Segment {
                a,
                b,
                level: [level[i], end.min(level[i])],
                width: [wa, wb],
            };
            let reach = segment.reach() + MEANDER + COLUMN_REACH;
            let chunk = CHUNK_SIZE as f64;
            let (x0, x1) = ((a[0].min(b[0]) - reach) / chunk, (a[0].max(b[0]) + reach) / chunk);
            let (z0, z1) = ((a[1].min(b[1]) - reach) / chunk, (a[1].max(b[1]) + reach) / chunk);
            for cz in z0.floor() as i32..=z1.floor() as i32 {
                for cx in x0.floor() as i32..=x1.floor() as i32 {
                    segments.entry((cx, cz)).or_default().push(segment);
                }
            }
        }
        Self {
            meander_x: Fractal::new(seed, "meander x", 2, MEANDER_WAVELENGTH),
            meander_z: Fractal::new(seed, "meander z", 2, MEANDER_WAVELENGTH),
            origin,
            n,
            ground,
            lake,
            flow,
            segments,
        }
    }

    /// The plan for `seed` and `shape`, shared with generators made before
    /// for the same world (the editor makes one again on every change of a
    /// setting, and tests make many).
    pub fn shared(seed: u64, shape: &WorldShape) -> Arc<Self> {
        static PLANS: Mutex<Vec<(u64, WorldSettings, Arc<Hydrology>)>> = Mutex::new(Vec::new());
        if let Some((_, _, plan)) = PLANS
            .lock()
            .unwrap()
            .iter()
            .find(|(s, settings, _)| *s == seed && *settings == shape.settings)
        {
            return plan.clone();
        }
        let plan = Arc::new(Self::new(seed, shape));
        let mut plans = PLANS.lock().unwrap();
        plans.push((seed, shape.settings.clone(), plan.clone()));
        if plans.len() > 4 {
            plans.remove(0);
        }
        plan
    }

    fn cell(&self, x: f64, z: f64) -> Option<usize> {
        let (i, j) = (((x - self.origin) / CELL).floor(), ((z - self.origin) / CELL).floor());
        (i >= 0.0 && j >= 0.0 && (i as usize) < self.n && (j as usize) < self.n).then(|| j as usize * self.n + i as usize)
    }

    /// The river pieces that may reshape the ground anywhere in chunk
    /// square `(cx, cz)` or within `COLUMN_REACH` of it.
    pub fn segments_near(&self, cx: i32, cz: i32) -> &[Segment] {
        self.segments.get(&(cx, cz)).map_or(&[], Vec::as_slice)
    }

    /// Where world `(x, z)` lies for the rivers: moved by a smooth field,
    /// so the straight pieces between the plan's cells wind.
    fn bent(&self, x: f64, z: f64) -> (f64, f64) {
        (
            x + MEANDER * self.meander_x.at(x, z).clamp(-1.0, 1.0),
            z + MEANDER * self.meander_z.at(x, z).clamp(-1.0, 1.0),
        )
    }

    /// The nearest river to world `(x, z)` among `segments`, if one is
    /// near enough to matter.
    pub fn nearest(&self, segments: &[Segment], x: f64, z: f64) -> Option<NearestRiver> {
        if segments.is_empty() {
            return None;
        }
        let (x, z) = self.bent(x, z);
        let mut best: Option<NearestRiver> = None;
        for s in segments {
            let (d, t) = s.distance(x, z);
            if d > s.reach() {
                continue;
            }
            if best.is_none_or(|b| (d as f32) < b.distance) {
                let lerp = |v: [f32; 2]| v[0] + (v[1] - v[0]) * t as f32;
                best = Some(NearestRiver {
                    distance: d as f32,
                    level: lerp(s.level),
                    width: lerp(s.width),
                });
            }
        }
        best
    }

    /// The ground `h` (metres) at a point `river` away from the nearest
    /// river: a bed under its water, and banks that rise from the water
    /// to the land's height.
    pub fn carve(h: f32, river: Option<NearestRiver>) -> f32 {
        let Some(r) = river else {
            return h;
        };
        let half = r.width * 0.5;
        if half <= 0.0 {
            return h;
        }
        let bank = r.level + 0.6;
        if r.distance < half {
            // A rounded channel: deepest in the middle.
            let t = r.distance / half;
            let bed = r.level - depth(r.width) * (1.0 - t * t);
            return bed.min(h.min(bank));
        }
        let s = smoothstep(0.0, bank_width(r.width) as f64, (r.distance - half) as f64) as f32;
        bank + (h - bank) * s
    }

    /// The lake near world `(x, z)`: its water level and how far into it
    /// the point lies, from 0 (on its outer edge) to 1 (well inside), blended
    /// between the plan's cells so its shore does not follow the grid.
    pub fn lake_at(&self, x: f64, z: f64) -> Option<(f32, f32)> {
        // Between the four cell centres around the point.
        let gx = (x - self.origin) / CELL - 0.5;
        let gz = (z - self.origin) / CELL - 0.5;
        let (fx, fz) = (gx.floor(), gz.floor());
        let (tx, tz) = ((gx - fx) as f32, (gz - fz) as f32);
        let mut level: Option<f32> = None;
        let mut inside = 0.0;
        for (dz, wz) in [(0, 1.0 - tz), (1, tz)] {
            for (dx, wx) in [(0, 1.0 - tx), (1, tx)] {
                let (i, j) = (fx as i64 + dx, fz as i64 + dz);
                if i < 0 || j < 0 || i >= self.n as i64 || j >= self.n as i64 {
                    continue;
                }
                if let Some(l) = self.lake[j as usize * self.n + i as usize] {
                    level = Some(level.map_or(l, |m: f32| m.max(l)));
                    inside += wx * wz;
                }
            }
        }
        // Smoothed, so the shore curves instead of running in straight
        // lines between cell centres.
        level.map(|l| (l, inside * inside * (3.0 - 2.0 * inside)))
    }

    /// The ground `h` around a lake at `level`, `inside` it as `lake_at`
    /// says: under the water a floor deepening toward the middle, around it
    /// a shore at least a little above the water, so the lake does not
    /// spill over low ground near it.
    pub fn shape_lake(h: f32, level: f32, inside: f32) -> f32 {
        let s = |a: f32, b: f32| smoothstep(a as f64, b as f64, inside as f64) as f32;
        let target = level + 2.0 - 12.0 * s(LAKE_WATER, 0.9);
        let raise = s(0.0, LAKE_WATER);
        let lower = s(LAKE_SHORE - 0.15, LAKE_SHORE + 0.1);
        let raised = h + (h.max(target) - h) * raise;
        raised + (raised.min(target) - raised) * lower
    }

    /// How much a lake keeps a river's banks from raising the ground there
    /// (1: not at all, well inside the lake).
    pub fn lake_keeps_floor(inside: f32) -> f32 {
        smoothstep((LAKE_SHORE - 0.15) as f64, (LAKE_SHORE + 0.1) as f64, inside as f64) as f32
    }

    /// Whether a river runs within `half` metres of world `(x, z)` or
    /// covers it (for maps, whose pixels are wider than most rivers).
    pub fn river_near(&self, x: f64, z: f64, half: f64) -> bool {
        let c = |v: f64| (v / CHUNK_SIZE as f64).floor() as i32;
        let segments = self.segments_near(c(x), c(z));
        let (x, z) = self.bent(x, z);
        segments.iter().any(|s| {
            let (d, t) = s.distance(x, z);
            let width = s.width[0] + (s.width[1] - s.width[0]) * t as f32;
            width > 0.5 && d < half.max(width as f64 * 0.5)
        })
    }

    /// Water gathered at world `(x, z)` (for maps).
    pub fn flow(&self, x: f64, z: f64) -> f32 {
        self.cell(x, z).map_or(0.0, |i| self.flow[i])
    }

    /// The large shape's height the plan was made from at `(x, z)`.
    pub fn plan_ground(&self, x: f64, z: f64) -> Option<f32> {
        self.cell(x, z).map(|i| self.ground[i])
    }

    pub fn river_flow_threshold() -> f32 {
        RIVER_FLOW
    }

    /// Every river piece, once (from the lists by chunk square).
    pub fn pieces(&self) -> Vec<Segment> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for list in self.segments.values() {
            for s in list {
                if seen.insert((s.a[0].to_bits(), s.a[1].to_bits())) {
                    out.push(*s);
                }
            }
        }
        out
    }

    /// The centres of the lake cells and their water levels.
    pub fn lake_cells(&self) -> impl Iterator<Item = ([f64; 2], f32)> + '_ {
        let centre = move |i: usize| self.origin + (i as f64 + 0.5) * CELL;
        (0..self.n * self.n).filter_map(move |i| self.lake[i].map(|l| ([centre(i % self.n), centre(i / self.n)], l)))
    }

    /// How many river pieces and lake cells the plan has (for tests).
    pub fn counts(&self) -> (usize, usize) {
        let mut pieces = std::collections::HashSet::new();
        for list in self.segments.values() {
            for s in list {
                pieces.insert((s.a[0].to_bits(), s.a[1].to_bits()));
            }
        }
        (pieces.len(), self.lake.iter().filter(|l| l.is_some()).count())
    }

    /// The most water gathered at any cell on land.
    pub fn max_flow(&self) -> f32 {
        (0..self.n * self.n)
            .filter(|&i| self.ground[i] > SEA_LEVEL)
            .map(|i| self.flow[i])
            .fold(0.0, f32::max)
    }
}

/// How far beyond a chunk square its column reaches (its margin), and a
/// little more.
pub const COLUMN_REACH: f64 = 12.0;
