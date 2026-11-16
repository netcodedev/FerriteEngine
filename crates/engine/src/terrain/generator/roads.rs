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
//!    climbing costs much and no step climbs more than a road can (so it
//!    winds up a mountain in turns), the sea and lakes cannot be crossed
//!    and a river only at a price. Roads already laid are cheaper, so they
//!    merge.
//! 4. The way is **smoothed** into curves, and its height along it too, so
//!    the road climbs gently and cuts into hills or runs on embankments,
//!    never more than `MAX_EMBANKMENT` from the ground's own height.
//!
//! 5. Where a road winds a long way round or up a mountain that a short,
//!    gentle, straight **tunnel** could cross, it takes the tunnel. Where
//!    enough rock lies over it, the tunnel is a tube cut into the ground
//!    (see `tunnel_air`); toward its ends the road runs in a cutting into
//!    the hillside, up to the portal.
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
/// Beside a road the ground is held within its slopes: in a cutting it
/// rises at most this steeply from the road's edge, under an embankment it
/// falls at most this steeply (metres per metre). Where the slopes meet the
/// land, the land's own height takes over.
const CUT_SLOPE: f32 = 1.2;
const FILL_SLOPE: f32 = 0.7;
/// On land steeper than those, how much steeper than the land they are,
/// and how steep at most.
const STEEPER: f32 = 1.2;
const MAX_FACE: f32 = 2.0;
/// The ground just beside a road counted as its shoulder (see `worked`).
const SHOULDER: f32 = 6.0;
/// How far beside a road its slopes reach at most; they fade out over the
/// last `SLOPE_FADE` metres where the land is steeper than they are.
const MAX_SHOULDER: f32 = 36.0;
const SLOPE_FADE: f32 = 12.0;
/// How far a settlement's level ground blends back into the land.
const SETTLEMENT_EDGE: f32 = 50.0;
/// Fewest metres between two settlements, and how many there are at most.
const SETTLEMENT_SPACING: f64 = 2600.0;
const MAX_SETTLEMENTS: usize = 48;
/// Longest road between two settlements, as the crow flies.
const MAX_ROAD: f64 = 9000.0;
/// Steepest climb of a road, metres per metre.
const MAX_GRADE: f32 = 0.12;
/// Steeper than this, a road rather goes round or up in turns.
const WINDING_GRADE: f32 = 0.09;
/// What turning costs a road, in metres of road: this times one less the
/// cosine of the angle (a right angle this much, turning back twice).
const TURN_COST: f32 = 60.0;
/// Where a road has to cut or fill to keep its grade, how far it goes from
/// cutting only (0) to filling only (1).
const CUT_SHARE: f32 = 0.35;
/// Steepest step between two cells of the plan a road may take. Steps
/// steeper than about `MAX_GRADE` cost dearly, so a road takes them only
/// over a pass it cannot wind up otherwise.
const MAX_STEP_GRADE: f32 = 0.24;
/// How far a road's surface may lie above or below the ground's own height
/// (embankment or cutting), metres. Where the ground climbs too fast for
/// both this and `MAX_GRADE`, the road climbs faster.
const MAX_EMBANKMENT: f32 = 6.0;
/// Steepest a road climbs where the ground forces it past `MAX_GRADE`
/// (unless it then would be far off the ground).
const MAX_STEEP_GRADE: f32 = 0.25;
/// How much further off the ground than `MAX_EMBANKMENT` a road may be
/// where the ground forces it past `MAX_STEEP_GRADE`.
const RECONCILED_EMBANKMENT: f32 = 3.0;
/// What crossing a river costs, in metres of road.
const RIVER_CROSSING: f32 = 700.0;
/// How far under the ground a river runs in a gorge, metres, and what
/// coming near one costs a road besides, in metres of road.
const GORGE_DEPTH: f32 = 20.0;
const GORGE_CROSSING: f32 = 4000.0;

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
    /// The roads that shape the ground (not those in tunnels).
    pub roads: Vec<RoadSegment>,
    pub settlements: Vec<Settlement>,
    /// Pieces of tunnel tubes (see `tunnel_air`).
    pub tunnels: Vec<RoadSegment>,
}

pub struct Roads {
    settlements: Vec<Settlement>,
    /// Every road's way, as points with the road's height (in tunnels too).
    ways: Vec<Vec<([f64; 2], f32)>>,
    /// Which points of each way lie on a tunnel's line (in its tube or the
    /// cuttings to it).
    in_tunnels: Vec<Vec<bool>>,
    /// Every tunnel's tube: the pieces of road it holds, end to end.
    tunnels: Vec<Vec<RoadSegment>>,
    /// Where the lamps hang in the tunnels (see `LAMP_SPACING`).
    lamps: Vec<[f32; 3]>,
    /// What lies near each chunk square.
    local: HashMap<(i32, i32), Local>,
}

/// Tunnels: how long at most and at least (straight, metres), how steep at
/// most, how much rock lies over the road at least where the plan's coarse
/// ground is looked at (away from the portals), how far from each end that
/// need not hold yet (the cutting into the hillside), what a metre of
/// tunnel costs in metres of road, and how much shorter a way must become
/// for a tunnel to be worth it (counting what the way climbs over its
/// ends, see `CLIMB_COST`).
const TUNNEL_MAX_LENGTH: f64 = 1500.0;
const TUNNEL_MIN_LENGTH: f64 = 250.0;
const TUNNEL_GRADE: f32 = 0.08;
const TUNNEL_PLAN_COVER: f32 = 20.0;
const PORTAL_APPROACH: f64 = 100.0;
const TUNNEL_COST: f64 = 2.0;
/// What a tunnel the way finding takes costs besides its length, in metres
/// of road (see `find_way`'s jumps).
const PORTAL_COST: f32 = 300.0;
const TUNNEL_SAVING: f64 = 300.0;
/// What climbing a metre over a tunnel's ends costs the way round, in
/// metres of road (and coming down again).
const CLIMB_COST: f64 = 10.0;
/// Where at least this much ground lies over the road, it runs in the tube;
/// less, and it runs in a cutting.
pub const TUNNEL_COVER: f32 = 10.0;
/// The tube: half its floor's width, how high its walls stand and the
/// arch over them (a half circle as wide as the floor).
pub const TUNNEL_HALF_WIDTH: f32 = 4.0;
pub const TUNNEL_WALL: f32 = 2.5;
/// Lamps hang this far apart along a tunnel's middle, just under its arch.
const LAMP_SPACING: f64 = 24.0;
const LAMP_UNDER_ARCH: f32 = 0.4;

/// How thick the rock around a tube is that no cave opens into.
const TUNNEL_SHELL: f32 = 6.0;

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
            /// By a river deep under the ground: a road along it would
            /// stand on the gorge's rim.
            Gorge,
        }
        let ground_kind = |i: usize| {
            let (x, z) = (centre(i % n), centre(i / n));
            if ground(i) < SEA_LEVEL + 1.0 || lake(x, z) {
                Ground::Water
            } else if river(x, z, CELL * 0.5) {
                let c = |v: f64| (v / CHUNK_SIZE as f64).floor() as i32;
                let level = rivers
                    .and_then(|r| r.nearest(r.segments_near(c(x), c(z)), x, z))
                    .map_or(f32::NEG_INFINITY, |r| r.level);
                if ground(i) - level > GORGE_DEPTH {
                    Ground::Gorge
                } else {
                    Ground::River
                }
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
        // The evened-out ground along each way, and which of its points are
        // pinned to a settlement's level.
        let mut grounds: Vec<(Vec<f32>, Vec<Option<f32>>)> = Vec::new();
        // Which points of each way lie in a tunnel (left alone when roads
        // that come close are reconciled: one may cross over it).
        let mut in_tunnels: Vec<Vec<bool>> = Vec::new();
        // Tunnels from cell `i` the plan's coarse ground allows, as steps for
        // `find_way`: straight in each of its directions, as `with_tunnels`
        // would take them, at what they cost.
        let tunnel_jumps = |i: usize| -> Vec<(usize, f32)> {
            let mut out = Vec::new();
            // Only at the foot of what a road cannot climb.
            let blocked = neighbours(i).any(|j| (ground(j) - ground(i)).abs() / CELL as f32 > 0.75 * MAX_STEP_GRADE);
            if kind[i] != Ground::Land || !blocked {
                return out;
            }
            let (x, z) = ((i % n) as i64, (i / n) as i64);
            let gi = ground(i);
            for &(dx, dz) in &STEPS {
                let unit = (dx as f64).hypot(dz as f64) * CELL;
                let mut cells = Vec::new();
                for k in 1.. {
                    let (cx, cz) = (x + dx * k, z + dz * k);
                    let length = unit * k as f64;
                    if cx < 0 || cz < 0 || cx >= n as i64 || cz >= n as i64 || length > TUNNEL_MAX_LENGTH {
                        break;
                    }
                    let j = cz as usize * n + cx as usize;
                    // Not under water (a valley it would come out into
                    // is ruled out below: the ground must stay over it).
                    if kind[j] == Ground::Water {
                        break;
                    }
                    cells.push(j);
                    let gj = ground(j);
                    if length < TUNNEL_MIN_LENGTH
                        || kind[j] != Ground::Land
                        || ((gj - gi).abs() as f64) > TUNNEL_GRADE as f64 * length
                    {
                        continue;
                    }
                    let covered = cells[..cells.len() - 1].iter().enumerate().all(|(m, &c)| {
                        let t = (m + 1) as f64 / k as f64;
                        let from_end = (t * length).min((1.0 - t) * length);
                        let needed = if from_end < PORTAL_APPROACH { -1.0 } else { TUNNEL_PLAN_COVER };
                        ground(c) - (gi + (gj - gi) * t as f32) >= needed
                    });
                    // And over the ground between the cells' middles (where
                    // a gully may cross the line), every 16 m.
                    let covered = covered && {
                        let (a, b) = ([centre(i % n), centre(i / n)], [centre(j % n), centre(j / n)]);
                        let samples = (length / 16.0).ceil() as usize;
                        (1..samples).all(|s| {
                            let t = s as f64 / samples as f64;
                            let from_end = (t * length).min((1.0 - t) * length);
                            from_end < PORTAL_APPROACH
                                || natural(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t).0
                                    - (gi + (gj - gi) * t as f32)
                                    >= TUNNEL_COVER
                        })
                    };
                    if covered {
                        out.push((j, (TUNNEL_COST * length) as f32 + PORTAL_COST));
                    }
                }
            }
            out
        };
        // The ground with the rivers' beds and banks carved in: where a
        // road fords a river it goes down the banks to the water.
        let carved = |p: &[f64; 2]| {
            let h = natural(p[0], p[1]).0;
            rivers.map_or(h, |plan| {
                let square = |v: f64| (v / CHUNK_SIZE as f64).floor() as i32;
                let near = plan.segments_near(square(p[0]), square(p[1]));
                Hydrology::carve(h, plan.nearest(near, p[0], p[1]))
            })
        };
        // The plan's ground between its samples (bilinear).
        let ground_at = |p: [f64; 2]| {
            let gx = ((p[0] - origin) / CELL - 0.5).clamp(0.0, (n - 1) as f64);
            let gz = ((p[1] - origin) / CELL - 0.5).clamp(0.0, (n - 1) as f64);
            let (i, j) = ((gx.floor() as usize).min(n - 2), (gz.floor() as usize).min(n - 2));
            let (tx, tz) = ((gx - i as f64) as f32, (gz - j as f64) as f32);
            let at = |i: usize, j: usize| ground(j * n + i);
            let top = at(i, j) + (at(i + 1, j) - at(i, j)) * tx;
            let bottom = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * tx;
            top + (bottom - top) * tz
        };
        // Every link once; then, for settlements left without a road, their
        // links again with tunnels through what the roads could not cross.
        let mut attempts: Vec<(usize, usize, bool)> = links.iter().map(|&(_, a, b)| (a, b, false)).collect();
        let mut failed: Vec<(usize, usize)> = Vec::new();
        let mut connected = vec![false; settlements.len()];
        let mut attempt = 0;
        loop {
            if attempt == links.len() {
                for &(a, b) in &failed {
                    if !connected[a] || !connected[b] {
                        attempts.push((a, b, true));
                    }
                }
            }
            let Some(&(a, b, through)) = attempts.get(attempt) else {
                break;
            };
            attempt += 1;
            if through && connected[a] && connected[b] {
                continue;
            }
            let (start, goal) = (cell_of(settlements[a].centre), cell_of(settlements[b].centre));
            let found = find_way(n, start, goal, |i, j| {
                if kind[j] == Ground::Water && j != goal && j != start {
                    return None;
                }
                // A long step (two cells one way, one the other) passes
                // over the two cells beside its middle.
                let (xi, zi, xj, zj) = ((i % n) as i64, (i / n) as i64, (j % n) as i64, (j / n) as i64);
                let passed = ((xi - xj).abs() + (zi - zj).abs() == 3).then(|| {
                    let (mx, mz) = (xi + xj, zi + zj);
                    [
                        (mz.div_euclid(2) as usize) * n + mx.div_euclid(2) as usize,
                        ((mz + 1).div_euclid(2) as usize) * n + (mx + 1).div_euclid(2) as usize,
                    ]
                });
                if passed.is_some_and(|cells| cells.iter().any(|&c| kind[c] == Ground::Water)) {
                    return None;
                }
                let wet = |k: Ground| k == Ground::River || k == Ground::Gorge;
                let crosses_river = wet(kind[j]) || passed.is_some_and(|cells| cells.iter().any(|&c| wet(kind[c])));
                let gorge =
                    kind[j] == Ground::Gorge || passed.is_some_and(|cells| cells.iter().any(|&c| kind[c] == Ground::Gorge));
                let (di, dj) = ((i % n) as f32 - (j % n) as f32, (i / n) as f32 - (j / n) as f32);
                let length = di.hypot(dj) * CELL as f32;
                let grade = (ground(j) - ground(i)).abs() / length;
                if grade > MAX_STEP_GRADE {
                    return None;
                }
                // Steps a road can climb straight cost little more than flat
                // ones; only beyond that is a way round (or up in turns)
                // better.
                let mut cost = length * (1.0 + 4.0 * grade + 300.0 * (grade - WINDING_GRADE).max(0.0).powi(2));
                if crosses_river {
                    cost += RIVER_CROSSING;
                }
                if gorge {
                    cost += GORGE_CROSSING;
                }
                // Roads merge: on one, cheaper; right beside one, dearer.
                if road[j] {
                    cost *= 0.4;
                } else if beside[j] {
                    cost *= 1.6;
                }
                Some(cost)
            }, |i| if through { tunnel_jumps(i) } else { Vec::new() },
            // A way through needs no search of the whole world.
            if through { 200_000 } else { usize::MAX });
            let Some(cells) = found else {
                if !through {
                    failed.push((a, b));
                }
                continue;
            };
            connected[a] = true;
            connected[b] = true;
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
            // A way that crosses itself goes round a loop for nothing: cut
            // out, before and after the smoothing (which can make some).
            points = without_loops(points);
            // 5. Tunnels where the way winds round or over what a straight
            // one could cross: the way between their ends replaced by the
            // straight line, its height going evenly from one end's ground
            // to the other's.
            let in_settlement = |p: [f64; 2]| {
                settlements
                    .iter()
                    .any(|s| (p[0] - s.centre[0]).hypot(p[1] - s.centre[1]) < s.reach())
            };
            // The tunnels the way finding took (its long steps).
            let jumps: Vec<([f64; 2], [f64; 2])> = cells
                .windows(2)
                .filter(|pair| {
                    let (a, b) = (pair[0], pair[1]);
                    ((a % n) as i64 - (b % n) as i64).abs().max(((a / n) as i64 - (b / n) as i64).abs()) > 2
                })
                .map(|pair| {
                    let at = |c: usize| [centre(c % n), centre(c / n)];
                    (at(pair[0]), at(pair[1]))
                })
                .collect();
            let lines: Vec<([f64; 2], [f64; 2], f32, f32)>;
            (points, lines) = with_tunnels(
                points,
                &jumps,
                |p| ground_at(p),
                |p| kind[cell_of(p)] == Ground::Water || in_settlement(p),
                |p| natural(p[0], p[1]).0,
            );
            for _ in 0..3 {
                points = chaikin(&points);
            }
            points = without_loops(points);
            // Which points lie in a tunnel, and its height there.
            let tunnel_level = |p: [f64; 2]| {
                lines.iter().find_map(|&(a, b, la, lb)| {
                    let line = RoadSegment { a, b, level: [la, lb] };
                    let (d, t) = line.distance(p[0], p[1]);
                    (d < 0.5 && t > 1e-3 && t < 1.0 - 1e-3).then(|| la + (lb - la) * t as f32)
                })
            };
            let in_tunnel: Vec<Option<f32>> = points.iter().map(|&p| tunnel_level(p)).collect();
            // The road's height: the ground's, evened out over about
            // 25 m, and the settlements' level in and near them.
            let raw: Vec<f32> = points.iter().map(carved).collect();
            let mut level = raw.clone();
            for _ in 0..2 {
                level = (0..level.len())
                    .map(|k| {
                        let (lo, hi) = (k.saturating_sub(2), (k + 2).min(level.len() - 1));
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
            // A tunnel's height is its own.
            for (k, l) in in_tunnel.iter().enumerate() {
                if let Some(l) = *l {
                    pinned[k] = Some(l);
                    level[k] = l;
                }
            }
            // The ground's height along the way, with the settlements'
            // level grounds.
            let ground_along: Vec<f32> = points
                .iter()
                .zip(&raw)
                .map(|(p, &h)| Self::level_settlements(&settlements, p[0], p[1], h).0)
                .collect();
            // Evened out over some 30 m, so a ditch or a bump the road
            // crosses does not pull it down or up with it.
            let ground_along: Vec<f32> = (0..ground_along.len())
                .map(|k| {
                    let (lo, hi) = (k.saturating_sub(4), (k + 4).min(ground_along.len() - 1));
                    ground_along[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
                })
                .collect();
            // Then no steeper than a road climbs, the settlements kept, and
            // never far above or below the ground: no long ramps or deep
            // cuttings where the ground climbs faster than the road may.
            for _ in 0..16 {
                limit_grade(&points, &mut level, MAX_GRADE, &pinned);
                for (k, l) in level.iter_mut().enumerate() {
                    let g = ground_along[k];
                    *l = l.clamp(g - MAX_EMBANKMENT, g + MAX_EMBANKMENT);
                    if let Some(p) = pinned[k] {
                        *l = p;
                    }
                }
            }
            // The clamp leaves kinks: evened out over a few metres.
            for _ in 0..2 {
                level = (0..level.len())
                    .map(|k| {
                        if pinned[k].is_some() {
                            return level[k];
                        }
                        let (lo, hi) = (k.saturating_sub(2), (k + 2).min(level.len() - 1));
                        level[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32
                    })
                    .collect();
            }
            let way = points.into_iter().zip(level).collect::<Vec<_>>();
            ways.push(way);
            grounds.push((ground_along, pinned));
            in_tunnels.push(in_tunnel.iter().map(Option::is_some).collect::<Vec<_>>());
        }

        // Where the ground forces both, nearness to it wins over the grade.
        // Then where roads come close to each other (two roads, or two legs
        // of a hairpin) their heights are made to agree, last: there the
        // road may be further off the ground, on a wall as it were.
        for (way, (ground_along, pinned)) in ways.iter_mut().zip(&grounds) {
            let points: Vec<[f64; 2]> = way.iter().map(|p| p.0).collect();
            let mut level: Vec<f32> = way.iter().map(|p| p.1).collect();
            let reach = MAX_EMBANKMENT + RECONCILED_EMBANKMENT;
            for _ in 0..4 {
                limit_grade(&points, &mut level, MAX_STEEP_GRADE, pinned);
                for (k, l) in level.iter_mut().enumerate() {
                    *l = l.clamp(ground_along[k] - reach, ground_along[k] + reach);
                    if let Some(p) = pinned[k] {
                        *l = p;
                    }
                }
            }
            for (p, l) in way.iter_mut().zip(level) {
                p.1 = l;
            }
        }
        for _ in 0..3 {
            reconcile(&mut ways, &in_tunnels);
            // Its moves may leave steps along a way: evened out.
            for (way, (_, pinned)) in ways.iter_mut().zip(&grounds) {
                let points: Vec<[f64; 2]> = way.iter().map(|p| p.0).collect();
                let mut level: Vec<f32> = way.iter().map(|p| p.1).collect();
                limit_grade(&points, &mut level, MAX_STEEP_GRADE, pinned);
                for ((p, l), pin) in way.iter_mut().zip(level).zip(pinned) {
                    p.1 = pin.unwrap_or(l);
                }
            }
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
        // Pieces of road in a tunnel with enough ground over them are in
        // its tube; the others shape the ground.
        let mut tunnels = Vec::new();
        for (way, in_tunnel) in ways.iter().zip(&in_tunnels) {
            let piece = |k: usize| {
                let ((a, la), (b, lb)) = (way[k], way[k + 1]);
                RoadSegment { a, b, level: [la, lb] }
            };
            let covered: Vec<bool> = (0..way.len() - 1)
                .map(|k| {
                    if !(in_tunnel[k] && in_tunnel[k + 1]) {
                        return false;
                    }
                    let s = piece(k);
                    let ends = [s.a, [(s.a[0] + s.b[0]) * 0.5, (s.a[1] + s.b[1]) * 0.5], s.b];
                    let levels = [s.level[0], 0.5 * (s.level[0] + s.level[1]), s.level[1]];
                    ends.iter().zip(levels).all(|(p, l)| carved(p) - l >= TUNNEL_COVER)
                })
                .collect();
            // Where little ground lies over a tunnel between stretches with
            // enough (a gully over it), it stays a tube, open to the gully
            // where the rock is too thin, rather than a cutting down to it.
            let mut covered = covered;
            let mut k = 0;
            while k < covered.len() {
                if covered[k] || k == 0 || !covered[k - 1] {
                    k += 1;
                    continue;
                }
                let start = k;
                // (Only where the ground still bears the road: over a
                // valley it would need a bridge.)
                let borne = |k: usize| {
                    let s = piece(k);
                    carved(&s.a) >= s.level[0] + 1.0 && carved(&s.b) >= s.level[1] + 1.0
                };
                while k < covered.len() && !covered[k] && in_tunnel[k] && in_tunnel[k + 1] && borne(k) {
                    k += 1;
                }
                if k < covered.len() && covered[k] {
                    covered[start..k].iter_mut().for_each(|c| *c = true);
                } else if k == start {
                    // Not on the tunnel's line: the tunnel ended here.
                    k += 1;
                }
            }
            for (k, &covered) in covered.iter().enumerate() {
                let segment = piece(k);
                let (lo, hi) = (
                    [segment.a[0].min(segment.b[0]), segment.a[1].min(segment.b[1])],
                    [segment.a[0].max(segment.b[0]), segment.a[1].max(segment.b[1])],
                );
                if !covered {
                    let reach = (ROAD_HALF_WIDTH + MAX_SHOULDER) as f64 + COLUMN_REACH;
                    for key in squares(lo, hi, reach) {
                        local.entry(key).or_default().roads.push(segment);
                    }
                }
            }
            // Each tube: a run of covered pieces, and one more each side so
            // it opens into the cutting.
            let mut k = 0;
            while k < covered.len() {
                if !covered[k] {
                    k += 1;
                    continue;
                }
                let start = k;
                while k < covered.len() && covered[k] {
                    k += 1;
                }
                let (first, last) = (start.saturating_sub(1), k.min(covered.len() - 1));
                let tube: Vec<RoadSegment> = (first..=last).map(piece).collect();
                // Roads that merged share a tunnel: one tube for it, the
                // first road's.
                let middle = piece((start + k - 1) / 2);
                let mid = [(middle.a[0] + middle.b[0]) * 0.5, (middle.a[1] + middle.b[1]) * 0.5];
                let built = tunnels
                    .iter()
                    .flatten()
                    .any(|s: &RoadSegment| s.distance(mid[0], mid[1]).0 < 2.0);
                if built {
                    continue;
                }
                for segment in &tube {
                    let lo = [segment.a[0].min(segment.b[0]), segment.a[1].min(segment.b[1])];
                    let hi = [segment.a[0].max(segment.b[0]), segment.a[1].max(segment.b[1])];
                    for key in squares(lo, hi, TUNNEL_HALF_WIDTH as f64 + COLUMN_REACH + 2.0) {
                        local.entry(key).or_default().tunnels.push(*segment);
                    }
                }
                tunnels.push(tube);
            }
        }
        let lamps = tunnels.iter().flat_map(|tube| lamps_along(tube)).collect();
        Self {
            settlements,
            ways,
            in_tunnels,
            tunnels,
            lamps,
            local,
        }
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

    /// Whether point `k` of way `way` (see `ways`) lies on a tunnel's
    /// line: in its tube, or in the cutting to it.
    pub fn in_tunnel(&self, way: usize, k: usize) -> bool {
        self.in_tunnels[way][k]
    }

    /// Every tunnel's tube: the pieces of road in it, end to end.
    pub fn tunnels(&self) -> &[Vec<RoadSegment>] {
        &self.tunnels
    }

    /// Where the tunnels' lamps hang, world `[x, y, z]`.
    pub fn tunnel_lamps(&self) -> &[[f32; 3]] {
        &self.lamps
    }

    /// A cave's density `cave` where the nearest tunnel tube's is `tunnel`
    /// (see `tunnel_air`): the rock around a tube is kept whole, so no cave
    /// opens a hole in its floor or wall; `TUNNEL_SHELL` metres away caves
    /// are as they were.
    pub fn cave_beside_tunnel(cave: f32, tunnel: f32) -> f32 {
        cave.min(-TUNNEL_SHELL - tunnel)
    }

    /// How far world `(x, y, z)` lies inside one of the tunnels' tubes
    /// among `pieces` (see `tunnel_air`): positive inside, in metres from
    /// its wall, floor or arch.
    pub fn tunnel_air(pieces: &[RoadSegment], x: f64, y: f32, z: f64) -> f32 {
        let closest = pieces.iter().map(|s| s.distance(x, z).0).fold(f64::INFINITY, f64::min);
        let r = closest as f32;
        if r > TUNNEL_HALF_WIDTH + 2.0 {
            return TUNNEL_HALF_WIDTH - r;
        }
        // The floor's height blended from the pieces around, the nearest
        // most (as the road's outside, see `nearest`), so it has no step
        // where the pieces meet at an angle.
        let (mut sum, mut weights) = (0.0f64, 0.0f64);
        for s in pieces {
            let (d, t) = s.distance(x, z);
            if d > closest + 6.0 {
                continue;
            }
            let w = ((closest + 0.5) / (d + 0.5)).powi(4);
            sum += w * (s.level[0] + (s.level[1] - s.level[0]) * t as f32) as f64;
            weights += w;
        }
        let up = y - (sum / weights) as f32;
        // The tube's distance outward: below the floor, beside the walls,
        // over the arch.
        let side = if up > TUNNEL_WALL {
            r.hypot(up - TUNNEL_WALL) - TUNNEL_HALF_WIDTH
        } else {
            r - TUNNEL_HALF_WIDTH
        };
        -side.max(-up)
    }

    /// What may shape the ground anywhere in chunk square `(cx, cz)` or
    /// within its column's margins.
    pub fn near(&self, cx: i32, cz: i32) -> Option<&Local> {
        self.local.get(&(cx, cz))
    }

    /// The road near world `(x, z)` among `roads`, within reach: the
    /// distance to the nearest piece, and a level blended from every piece
    /// in reach, the nearer the more. Where two pieces at different heights
    /// come close (the legs of a hairpin), the ground between goes from
    /// one's level to the other's instead of jumping where the nearest
    /// changes; on a road its own piece is all but alone.
    pub fn nearest(roads: &[RoadSegment], x: f64, z: f64) -> Option<NearestRoad> {
        let reach = (ROAD_HALF_WIDTH + MAX_SHOULDER) as f64;
        let closest = roads
            .iter()
            .map(|s| s.distance(x, z).0)
            .fold(f64::INFINITY, f64::min);
        if closest > reach {
            return None;
        }
        // By the distance from each piece's edge: on a road its own piece
        // has all but all the weight (its surface stays level across), and
        // between two the ground goes evenly from one to the other. Pieces
        // fade out toward the reach, so none drops out at once.
        let edge = |d: f64| (d - ROAD_HALF_WIDTH as f64).max(0.0) + 0.5;
        let (mut sum, mut weights) = (0.0f64, 0.0f64);
        for s in roads {
            let (d, t) = s.distance(x, z);
            if d > reach {
                continue;
            }
            // Relative to the nearest, so it neither overflows nor vanishes.
            let w = (edge(closest) / edge(d)).powi(2) * (1.0 - smoothstep(reach - 10.0, reach, d));
            let level = s.level[0] + (s.level[1] - s.level[0]) * t as f32;
            sum += w * level as f64;
            weights += w;
        }
        // The nearest piece itself fades at the reach: then its level.
        if weights <= 1e-9 {
            return None;
        }
        Some(NearestRoad {
            distance: closest as f32,
            level: (sum / weights) as f32,
        })
    }

    /// How much the ground at world `(x, z)` is made by a road or a
    /// settlement (1 on a road, its shoulders and a settlement's level
    /// ground, fading to 0 beyond): there no overhang pushes it and no cave
    /// opens in it.
    pub fn worked(settlements: &[Settlement], roads: &[RoadSegment], x: f64, z: f64) -> f32 {
        let road = Self::nearest(roads, x, z).map_or(0.0, |r| {
            let edge = (ROAD_HALF_WIDTH + SHOULDER) as f64;
            1.0 - smoothstep(edge, edge + 2.0 * SHOULDER as f64, r.distance as f64) as f32
        });
        let settlement = settlements
            .iter()
            .map(|s| {
                let d = (x - s.centre[0]).hypot(z - s.centre[1]);
                1.0 - smoothstep(s.radius as f64, s.reach(), d) as f32
            })
            .fold(0.0f32, f32::max);
        road.max(settlement)
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

    /// The ground `h` beside or under a road: level on the road, then held
    /// between the slopes of its cutting and embankment (`CUT_SLOPE`,
    /// `FILL_SLOPE`), with rounded edges where they meet the land.
    pub fn bed(h: f32, road: Option<NearestRoad>) -> f32 {
        let Some(r) = road else {
            return h;
        };
        let x = (r.distance - ROAD_HALF_WIDTH).max(0.0);
        // On a hillside steeper than the slopes they would never meet the
        // land again: there they are a little steeper than the land (its
        // mean slope from the road's middle here), and meet it after
        // about 17 m.
        // (What the road's own embankment or cutting accounts for left out,
        // and never steeper than `MAX_FACE`.)
        let beyond = ((h - r.level).abs() - MAX_EMBANKMENT).max(0.0) * (h - r.level).signum();
        let rise = (STEEPER * beyond / (x + ROAD_HALF_WIDTH)).clamp(-MAX_FACE, MAX_FACE);
        let (cut, fill) = (CUT_SLOPE.max(rise), FILL_SLOPE.max(-rise));
        let (up, down) = (r.level + cut * x, r.level - fill * x);
        // Rounded by up to 1.5 m, less near the road so its edge stays.
        let k = (0.5 * (up - down)).min(1.5);
        let held = smooth_max(smooth_min(h, up, k), down, k);
        // Where the land is steeper than the slopes, back to it at the end.
        let fade = smoothstep((MAX_SHOULDER - SLOPE_FADE) as f64, MAX_SHOULDER as f64, x as f64) as f32;
        held + (h - held) * fade
    }

    /// How much of the road's surface a point `road` away is (0..1): its
    /// colour, with a soft edge.
    pub fn surface(road: Option<NearestRoad>) -> f32 {
        road.map_or(0.0, |r| {
            1.0 - smoothstep((ROAD_HALF_WIDTH - 0.8) as f64, (ROAD_HALF_WIDTH + 0.6) as f64, r.distance as f64) as f32
        })
    }
}

/// The steps from a cell to its neighbours (see `find_way`).
/// How much the way finding's estimate inflates the distance left.
const ESTIMATE_WEIGHT: f32 = 1.3;

const STEPS: [(i64, i64); 16] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
    (-2, -1),
    (-2, 1),
    (2, -1),
    (2, 1),
    (-1, -2),
    (1, -2),
    (-1, 2),
    (1, 2),
];

/// The cheapest way from cell `start` to `goal` over an `n` × `n` grid,
/// where `cost(from, to)` is what stepping to a neighbour costs (`None`:
/// it cannot be entered). Neighbours are the eight around a cell and the
/// eight a knight's move away: more directions, so a way can cross a slope
/// at a shallower angle and wind up it. Turning costs too (see
/// `TURN_COST`), so a way that has to wind does it in long straights and few
/// turns rather than in many small ones. `jumps(cell)` gives longer steps
/// from a cell and their cost (tunnels), after which the way may leave in
/// any direction. A* over cells, a turn costed by the direction a cell was
/// entered in on its best way so far (close to, though not always, the
/// cheapest), with the distance (times `ESTIMATE_WEIGHT`) as the
/// estimate; it gives up after `max_steps` cells.
fn find_way(
    n: usize,
    start: usize,
    goal: usize,
    cost: impl Fn(usize, usize) -> Option<f32>,
    jumps: impl Fn(usize) -> Vec<(usize, f32)>,
    max_steps: usize,
) -> Option<Vec<usize>> {
    // The step each cell was entered by on its best way so far, or
    // `ARRIVED` (the start, or after a jump).
    const ARRIVED: u8 = STEPS.len() as u8;
    let turn = |from: u8, to: usize| {
        if from == ARRIVED {
            return 0.0;
        }
        let (a, b) = (STEPS[from as usize], STEPS[to]);
        let cos = (a.0 * b.0 + a.1 * b.1) as f32 / ((a.0 as f32).hypot(a.1 as f32) * (b.0 as f32).hypot(b.1 as f32));
        TURN_COST * (1.0 - cos)
    };
    // The distance, a little inflated: a way costs much more than its
    // length (climbing, turning), so the plain distance would search far
    // too wide; this finds ways about as cheap, much sooner.
    let estimate = |i: usize| {
        let (dx, dz) = ((i % n) as f32 - (goal % n) as f32, (i / n) as f32 - (goal / n) as f32);
        dx.hypot(dz) * CELL as f32 * ESTIMATE_WEIGHT
    };
    let mut best = vec![f32::INFINITY; n * n];
    let mut from = vec![u32::MAX; n * n];
    let mut entered = vec![ARRIVED; n * n];
    let mut jumped: HashMap<usize, Vec<(usize, f32)>> = HashMap::new();
    let mut open = BinaryHeap::new();
    best[start] = 0.0;
    open.push((Reverse((estimate(start) * 10.0) as i64), start));
    let mut steps = 0;
    while let Some((Reverse(key), i)) = open.pop() {
        steps += 1;
        if steps > max_steps {
            return None;
        }
        if i == goal {
            let mut way = vec![goal];
            while from[*way.last().unwrap()] != u32::MAX {
                way.push(from[*way.last().unwrap()] as usize);
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
        for (d, &(dx, dz)) in STEPS.iter().enumerate() {
            let (nx, nz) = (x + dx, z + dz);
            if nx < 0 || nz < 0 || nx >= n as i64 || nz >= n as i64 {
                continue;
            }
            let j = nz as usize * n + nx as usize;
            let Some(step) = cost(i, j) else {
                continue;
            };
            let total = here + step + turn(entered[i], d);
            if total < best[j] {
                best[j] = total;
                from[j] = i as u32;
                entered[j] = d as u8;
                open.push((Reverse(((total + estimate(j)) * 10.0) as i64), j));
            }
        }
        // Longer steps where there are (tunnels), leaving in any direction;
        // worked out once per cell.
        let cell_jumps = jumped.entry(i).or_insert_with(|| jumps(i));
        for &(j, step) in cell_jumps.iter() {
            let total = here + step;
            if total < best[j] {
                best[j] = total;
                from[j] = i as u32;
                entered[j] = ARRIVED;
                open.push((Reverse(((total + estimate(j)) * 10.0) as i64), j));
            }
        }
    }
    None
}

/// Even out `level` along `points` so the road climbs at most `grade`:
/// between the highest and the lowest such line that stay under and over
/// the levels, nearer the lower (`CUT_SHARE`). `pinned` levels stay as they
/// are, and the road climbs to them.
fn limit_grade(points: &[[f64; 2]], level: &mut [f32], grade: f32, pinned: &[Option<f32>]) {
    let run: Vec<f32> = points
        .windows(2)
        .map(|p| (p[0][0] - p[1][0]).hypot(p[0][1] - p[1][1]) as f32 * grade)
        .collect();
    let envelope = |up: bool| {
        let mut l = level.to_vec();
        let pick = |a: f32, b: f32| if up { a.max(b) } else { a.min(b) };
        let sign = if up { -1.0 } else { 1.0 };
        for k in 1..l.len() {
            l[k] = pinned[k].unwrap_or_else(|| pick(l[k], l[k - 1] + sign * run[k - 1]));
        }
        for k in (0..l.len() - 1).rev() {
            l[k] = pinned[k].unwrap_or_else(|| pick(l[k], l[k + 1] + sign * run[k]));
        }
        l
    };
    let (high, low) = (envelope(true), envelope(false));
    // Nearer the lower line: a road rather runs in a cutting than on an
    // embankment, which stands out of the land.
    for (k, l) in level.iter_mut().enumerate() {
        *l = pinned[k].unwrap_or(low[k] + CUT_SHARE * (high[k] - low[k]));
    }
    // A pinned level holds both lines in place, so beside it they may still
    // be too steep: each free level within the grade of the one before it,
    // forward and back.
    for k in 1..level.len() {
        if pinned[k].is_none() {
            level[k] = level[k].clamp(level[k - 1] - run[k - 1], level[k - 1] + run[k - 1]);
        }
    }
    for k in (0..level.len() - 1).rev() {
        if pinned[k].is_none() {
            level[k] = level[k].clamp(level[k + 1] - run[k], level[k + 1] + run[k]);
        }
    }
}

/// How far apart two roads may be at most for their heights to be
/// reconciled (see `reconcile`), metres between their middles.
const RECONCILE_REACH: f64 = 24.0;
/// Pieces of one way this far apart along it are its neighbours, not
/// another leg of it.
const SAME_LEG: f64 = 60.0;
/// Over how many points either way a reconciling move spreads.
const SPREAD: isize = 4;

/// Make the heights of roads that come close agree: two roads, or two legs
/// of one (a hairpin), at most as far apart in height as the slope of a
/// cutting can bridge between their edges. Where they overlap (a junction,
/// or roads that merged) they lie at the same height. Each point too far
/// from the other's height moves halfway toward it, the other piece's ends
/// the other half, over a few rounds.
fn reconcile(ways: &mut [Vec<([f64; 2], f32)>], fixed: &[Vec<bool>]) {
    // Every point by a coarse grid, and how far along its way it lies.
    let grid = RECONCILE_REACH;
    let key = |p: [f64; 2]| ((p[0] / grid).floor() as i64, (p[1] / grid).floor() as i64);
    let mut along: Vec<Vec<f64>> = Vec::with_capacity(ways.len());
    let mut cells: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (w, way) in ways.iter().enumerate() {
        let mut run = vec![0.0];
        for pair in way.windows(2) {
            let (a, b) = (pair[0].0, pair[1].0);
            run.push(run.last().unwrap() + (a[0] - b[0]).hypot(a[1] - b[1]));
        }
        along.push(run);
        for (k, &(p, _)) in way.iter().enumerate() {
            cells.entry(key(p)).or_default().push((w, k));
        }
    }
    // Which point is near which piece (the ways do not move, only their
    // heights): for every point the nearest piece of every other leg
    // around, as (point's way, point, piece's way, piece, distance, how far
    // along the piece).
    let mut pairs: Vec<(usize, usize, usize, usize, f64, f64)> = Vec::new();
    for (w, way) in ways.iter().enumerate() {
        for (k, &(p, _)) in way.iter().enumerate() {
            if fixed[w][k] {
                continue;
            }
            let (cx, cz) = key(p);
            let mut nearest: Vec<((usize, i64), (f64, usize, f64))> = Vec::new();
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let Some(list) = cells.get(&(cx + dx, cz + dz)) else {
                        continue;
                    };
                    for &(v, l) in list {
                        if l + 1 >= ways[v].len() || fixed[v][l] || fixed[v][l + 1] {
                            continue;
                        }
                        if v == w && (along[w][k] - along[v][l]).abs() < SAME_LEG {
                            continue;
                        }
                        let (a, b) = (ways[v][l].0, ways[v][l + 1].0);
                        let (d, t) = RoadSegment { a, b, level: [0.0; 2] }.distance(p[0], p[1]);
                        if d > RECONCILE_REACH {
                            continue;
                        }
                        // One per leg: legs of a way told apart by how far
                        // along it they lie.
                        let leg = (v, (along[v][l] / SAME_LEG).floor() as i64);
                        match nearest.iter_mut().find(|(k, _)| *k == leg) {
                            Some((_, n)) if d < n.0 => *n = (d, l, t),
                            Some(_) => {}
                            None => nearest.push((leg, (d, l, t))),
                        }
                    }
                }
            }
            pairs.extend(nearest.into_iter().map(|((v, _), (d, l, t))| (w, k, v, l, d, t)));
        }
    }
    let width = 2.0 * ROAD_HALF_WIDTH as f64;
    for _ in 0..12 {
        // (way, point, how far, weight): a point asked to move by several
        // others moves by their weighted mean.
        let mut moves: Vec<(usize, usize, f32, f32)> = Vec::new();
        for &(w, k, v, l, d, t) in &pairs {
            let level = ways[w][k].1;
            let other = ways[v][l].1 + (ways[v][l + 1].1 - ways[v][l].1) * t as f32;
            let allowed = (CUT_SLOPE.min(1.0) as f64 * (d - width).max(0.0)) as f32;
            let excess = (level - other).abs() - allowed;
            // (What is left is not worth another round.)
            if excess <= 0.05 {
                continue;
            }
            // Each side moves half: this point, and the piece's ends by how
            // near they are.
            let step = 0.5 * excess * (other - level).signum();
            moves.push((w, k, step, 1.0));
            moves.push((v, l, -step, 1.0 - t as f32));
            moves.push((v, l + 1, -step, t as f32));
        }
        if moves.is_empty() {
            break;
        }
        // A move spreads along the way to the points around, less the
        // further, so the road bends to it instead of stepping.
        let mut sums: HashMap<(usize, usize), (f32, f32)> = HashMap::new();
        for (w, k, step, weight) in moves {
            for j in -SPREAD..=SPREAD {
                let Some(at) = k.checked_add_signed(j) else {
                    continue;
                };
                if at >= ways[w].len() {
                    continue;
                }
                let falloff = 1.0 - j.unsigned_abs() as f32 / (SPREAD + 1) as f32;
                let sum = sums.entry((w, at)).or_default();
                sum.0 += step * weight * falloff;
                sum.1 += weight * falloff;
            }
        }
        for ((w, k), (sum, weight)) in sums {
            if weight > 1e-6 && !fixed[w][k] {
                ways[w][k].1 += sum / weight.max(1.0);
            }
        }
    }
}

/// The lamps of a tunnel's `tube`: every `LAMP_SPACING` metres along its
/// middle, under the top of its arch, half a spacing in from each end.
fn lamps_along(tube: &[RoadSegment]) -> Vec<[f32; 3]> {
    let mut lamps = Vec::new();
    let mut next = LAMP_SPACING * 0.5;
    let mut run = 0.0;
    for s in tube {
        let length = (s.a[0] - s.b[0]).hypot(s.a[1] - s.b[1]);
        while next <= run + length {
            let t = (next - run) / length.max(1e-9);
            let floor = s.level[0] + (s.level[1] - s.level[0]) * t as f32;
            lamps.push([
                (s.a[0] + (s.b[0] - s.a[0]) * t) as f32,
                floor + TUNNEL_WALL + TUNNEL_HALF_WIDTH - LAMP_UNDER_ARCH,
                (s.a[1] + (s.b[1] - s.a[1]) * t) as f32,
            ]);
            next += LAMP_SPACING;
        }
        run += length;
    }
    lamps
}

/// `points` with tunnels where they shorten the way much: two points of it
/// at most `TUNNEL_MAX_LENGTH` apart, between which the way runs at least
/// `TUNNEL_COST` times as far plus `TUNNEL_SAVING`, joined by a straight
/// line no steeper than `TUNNEL_GRADE` from one's ground to the other's,
/// with `TUNNEL_PLAN_COVER` of ground over it except near its ends (and
/// `TUNNEL_COVER` of the ground as it lies, `exact`), and
/// never under the ground there (no valley in between) or under water
/// (`excluded`, which also keeps the ends out of settlements). The best
/// first, then the next that does not overlap it. `ground` is the coarse
/// ground the plan looks at, `exact` the ground at a point (for the
/// tunnels' heights). `forced`: tunnels the way finding took already, from
/// one point to the next, kept whatever they save. Returns the new points and each tunnel's line with
/// its height at either end.
#[allow(clippy::type_complexity)]
fn with_tunnels(
    points: Vec<[f64; 2]>,
    forced: &[([f64; 2], [f64; 2])],
    ground: impl Fn([f64; 2]) -> f32,
    excluded: impl Fn([f64; 2]) -> bool,
    exact: impl Fn([f64; 2]) -> f32,
) -> (Vec<[f64; 2]>, Vec<([f64; 2], [f64; 2], f32, f32)>) {
    let n = points.len();
    let mut along = vec![0.0f64; n];
    for k in 1..n {
        along[k] = along[k - 1] + (points[k][0] - points[k - 1][0]).hypot(points[k][1] - points[k - 1][1]);
    }
    let works = |i: usize, j: usize| -> Option<f64> {
        let (a, b) = (points[i], points[j]);
        let length = (a[0] - b[0]).hypot(a[1] - b[1]);
        if !(TUNNEL_MIN_LENGTH..=TUNNEL_MAX_LENGTH).contains(&length) {
            return None;
        }
        let (ga, gb) = (ground(a), ground(b));
        let climb = (i..=j).map(|k| ground(points[k])).fold(f32::MIN, f32::max) - ga.max(gb);
        let saving = along[j] - along[i] + CLIMB_COST * climb.max(0.0) as f64 - TUNNEL_COST * length;
        if saving < TUNNEL_SAVING || excluded(a) || excluded(b) {
            return None;
        }
        if ((gb - ga).abs() as f64) > TUNNEL_GRADE as f64 * length || ga.min(gb) < SEA_LEVEL + 3.0 {
            return None;
        }
        let samples = (length / 16.0).ceil() as usize;
        for s in 1..samples {
            let t = s as f64 / samples as f64;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let level = ga + (gb - ga) * t as f32;
            let cover = ground(p) - level;
            let from_end = (t * length).min((1.0 - t) * length);
            let needed = if from_end < PORTAL_APPROACH { -1.0 } else { TUNNEL_PLAN_COVER };
            if cover < needed || excluded(p) {
                return None;
            }
        }
        // And over the ground as it lies (the coarse ground misses gullies
        // between its points, which the tunnel would open into): looked at
        // only for tunnels that pass so far, as it costs more.
        for s in 1..samples {
            let t = s as f64 / samples as f64;
            let from_end = (t * length).min((1.0 - t) * length);
            if from_end < PORTAL_APPROACH {
                continue;
            }
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if exact(p) - (ga + (gb - ga) * t as f32) < TUNNEL_COVER {
                return None;
            }
        }
        Some(saving)
    };
    // The best tunnel from each point, then the best of all that do not
    // overlap one taken.
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for i in 0..n {
        let mut best: Option<(f64, usize)> = None;
        for j in i + 2..n {
            if along[j] - along[i] > TUNNEL_MAX_LENGTH * 6.0 {
                break;
            }
            if let Some(saving) = works(i, j) {
                if best.is_none_or(|b| saving > b.0) {
                    best = Some((saving, j));
                }
            }
        }
        if let Some((saving, j)) = best {
            candidates.push((saving, i, j));
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    // The tunnels the way finding took (`forced`) first of all.
    let mut taken: Vec<(usize, usize)> = (1..n)
        .filter(|&k| forced.contains(&(points[k - 1], points[k])))
        .map(|k| (k - 1, k))
        .collect();
    for (_, i, j) in candidates {
        if taken.iter().all(|&(a, b)| j <= a || i >= b) {
            taken.push((i, j));
        }
    }
    taken.sort();
    let mut out = Vec::with_capacity(n);
    let mut lines = Vec::new();
    let mut k = 0;
    for (i, j) in taken {
        out.extend_from_slice(&points[k..=i]);
        let (a, b) = (points[i], points[j]);
        lines.push((a, b, exact(a), exact(b)));
        // Points along it as close as the rest of the way's, so it is
        // smoothed, levelled and cut into pieces like the rest.
        let length = (a[0] - b[0]).hypot(a[1] - b[1]);
        let steps = (length / CELL).ceil() as usize;
        for step in 1..steps {
            let t = step as f64 / steps as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
        k = j;
    }
    out.extend_from_slice(&points[k..]);
    (out, lines)
}

/// `points` with every loop cut out: where the way crosses itself, it goes
/// on from the crossing along its later part.
fn without_loops(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    'search: loop {
        for i in 0..points.len().saturating_sub(1) {
            // The last crossing with this piece cuts out the most.
            for j in (i + 2..points.len() - 1).rev() {
                if let Some(at) = crossing(points[i], points[i + 1], points[j], points[j + 1]) {
                    let mut kept = points[..=i].to_vec();
                    kept.push(at);
                    kept.extend_from_slice(&points[j + 1..]);
                    points = kept;
                    continue 'search;
                }
            }
        }
        return points;
    }
}

/// Where the pieces `a`–`b` and `c`–`d` cross, if they do (not where they
/// only touch).
fn crossing(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let denominator = r[0] * s[1] - r[1] * s[0];
    if denominator.abs() < 1e-12 {
        return None;
    }
    let (qp0, qp1) = (c[0] - a[0], c[1] - a[1]);
    let t = (qp0 * s[1] - qp1 * s[0]) / denominator;
    let u = (qp0 * r[1] - qp1 * r[0]) / denominator;
    (t > 0.0 && t < 1.0 && u > 0.0 && u < 1.0).then(|| [a[0] + r[0] * t, a[1] + r[1] * t])
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

/// The smaller of `a` and `b`, rounded where they are within `k` of each
/// other (polynomial smooth minimum).
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn smooth_max(a: f32, b: f32, k: f32) -> f32 {
    -smooth_min(-a, -b, k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loops_are_cut_out_at_the_crossing() {
        // East, then a loop up and back across the way, then on east.
        let points = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [5.0, 5.0], [5.0, -5.0], [20.0, -5.0]];
        let kept = without_loops(points);
        assert_eq!(kept, vec![[0.0, 0.0], [5.0, 0.0], [5.0, -5.0], [20.0, -5.0]]);
        // A way that does not cross itself stays as it is.
        let bend = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert_eq!(without_loops(bend.clone()), bend);
    }
}
