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
/// inside one): the lake's water stands from `LAKE_WATER` on, wherever the
/// land lies under it (the land, not the plan's cells, makes the shore);
/// low land up to `LAKE_EDGE` is raised to a shore, so the water does not
/// end over a pit. From `LAKE_SHORE` on the lake is well under way.
pub const LAKE_WATER: f32 = 0.02;
pub const LAKE_EDGE: f32 = 0.08;
pub const LAKE_SHORE: f32 = 0.45;
/// How much deeper a lake's floor gets than the land under its water,
/// metres, reached that far under the water (from its shore on).
const LAKE_DEEPEN: f32 = 2.0;
const LAKE_DEEPEN_OVER: f32 = 3.0;
/// Which of the heights around a lake's rim (in percent, from the lowest)
/// its water stays under.
const RIM_QUANTILE_PERCENT: usize = 10;
/// How far under the ground on its rim a lake's water stays.
const LAKE_RIM: f32 = 0.5;

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
        let w = bed_width(self.width[0].max(self.width[1]));
        // With the flat banks beside a lake (see `carve_into_lake`).
        (w * 0.5 + max_bank_width(w) + 4.0 + 0.2 * w) as f64
    }
}

/// Width of the banks beside a river `width` wide, over which the ground
/// rises from the water back to the land's own height: in proportion to
/// the river, wider (up to `max_bank_width`) where the land beside it
/// lies `rise` metres higher, so the bank does not become a wall.
fn bank_width(width: f32, rise: f32) -> f32 {
    (3.0 + 1.5 * width).max(1.5 * rise).min(max_bank_width(width))
}

/// The widest banks a river `width` wide gets (see `bank_width`).
fn max_bank_width(width: f32) -> f32 {
    16.0 + 2.5 * width
}

/// How far an end into the sea or a lake reaches on past the plan's cell
/// there, so the river's bed cuts through the beach or the lake's shore
/// into the water.
const MOUTH: f64 = 80.0;

/// How far from a lake's cell toward the first cell of the river running
/// out of it (0..1) the river stays level with the lake before it falls.
const SOURCE_SHORE: f64 = 0.5;
/// How much higher than its lake the ground between may stand for a river
/// to be cut through to the lake, metres.
const MAX_SOURCE_CUT: f32 = 8.0;

/// Narrowest bed a river a metre or more wide gets, metres: the terrain's
/// mesh samples the ground every metre, and catches a narrower channel
/// only here and there, so the water would show in pieces.
const MIN_BED: f32 = 4.0;

/// Width of the bed cut for a river `width` wide: its own, or `MIN_BED`
/// (less for the last trickles near a river's end).
fn bed_width(width: f32) -> f32 {
    width.max(MIN_BED * width.min(1.0))
}

/// How high a river's banks start over its water, metres.
const BANK: f32 = 0.6;

/// Depth of a river `width` wide in its middle: shallow for a narrow one,
/// so its bed does not become a slot.
fn depth(width: f32) -> f32 {
    (1.2 + 0.07 * width).min(0.35 * bed_width(width))
}

/// The nearest river to a point, as far as the ground cares.
#[derive(Clone, Copy, Debug)]
pub struct NearestRiver {
    pub distance: f32,
    pub level: f32,
    pub width: f32,
    /// Which way it flows (world x, z; unit length) and how steeply
    /// (metres down per metre).
    pub direction: [f32; 2],
    pub grade: f32,
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
    /// Where rivers run out of lakes: a point in the lake and the river's
    /// first point beyond it.
    outlets: Vec<([f64; 2], [f64; 2])>,
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
        // The ground with its hills at any point.
        let natural = |x: f64, z: f64| {
            let m = shape.macro_at(x, z);
            m.height + m.hills * shape.detail(x, z)
        };

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
                // No higher than the lowest ground on its rim, looked at
                // between the cells (the plan only knows the ground at
                // their middles; between them it may lie lower, and the
                // lake would stand over the land beside it, held by a wall).
                // The rim: the outer half of the way to the cells beyond,
                // where the lake's water ends (nearer, the ground lies under
                // the lake, and would sink it into a puddle).
                let in_basin: std::collections::HashSet<usize> = basin.iter().copied().collect();
                let mut samples = Vec::new();
                for &i in &basin {
                    for j in neighbours(i) {
                        if in_basin.contains(&j) {
                            continue;
                        }
                        let (a, b) = ([centre(i % n), centre(i / n)], [centre(j % n), centre(j / n)]);
                        for k in 4..=8 {
                            let t = k as f64 / 8.0;
                            samples.push(natural(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t));
                        }
                    }
                }
                // Not the very lowest: a few dips in the rim get a shore
                // raised over the water instead (see `shape_lake`), rather
                // than the whole lake sinking to a puddle under them.
                samples.sort_by(f32::total_cmp);
                let rim = samples.get(samples.len() * RIM_QUANTILE_PERCENT / 100).copied().unwrap_or(f32::INFINITY);
                for &i in &basin {
                    lake[i] = Some(filled[i].min(rim - LAKE_RIM));
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
        // The lake a river cell drains, if one flows into it: the lake's
        // cell with the most water running out. Not where the ground
        // between stands high over the lake (it drains over a saddle the
        // lake's level was lowered under, see above): the river would cut
        // a gorge to reach it.
        let source = |i: usize| -> Option<usize> {
            let l = neighbours(i)
                .filter(|&l| down[l] == i as u32 && lake[l].is_some())
                .max_by(|&p, &q| flow[p].total_cmp(&flow[q]))?;
            let (a, b) = (point(l), point(i));
            let crest = (0..=8)
                .map(|k| {
                    let t = k as f64 / 8.0;
                    natural(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
                })
                .fold(f32::NEG_INFINITY, f32::max);
            (crest < lake[l]? + MAX_SOURCE_CUT).then_some(l)
        };
        // The water's level at each river cell: the plan's, but never above
        // the ground beside the river down to the next cell (the plan only
        // knows the ground at cell centres, and a river standing above its
        // banks would need dykes), and never above the level upstream.
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
            // Never above the lake it runs out of.
            if let Some(l) = source(i) {
                level[i] = level[i].min(lake[l].unwrap_or(f32::INFINITY));
            }
            if is_river(j) {
                level[j] = level[j].min(level[i]);
            } else if ground[j] < SEA_LEVEL {
                // The last cell before the sea or a lake is already at its
                // level: the river falls before it and runs into it level,
                // instead of standing over it at the shore.
                level[i] = level[i].min(SEA_LEVEL);
            } else if let Some(lake) = lake[j] {
                level[i] = level[i].min(lake);
            }
        }
        let mut segments: HashMap<(i32, i32), Vec<Segment>> = HashMap::new();
        let mut outlets = Vec::new();
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
            let mut pieces = vec![segment];
            // Into the sea or a lake: on past the plan's cell there, level,
            // into the water (the coast or the lake's shore may lie beyond
            // the cell's middle).
            if !is_river(j) && (ground[j] < SEA_LEVEL || lake[j].is_some()) {
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let length = dx.hypot(dz).max(1e-6);
                let on = [b[0] + dx / length * MOUTH, b[1] + dz / length * MOUTH];
                let level = end.min(level[i]);
                pieces.push(Segment {
                    a: b,
                    b: on,
                    level: [level, level],
                    width: [wb, wb],
                });
            }
            // Out of a lake: from in it, level with its water out to about
            // its shore (it lies between the two cells' middles), then
            // down to the river's level, so the river leaves the lake
            // through its shore instead of starting beyond it.
            if let Some(l) = source(i) {
                let water = lake[l].unwrap_or(level[i]).max(level[i]);
                let from = point(l);
                let shore = [from[0] + (a[0] - from[0]) * SOURCE_SHORE, from[1] + (a[1] - from[1]) * SOURCE_SHORE];
                pieces.push(Segment {
                    a: from,
                    b: shore,
                    level: [water, water],
                    width: [wa, wa],
                });
                pieces.push(Segment {
                    a: shore,
                    b: a,
                    level: [water, level[i]],
                    width: [wa, wa],
                });
                outlets.push((from, a));
            }
            for segment in pieces {
                let (a, b) = (segment.a, segment.b);
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
            outlets,
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
                let (dx, dz) = (s.b[0] - s.a[0], s.b[1] - s.a[1]);
                let length = dx.hypot(dz).max(1e-6);
                best = Some(NearestRiver {
                    distance: d as f32,
                    level: lerp(s.level),
                    width: lerp(s.width),
                    direction: [(dx / length) as f32, (dz / length) as f32],
                    grade: (s.level[0] - s.level[1]) / length as f32,
                });
            }
        }
        best
    }

    /// The ground `h` (metres) at a point `river` away from the nearest
    /// river: a bed under its water, and banks that rise from the water
    /// to the land's height. It only ever cuts: where the land lies lower
    /// than the banks would (the sea floor or a beach a river runs out
    /// over), it stays as it is, or the river would run out into the sea
    /// between dykes.
    pub fn carve(h: f32, river: Option<NearestRiver>) -> f32 {
        Self::carve_with_banks(h, river, BANK, 0.0)
    }

    /// `carve`, for a river carrying a lake's own water beside or out of
    /// it: its banks start under the water and stay there a while (over
    /// the lake's shore, which would part the two), so river and lake are
    /// one.
    pub fn carve_into_lake(h: f32, river: Option<NearestRiver>) -> f32 {
        let flat = river.map_or(0.0, |r| 4.0 + 0.2 * r.width);
        // Deeper than the shore's foam reaches (`SHORE_FOAM_DEPTH` in
        // `water_fragment.glsl`), or the whole shallow foams.
        Self::carve_with_banks(h, river, -1.2, flat)
    }

    /// `carve`, with the top of the banks `bank` metres over the water,
    /// flat for `flat` metres before they rise.
    fn carve_with_banks(h: f32, river: Option<NearestRiver>, bank: f32, flat: f32) -> f32 {
        let cut = Self::shape_by_river(h, river, bank, flat).min(h);
        h + (cut - h) * (1.0 - Self::submerged(h, river))
    }

    /// How far the land `h` already lies under `river`'s water, 0 (not)
    /// to 1 (as deep as the river's bed): past its mouth, on the floor of
    /// the sea or a lake, the bed and its gravel fade out instead of
    /// drawing a trench (and a dark trail) into the water.
    fn submerged(h: f32, river: Option<NearestRiver>) -> f32 {
        river.map_or(0.0, |r| smoothstep(0.0, depth(r.width) as f64, (r.level - h) as f64) as f32)
    }

    fn shape_by_river(h: f32, river: Option<NearestRiver>, bank: f32, flat: f32) -> f32 {
        let Some(r) = river else {
            return h;
        };
        let width = bed_width(r.width);
        let half = width * 0.5;
        if half <= 0.0 {
            return h;
        }
        let bank = r.level + bank;
        if r.distance < half {
            // A rounded channel: deepest in the middle.
            let t = r.distance / half;
            let bed = r.level - depth(r.width) * (1.0 - t * t);
            return bed.min(h.min(bank));
        }
        let s = smoothstep(0.0, bank_width(width, h - bank) as f64, (r.distance - half - flat) as f64) as f32;
        bank + (h - bank) * s
    }

    /// How much of the ground is gravel, 0..1: river beds and a strip of
    /// each bank (wider for wider rivers), and lake floors and shores.
    /// `river` the nearest river, `lake` what `lake_at` says, `h` the
    /// ground before the river shaped it.
    pub fn gravel(river: Option<NearestRiver>, lake: Option<(f32, f32)>, h: f32) -> f32 {
        let by_river = river.map_or(0.0, |r| {
            let edge = (r.width * 0.5 + 1.5) as f64;
            let reach = edge + 1.0 + 0.4 * r.width as f64;
            (1.0 - smoothstep(edge, reach, r.distance as f64) as f32) * (1.0 - Self::submerged(h, river))
        });
        // The lake's floor and the shore just over its water.
        let by_lake = lake.map_or(0.0, |(level, inside)| {
            let near = smoothstep(0.0, LAKE_EDGE as f64, inside as f64) as f32;
            near * (1.0 - smoothstep((level + 0.4) as f64, (level + 1.5) as f64, h as f64) as f32)
        });
        by_river.max(by_lake)
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
    /// says. The land as it lies makes the shore: the lake fills it up to
    /// its level and eats nothing away, only its floor gets a little deeper
    /// away from the shore; and at its outer edge low land is raised to a
    /// shore just over the water, so the lake does not spill over it.
    pub fn shape_lake(h: f32, level: f32, inside: f32) -> f32 {
        let s = |a: f32, b: f32| smoothstep(a as f64, b as f64, inside as f64) as f32;
        let under = level - h;
        let h = h - LAKE_DEEPEN * smoothstep(0.0, LAKE_DEEPEN_OVER as f64, under as f64) as f32;
        let shore = level + 0.3;
        h + (h.max(shore) - h) * (1.0 - s(LAKE_EDGE * 0.4, LAKE_EDGE))
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

    /// Whether a river cell of the plan lies within `radius` metres of
    /// world `(x, z)` (coarse: by the plan's cells).
    pub fn river_within(&self, x: f64, z: f64, radius: f64) -> bool {
        let Some(i) = self.cell(x, z) else {
            return false;
        };
        let (cx, cz) = ((i % self.n) as i64, (i / self.n) as i64);
        let r = (radius / CELL).ceil() as i64;
        for dz in -r..=r {
            for dx in -r..=r {
                let (px, pz) = (cx + dx, cz + dz);
                if px < 0 || pz < 0 || px >= self.n as i64 || pz >= self.n as i64 {
                    continue;
                }
                if ((dx * dx + dz * dz) as f64).sqrt() * CELL > radius + CELL * 0.5 {
                    continue;
                }
                let j = pz as usize * self.n + px as usize;
                if self.flow[j] >= RIVER_FLOW && self.ground[j] >= SEA_LEVEL && self.lake[j].is_none() {
                    return true;
                }
            }
        }
        false
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
    /// Where rivers run out of lakes (see `outlets` on the struct).
    pub fn outlets(&self) -> &[([f64; 2], [f64; 2])] {
        &self.outlets
    }

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
