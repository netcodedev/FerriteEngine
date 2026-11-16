//! Settlements and the roads between them, planned over the whole world at
//! once, after the rivers.
//!
//! 1. **Settlements** go where the land is flat, dry and not too high or
//!    cold, the better near a river; they keep their distance from each
//!    other. One stands at the spawn.
//! 2. **Which settlements are joined**: two are, unless a third lies closer
//!    to both than they are to each other (a relative neighbourhood graph):
//!    a network without long detours or crossing shortcuts.
//! 3. **Where the roads run**: the cheapest way over a coarse grid, where
//!    climbing costs much, the sea and lakes cannot be crossed and a river
//!    only at a price. Roads already laid are cheaper, so they merge.
//! 4. The way is **smoothed** into curves, and its height along it too, so
//!    the road climbs gently and cuts into hills or runs on embankments.
//!
//! The ground then gets the settlements' level grounds and the roads'
//! beds worked in (see `WorldGenerator::shaped`), and their colours.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    sync::{Arc, Mutex},
};

use super::{
    hydrology::Hydrology,
    noise::field_seed,
    world::{smoothstep, WorldShape},
    WorldSettings, SEA_LEVEL,
};
use crate::terrain::CHUNK_SIZE;

/// Metres between the plan's samples.
const CELL: f64 = 64.0;
/// Half a road's width, metres.
pub const ROAD_HALF_WIDTH: f32 = 3.5;
/// The road's shoulder: over how many metres beside it the ground goes
/// back to its own height, at least and at most (more where the road cuts
/// deep or runs high: a slope of about 1 in 1.5).
const SHOULDER: f32 = 6.0;
const MAX_SHOULDER: f32 = 36.0;
/// How far a settlement's level ground blends back into the land.
const SETTLEMENT_EDGE: f32 = 50.0;
/// Fewest metres between two settlements, and how many there are at most.
const SETTLEMENT_SPACING: f64 = 2600.0;
const MAX_SETTLEMENTS: usize = 48;
/// Longest road between two settlements, as the crow flies.
const MAX_ROAD: f64 = 9000.0;
/// Steepest climb of a road, metres per metre.
const MAX_GRADE: f32 = 0.12;
/// What crossing a river costs, in metres of road.
const RIVER_CROSSING: f32 = 700.0;

/// A settlement: for now, a level ground for its houses to stand on.
#[derive(Clone, Copy, Debug)]
pub struct Settlement {
    pub centre: [f64; 2],
    /// Metres; the ground is level to here and blends back beyond.
    pub radius: f32,
    /// Height of its ground.
    pub level: f32,
}

impl Settlement {
    fn reach(&self) -> f64 {
        (self.radius + SETTLEMENT_EDGE) as f64
    }
}

/// A piece of road with the height of its surface at each end.
#[derive(Clone, Copy, Debug)]
pub struct RoadSegment {
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub level: [f32; 2],
}

impl RoadSegment {
    /// Whether the piece may shape the ground anywhere along world row `z`.
    pub fn reaches_row(&self, z: f64) -> bool {
        let reach = (ROAD_HALF_WIDTH + MAX_SHOULDER) as f64;
        z >= self.a[1].min(self.b[1]) - reach && z <= self.a[1].max(self.b[1]) + reach
    }

    /// Distance from `(x, z)` to the piece, and how far along it (0..1)
    /// the nearest point lies.
    fn distance(&self, x: f64, z: f64) -> (f64, f64) {
        let (dx, dz) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        let length2 = dx * dx + dz * dz;
        let t = if length2 > 0.0 {
            (((x - self.a[0]) * dx + (z - self.a[1]) * dz) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((x - self.a[0] - dx * t).hypot(z - self.a[1] - dz * t), t)
    }
}

/// The nearest road to a point.
#[derive(Clone, Copy, Debug)]
pub struct NearestRoad {
    pub distance: f32,
    pub level: f32,
}

/// What the plan puts on the ground near a chunk square.
#[derive(Default)]
pub struct Local {
    pub roads: Vec<RoadSegment>,
    pub settlements: Vec<Settlement>,
}

pub struct Roads {
    settlements: Vec<Settlement>,
    /// Every road's way, as points with the road's height.
    ways: Vec<Vec<([f64; 2], f32)>>,
    /// What lies near each chunk square.
    local: HashMap<(i32, i32), Local>,
}

/// How far beyond a chunk square its column reaches, and a little more.
const COLUMN_REACH: f64 = 12.0;

impl Roads {
    /// The plan of the world `shape` describes, around its rivers.
    pub fn new(seed: u64, shape: &WorldShape, rivers: Option<&Hydrology>) -> Self {
        let size = shape.settings.size;
        let n = (size / CELL).ceil() as usize;
        let origin = -(n as f64) * CELL * 0.5;
        let centre = |i: usize| origin + (i as f64 + 0.5) * CELL;
        let natural = |x: f64, z: f64| {
            let m = shape.macro_at(x, z);
            (m.height + m.hills * shape.detail(x, z), m.temperature, m.humidity)
        };
        // The ground at every cell, on all cores.
        let mut samples = vec![(0.0f32, 0.0f32, 0.0f32); n * n];
        let threads = std::thread::available_parallelism().map_or(4, |t| t.get());
        let rows = n.div_ceil(threads);
        std::thread::scope(|scope| {
            for (k, chunk) in samples.chunks_mut(rows * n).enumerate() {
                scope.spawn(move || {
                    for (i, s) in chunk.iter_mut().enumerate() {
                        *s = natural(centre(i % n), centre(k * rows + i / n));
                    }
                });
            }
        });
        let ground = |i: usize| samples[i].0;
        let lake = |x: f64, z: f64| {
            rivers
                .and_then(|r| r.lake_at(x, z))
                .is_some_and(|(_, inside)| inside > 0.2)
        };
        let river = |x: f64, z: f64, near: f64| rivers.is_some_and(|r| r.river_near(x, z, near));
        // What a cell is to a road: open land, water or a river.
        #[derive(Clone, Copy, PartialEq)]
        enum Ground {
            Land,
            Water,
            River,
        }
        let ground_kind = |i: usize| {
            let (x, z) = (centre(i % n), centre(i / n));
            if ground(i) < SEA_LEVEL + 1.0 || lake(x, z) {
                Ground::Water
            } else if river(x, z, CELL * 0.5) {
                Ground::River
            } else {
                Ground::Land
            }
        };
        let mut kind = vec![Ground::Water; n * n];
        std::thread::scope(|scope| {
            for (k, chunk) in kind.chunks_mut(rows * n).enumerate() {
                let ground_kind = &ground_kind;
                scope.spawn(move || {
                    for (i, kind) in chunk.iter_mut().enumerate() {
                        *kind = ground_kind(k * rows * n + i);
                    }
                });
            }
        });
        let neighbours = |i: usize| {
            let (x, z) = ((i % n) as i64, (i / n) as i64);
            [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)]
                .into_iter()
                .map(move |(dx, dz)| (x + dx, z + dz))
                .filter(move |&(x, z)| x >= 0 && z >= 0 && x < n as i64 && z < n as i64)
                .map(move |(x, z)| z as usize * n + x as usize)
        };
        let slope = |i: usize| {
            neighbours(i)
                .map(|j| (ground(j) - ground(i)).abs() / CELL as f32)
                .fold(0.0f32, f32::max)
        };

        // 1. Settlements.
        let spawn = shape.settings.spawn_radius;
        let hash = |i: usize| {
            let h = field_seed(seed, "settlement") ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
            (field_seed(h, "score") & 0xFFFF) as f32 / 65535.0
        };
        let mut candidates: Vec<(f32, usize)> = (0..n * n)
            .filter_map(|i| {
                let (x, z) = (centre(i % n), centre(i / n));
                let (h, temperature, humidity) = samples[i];
                let s = slope(i);
                if kind[i] != Ground::Land || !(4.0..=600.0).contains(&h) || s > 0.07 || temperature < 0.0 {
                    return None;
                }
                let river_within = |r: f64| rivers.is_some_and(|p| p.river_within(x, z, r));
                if x.hypot(z) < spawn * 3.0 || river_within(160.0) {
                    return None;
                }
                let flat = 1.0 - s / 0.07;
                let water = if river_within(700.0) { 0.6 } else { 0.0 };
                let mild = 1.0 - ((temperature - 14.0) / 14.0).abs().min(1.0);
                Some((flat + water + 0.4 * mild + 0.3 * humidity + 0.5 * hash(i), i))
            })
            .collect();
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        let level_around = |c: [f64; 2], radius: f64| {
            let mut sum = natural(c[0], c[1]).0;
            for k in 0..8 {
                let a = k as f64 * std::f64::consts::FRAC_PI_4;
                sum += natural(c[0] + radius * a.cos(), c[1] + radius * a.sin()).0;
            }
            (sum / 9.0).max(SEA_LEVEL + 2.0)
        };
        // One at the spawn, in the middle of its gentle land.
        let mut settlements = vec![Settlement {
            centre: [0.0, 0.0],
            radius: 110.0,
            level: level_around([0.0, 0.0], 80.0),
        }];
        for &(score, i) in &candidates {
            if settlements.len() >= MAX_SETTLEMENTS {
                break;
            }
            let c = [centre(i % n), centre(i / n)];
            if settlements
                .iter()
                .any(|s| (s.centre[0] - c[0]).hypot(s.centre[1] - c[1]) < SETTLEMENT_SPACING)
            {
                continue;
            }
            // The best places become towns, the rest villages.
            let radius = if settlements.len() < 6 { 170.0 } else { 90.0 + 30.0 * score.min(2.0) };
            settlements.push(Settlement {
                centre: c,
                radius,
                level: level_around(c, radius as f64 * 0.7),
            });
        }

        // 2. Which settlements are joined.
        let distance = |a: &Settlement, b: &Settlement| (a.centre[0] - b.centre[0]).hypot(a.centre[1] - b.centre[1]);
        let mut links = Vec::new();
        for a in 0..settlements.len() {
            for b in a + 1..settlements.len() {
                let d = distance(&settlements[a], &settlements[b]);
                if d > MAX_ROAD {
                    continue;
                }
                let shadowed = (0..settlements.len()).any(|c| {
                    c != a
                        && c != b
                        && distance(&settlements[a], &settlements[c]).max(distance(&settlements[b], &settlements[c])) < d
                });
                if !shadowed {
                    links.push((d, a, b));
                }
            }
        }
        // Short roads first: long ones then follow them where they can.
        links.sort_by(|x, y| x.0.total_cmp(&y.0));

        // 3. Where the roads run.
        let cell_of = |p: [f64; 2]| {
            let i = (((p[0] - origin) / CELL).floor() as i64).clamp(0, n as i64 - 1) as usize;
            let j = (((p[1] - origin) / CELL).floor() as i64).clamp(0, n as i64 - 1) as usize;
            j * n + i
        };
        let mut road = vec![false; n * n];
        let mut beside = vec![false; n * n];
        let mut ways = Vec::new();
        for &(_, a, b) in &links {
            let (start, goal) = (cell_of(settlements[a].centre), cell_of(settlements[b].centre));
            let Some(cells) = find_way(n, start, goal, |i, j| {
                if kind[j] == Ground::Water && j != goal && j != start {
                    return None;
                }
                let (di, dj) = ((i % n) as f32 - (j % n) as f32, (i / n) as f32 - (j / n) as f32);
                let length = di.hypot(dj) * CELL as f32;
                let grade = (ground(j) - ground(i)).abs() / length;
                if grade > 0.35 {
                    return None;
                }
                let mut cost = length * (1.0 + 4.0 * grade + 200.0 * grade * grade);
                if kind[j] == Ground::River {
                    cost += RIVER_CROSSING;
                }
                // Roads merge: on one, cheaper; right beside one, dearer.
                if road[j] {
                    cost *= 0.4;
                } else if beside[j] {
                    cost *= 1.6;
                }
                Some(cost)
            }) else {
                continue;
            };
            for &c in &cells {
                road[c] = true;
                for j in neighbours(c) {
                    beside[j] = true;
                }
            }
            // 4. Smoothed: the settlements' centres at the ends, curves
            // between the cells.
            // Within the settlements, straight to their centres (the
            // cells there would wind around them).
            let inside = |p: &[f64; 2], s: &Settlement| {
                (p[0] - s.centre[0]).hypot(p[1] - s.centre[1]) < s.radius as f64 * 0.9
            };
            let mut points = vec![settlements[a].centre];
            points.extend(
                cells
                    .iter()
                    .map(|&c| [centre(c % n), centre(c / n)])
                    .filter(|p| !inside(p, &settlements[a]) && !inside(p, &settlements[b])),
            );
            points.push(settlements[b].centre);
            for _ in 0..3 {
                points = chaikin(&points);
            }
            // The road's height: the ground's, evened out over about
            // 100 m, and the settlements' level in and near them.
            let raw: Vec<f32> = points.iter().map(|p| natural(p[0], p[1]).0).collect();
            let mut level = raw.clone();
            for _ in 0..3 {
                level = (0..level.len())
                    .map(|k| {
                        let (lo, hi) = (k.saturating_sub(6), (k + 6).min(level.len() - 1));
                        level[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
                    })
                    .collect();
            }
            // Level with every settlement the road passes: fully inside
            // (`pinned`), blending out beyond.
            let mut pinned = vec![None; points.len()];
            let mut level: Vec<f32> = points
                .iter()
                .zip(&level)
                .enumerate()
                .map(|(k, (p, &l))| {
                    let mut l = l.max(SEA_LEVEL + 1.5);
                    for s in &settlements {
                        let d = (p[0] - s.centre[0]).hypot(p[1] - s.centre[1]);
                        if d > s.reach() + 40.0 {
                            continue;
                        }
                        let w = 1.0 - smoothstep(s.radius as f64 * 0.8, s.reach() + 40.0, d) as f32;
                        l += (s.level - l) * w;
                        if d < s.radius as f64 {
                            pinned[k] = Some(s.level);
                        }
                    }
                    l
                })
                .collect();
            // Then no steeper than a road climbs, the settlements kept.
            for _ in 0..16 {
                limit_grade(&points, &mut level);
                for (l, p) in level.iter_mut().zip(&pinned) {
                    if let Some(p) = p {
                        *l = *p;
                    }
                }
            }
            let way = points.into_iter().zip(level).collect::<Vec<_>>();
            ways.push(way);
        }

        // What lies near each chunk square.
        let chunk = CHUNK_SIZE as f64;
        let mut local: HashMap<(i32, i32), Local> = HashMap::new();
        let squares = |lo: [f64; 2], hi: [f64; 2], reach: f64| {
            let r = |v: f64| (v / chunk).floor() as i32;
            let (x0, x1, z0, z1) = (r(lo[0] - reach), r(hi[0] + reach), r(lo[1] - reach), r(hi[1] + reach));
            (z0..=z1).flat_map(move |z| (x0..=x1).map(move |x| (x, z)))
        };
        for s in &settlements {
            for key in squares(s.centre, s.centre, s.reach() + COLUMN_REACH) {
                local.entry(key).or_default().settlements.push(*s);
            }
        }
        for way in &ways {
            for pair in way.windows(2) {
                let ((a, la), (b, lb)) = (pair[0], pair[1]);
                let segment = RoadSegment { a, b, level: [la, lb] };
                let lo = [a[0].min(b[0]), a[1].min(b[1])];
                let hi = [a[0].max(b[0]), a[1].max(b[1])];
                let reach = (ROAD_HALF_WIDTH + MAX_SHOULDER) as f64 + COLUMN_REACH;
                for key in squares(lo, hi, reach) {
                    local.entry(key).or_default().roads.push(segment);
                }
            }
        }
        Self { settlements, ways, local }
    }

    /// The plan for `seed` and `shape`, shared with generators made before
    /// for the same world (see `Hydrology::shared`).
    pub fn shared(seed: u64, shape: &WorldShape, rivers: Option<&Hydrology>) -> Arc<Self> {
        static PLANS: Mutex<Vec<(u64, WorldSettings, Arc<Roads>)>> = Mutex::new(Vec::new());
        if let Some((_, _, plan)) = PLANS
            .lock()
            .unwrap()
            .iter()
            .find(|(s, settings, _)| *s == seed && *settings == shape.settings)
        {
            return plan.clone();
        }
        let plan = Arc::new(Self::new(seed, shape, rivers));
        let mut plans = PLANS.lock().unwrap();
        plans.push((seed, shape.settings.clone(), plan.clone()));
        if plans.len() > 4 {
            plans.remove(0);
        }
        plan
    }

    pub fn settlements(&self) -> &[Settlement] {
        &self.settlements
    }

    /// Every road's way: points and the road's height there.
    pub fn ways(&self) -> &[Vec<([f64; 2], f32)>] {
        &self.ways
    }

    /// What may shape the ground anywhere in chunk square `(cx, cz)` or
    /// within its column's margins.
    pub fn near(&self, cx: i32, cz: i32) -> Option<&Local> {
        self.local.get(&(cx, cz))
    }

    /// The nearest road to world `(x, z)` among `roads`, within reach.
    pub fn nearest(roads: &[RoadSegment], x: f64, z: f64) -> Option<NearestRoad> {
        let reach = (ROAD_HALF_WIDTH + MAX_SHOULDER) as f64;
        let mut best: Option<NearestRoad> = None;
        for s in roads {
            let (d, t) = s.distance(x, z);
            if d > reach {
                continue;
            }
            if best.is_none_or(|b| (d as f32) < b.distance) {
                best = Some(NearestRoad {
                    distance: d as f32,
                    level: s.level[0] + (s.level[1] - s.level[0]) * t as f32,
                });
            }
        }
        best
    }

    /// The ground `h` with `settlements` levelled in, and how
    /// much of a settlement's ground the point is (0..1).
    pub fn level_settlements(settlements: &[Settlement], x: f64, z: f64, h: f32) -> (f32, f32) {
        let mut h = h;
        let mut inside = 0.0f32;
        for s in settlements {
            let d = (x - s.centre[0]).hypot(z - s.centre[1]) as f32;
            if d > s.reach() as f32 {
                continue;
            }
            let w = 1.0 - smoothstep(s.radius as f64, s.reach(), d as f64) as f32;
            h += (s.level - h) * w;
            inside = inside.max(1.0 - smoothstep(s.radius as f64 * 0.6, s.radius as f64, d as f64) as f32);
        }
        (h, inside)
    }

    /// The ground `h` beside or under a road: its bed, then a shoulder
    /// back to the land's own height.
    pub fn bed(h: f32, road: Option<NearestRoad>) -> f32 {
        let Some(r) = road else {
            return h;
        };
        let shoulder = (SHOULDER + 1.5 * (h - r.level).abs()).min(MAX_SHOULDER);
        let s = smoothstep(ROAD_HALF_WIDTH as f64, (ROAD_HALF_WIDTH + shoulder) as f64, r.distance as f64) as f32;
        r.level + (h - r.level) * s
    }

    /// How much of the road's surface a point `road` away is (0..1): its
    /// colour, with a soft edge.
    pub fn surface(road: Option<NearestRoad>) -> f32 {
        road.map_or(0.0, |r| {
            1.0 - smoothstep((ROAD_HALF_WIDTH - 0.8) as f64, (ROAD_HALF_WIDTH + 0.6) as f64, r.distance as f64) as f32
        })
    }
}

/// The cheapest way from cell `start` to `goal` over an `n` × `n` grid,
/// where `cost(from, to)` is what stepping to a neighbour costs (`None`:
/// it cannot be entered). A*, with the distance as the estimate (no step
/// costs less than its length).
fn find_way(n: usize, start: usize, goal: usize, cost: impl Fn(usize, usize) -> Option<f32>) -> Option<Vec<usize>> {
    let estimate = |i: usize| {
        let (dx, dz) = ((i % n) as f32 - (goal % n) as f32, (i / n) as f32 - (goal / n) as f32);
        dx.hypot(dz) * CELL as f32
    };
    let mut best = vec![f32::INFINITY; n * n];
    let mut from = vec![usize::MAX; n * n];
    let mut open = BinaryHeap::new();
    best[start] = 0.0;
    open.push((Reverse((estimate(start) * 10.0) as i64), start));
    while let Some((Reverse(key), i)) = open.pop() {
        if i == goal {
            let mut way = vec![goal];
            while from[*way.last().unwrap()] != usize::MAX {
                way.push(from[*way.last().unwrap()]);
            }
            way.reverse();
            return Some(way);
        }
        let here = best[i];
        // Already reached more cheaply since this entry was queued.
        if key > ((here + estimate(i)) * 10.0) as i64 + 1 {
            continue;
        }
        let (x, z) = ((i % n) as i64, (i / n) as i64);
        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= n as i64 || nz >= n as i64 {
                continue;
            }
            let j = nz as usize * n + nx as usize;
            let Some(step) = cost(i, j) else {
                continue;
            };
            let total = here + step;
            if total < best[j] {
                best[j] = total;
                from[j] = i;
                open.push((Reverse(((total + estimate(j)) * 10.0) as i64), j));
            }
        }
    }
    None
}

/// Even out `level` along `points` so the road climbs at most `MAX_GRADE`:
/// the middle between the highest and the lowest such line that stay
/// under and over the levels, so it cuts as much as it fills.
fn limit_grade(points: &[[f64; 2]], level: &mut [f32]) {
    let run: Vec<f32> = points
        .windows(2)
        .map(|p| (p[0][0] - p[1][0]).hypot(p[0][1] - p[1][1]) as f32 * MAX_GRADE)
        .collect();
    let envelope = |up: bool| {
        let mut l = level.to_vec();
        let pick = |a: f32, b: f32| if up { a.max(b) } else { a.min(b) };
        let sign = if up { -1.0 } else { 1.0 };
        for k in 1..l.len() {
            l[k] = pick(l[k], l[k - 1] + sign * run[k - 1]);
        }
        for k in (0..l.len() - 1).rev() {
            l[k] = pick(l[k], l[k + 1] + sign * run[k]);
        }
        l
    };
    let (high, low) = (envelope(true), envelope(false));
    for (k, l) in level.iter_mut().enumerate() {
        *l = 0.5 * (high[k] + low[k]);
    }
}

/// One step of Chaikin's corner cutting: each corner replaced by two points
/// a quarter of the way along its sides. The ends stay.
fn chaikin(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len() * 2);
    out.push(points[0]);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        out.push([0.75 * a[0] + 0.25 * b[0], 0.75 * a[1] + 0.25 * b[1]]);
        out.push([0.25 * a[0] + 0.75 * b[0], 0.25 * a[1] + 0.75 * b[1]]);
    }
    out.push(*points.last().unwrap());
    out
}
