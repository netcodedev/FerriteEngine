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
    bridges::{Bridge, DECK_HALF_WIDTH},
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
    /// The bridges (see `Bridge::solid`).
    pub bridges: Vec<Arc<Bridge>>,
}

pub struct Roads {
    settlements: Vec<Settlement>,
    /// Every road's way, as points with the road's height (in tunnels too).
    ways: Vec<Vec<([f64; 2], f32)>>,
    /// What each point of each way lies on: a tunnel's line (in its tube or
    /// the cuttings to it), a bridge's shortcut, or neither.
    in_tunnels: Vec<Vec<Line>>,
    /// Every tunnel's tube: the pieces of road it holds, end to end.
    tunnels: Vec<Vec<RoadSegment>>,
    /// Every bridge, and which points of each way lie on one.
    bridges: Vec<Arc<Bridge>>,
    on_bridges: Vec<Vec<bool>>,
    /// Where the lamps hang in the tunnels (see `LAMP_SPACING`).
    lamps: Vec<[f32; 3]>,
    /// What lies near each chunk square.
    local: HashMap<(i32, i32), Local>,
}

/// Tunnels: how long at most (metres), how steep at most, how far from
/// each end at most the road may run in a cutting into the hillside
/// before the tube (a quarter of a short tunnel), and what a metre of
/// tunnel (or bridge) costs in metres of road.
const TUNNEL_MAX_LENGTH: f64 = 1500.0;
const TUNNEL_GRADE: f32 = 0.08;
const PORTAL_APPROACH: f64 = 100.0;
const TUNNEL_COST: f64 = 2.0;
/// The shortest tube a tunnel has (between the cuttings at its ends).
const TUNNEL_MIN_LENGTH: f64 = 40.0;
/// The tunnels the way finding takes, through what roads cannot cross:
/// how long at least, how much rock lies over them (on the plan's coarse
/// ground, away from the portals), and what one costs besides its length,
/// in metres of road (see `find_way`'s jumps).
const JUMP_MIN_LENGTH: f64 = 250.0;
const JUMP_PLAN_COVER: f32 = 20.0;
const PORTAL_COST: f32 = 300.0;
/// Shortcuts (tunnels and bridges where the way winds round a spur or a
/// bay, along a hillside or over what it climbs or dips into): how much
/// cheaper the way must become for one to be worth it, in metres of road;
/// how short one is at least (straight, end to end); how far along the way
/// its ends may lie apart at most.
const SHORTCUT_SAVING: f64 = 100.0;
/// And besides, for each metre it is long.
const SHORTCUT_SAVING_PER_METRE: f64 = 0.2;
const SHORTCUT_MIN_LENGTH: f64 = 60.0;
const SHORTCUT_SEARCH: f64 = 6.0 * TUNNEL_MAX_LENGTH;
/// What climbing a metre over a shortcut's ends (or dipping a metre under
/// them) costs the way round, in metres of road (and coming back again).
const CLIMB_COST: f64 = 10.0;
/// What a metre of road along a hillside costs besides, in metres of road:
/// none where the ground across it is gentler than `HILLSIDE_FROM` (rise
/// per metre, over `HILLSIDE_REACH` either side), all from `HILLSIDE_TO`.
const HILLSIDE_COST: f64 = 1.5;
const HILLSIDE_FROM: f64 = 0.35;
const HILLSIDE_TO: f64 = 1.0;
const HILLSIDE_REACH: f64 = 12.0;
/// How a shortcut may bend: the share of its length its ends run on in the
/// way's direction (0: straight, then with at most `MAX_KINK` radians of a
/// kink where it leaves the way), and how tight it may turn at most.
const SHORTCUT_BENDS: [f64; 3] = [0.0, 0.2, 0.35];
const MAX_KINK: f64 = 0.26;
const MIN_RADIUS: f64 = 90.0;
/// Metres between a shortcut's points.
const SHORTCUT_STEP: f64 = 8.0;
/// Bridges: how high over the ground a bridge shortcut runs at least in its
/// middle and at most anywhere, and how long one is at most; how high over
/// the ground the road must run elsewhere for a bridge instead of an
/// embankment, at which height a bridge lands on the ground, and how long
/// one over dry ground is at least.
const BRIDGE_MIN_HEIGHT: f32 = 6.0;
const BRIDGE_MAX_HEIGHT: f32 = 50.0;
const BRIDGE_MAX_LENGTH: f64 = 500.0;
const BRIDGE_HEIGHT: f32 = 6.0;
const BRIDGE_LANDS: f32 = 2.0;
const BRIDGE_MIN_LENGTH: f64 = 12.0;
/// How far over the ground the road may be where a bridge ends.
const BRIDGE_ENDS: f32 = 0.5;
/// Steepest a bridge's deck climbs, metres per metre.
const BRIDGE_MAX_GRADE: f32 = 0.07;
/// How far before and after a point the ground is looked at for how deep
/// a dip the road crosses there (see `BRIDGE_HEIGHT`).
const DIP_REACH: f64 = 80.0;
/// Near a river the road keeps this high over its water (to cross on a
/// bridge rather than down the banks), within this of the water's edge.
const BRIDGE_OVER_WATER: f32 = 4.5;
const RIVER_APPROACH: f32 = 25.0;
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
        let mut grounds: Vec<(Vec<f32>, Vec<Option<f32>>, Vec<f32>)> = Vec::new();
        // Which points of each way lie in a tunnel (left alone when roads
        // that come close are reconciled: one may cross over it).
        let mut in_tunnels: Vec<Vec<Line>> = Vec::new();
        // The shortcuts taken so far, for the roads after to share.
        let mut known_shortcuts: Vec<Shortcut> = Vec::new();
        // The ground under each point of each way (without the road), and
        // whether a river's water is there.
        let mut unders: Vec<Vec<(f32, bool)>> = Vec::new();
        // Tunnels from cell `i` the plan's coarse ground allows, as steps for
        // `find_way`: straight in each of its directions, as `with_tunnels`
        // would take them, at what they cost.
        // The ground as it lies, by the nearest point of an 8 m grid, each
        // looked at once: the way through looks at the same ground from
        // many cells.
        let near_ground: Mutex<HashMap<(i64, i64), f32>> = Mutex::default();
        let ground_near = |x: f64, z: f64| {
            let key = ((x / 8.0).round() as i64, (z / 8.0).round() as i64);
            if let Some(&h) = near_ground.lock().unwrap().get(&key) {
                return h;
            }
            let h = natural(key.0 as f64 * 8.0, key.1 as f64 * 8.0).0;
            near_ground.lock().unwrap().insert(key, h);
            h
        };
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
                    if length < JUMP_MIN_LENGTH
                        || kind[j] != Ground::Land
                        || ((gj - gi).abs() as f64) > TUNNEL_GRADE as f64 * length
                    {
                        continue;
                    }
                    let covered = cells[..cells.len() - 1].iter().enumerate().all(|(m, &c)| {
                        let t = (m + 1) as f64 / k as f64;
                        let from_end = (t * length).min((1.0 - t) * length);
                        let needed = if from_end < PORTAL_APPROACH { -1.0 } else { JUMP_PLAN_COVER };
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
                                || ground_near(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
                                    - (gi + (gj - gi) * t as f32)
                                    >= TUNNEL_COVER
                        })
                    };
                    // The shortest each way only: from where it comes out
                    // the way may tunnel on (and looking at every length
                    // over the ground as it lies costs much).
                    if covered {
                        out.push((j, (TUNNEL_COST * length) as f32 + PORTAL_COST));
                        break;
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
        let square = |v: f64| (v / CHUNK_SIZE as f64).floor() as i32;
        let river_at = |p: [f64; 2]| rivers.and_then(|plan| plan.nearest(plan.segments_near(square(p[0]), square(p[1])), p[0], p[1]));
        // The level of a river's water at a point, where it is.
        let water_at = |p: [f64; 2]| river_at(p).filter(|r| r.distance < r.width * 0.5 + 0.5).map(|r| r.level);
        // The ground a road keeps to: near a river, over its water, to
        // cross it on a bridge rather than down its banks (over a gorge at
        // its rim, or near it).
        // (Only where the road crosses the river, not along it: `direction`
        // is the road's.)
        let over_rivers = |p: &[f64; 2], direction: [f64; 2]| {
            let h = carved(p);
            let Some(r) = river_at(*p) else {
                return h;
            };
            let along = (direction[0] * r.direction[0] as f64 + direction[1] * r.direction[1] as f64).abs();
            let edge = (r.distance - r.width * 0.5) as f64;
            let near = (1.0 - smoothstep(10.0, RIVER_APPROACH as f64, edge) as f32)
                * (1.0 - smoothstep(0.6, 0.85, along) as f32);
            if near <= 0.0 {
                return h;
            }
            let natural = natural(p[0], p[1]).0;
            let depth = natural - r.level;
            let gorge = smoothstep(GORGE_DEPTH as f64 * 0.5, GORGE_DEPTH as f64, depth as f64) as f32;
            let keep = natural - (depth - BRIDGE_OVER_WATER).max(0.0) * (1.0 - gorge);
            h + (keep - h).max(0.0) * near
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
            // 5. Tunnels and bridges where the way winds round or over what
            // a shorter one could cross: the way between their ends
            // replaced by a straight or gently curved line, its height
            // going evenly from one end's ground to the other's.
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
            (points, lines) = with_jumps(points, &jumps, |p| natural(p[0], p[1]).0);
            for _ in 0..3 {
                points = chaikin(&points);
            }
            points = without_loops(points);
            // Which points lie in a tunnel the way finding took, and its
            // height there.
            let tunnel_level = |p: [f64; 2]| {
                lines.iter().find_map(|&(a, b, la, lb)| {
                    let line = RoadSegment { a, b, level: [la, lb] };
                    let (d, t) = line.distance(p[0], p[1]);
                    (d < 0.5 && t > 1e-3 && t < 1.0 - 1e-3).then(|| la + (lb - la) * t as f32)
                })
            };
            let jumped: Vec<Option<(f32, Line)>> =
                points.iter().map(|&p| tunnel_level(p).map(|l| (l, Line::Tunnel))).collect();
            // Then the shortcuts, between the smoothed way's points (no
            // smoothing after: they leave the way in its own direction).
            let on_line: Vec<Option<(f32, Line)>>;
            (points, on_line) = shortcuts(
                points,
                jumped,
                &mut known_shortcuts,
                |p| kind[cell_of(p)] == Ground::Water || in_settlement(p),
                |p| carved(&p),
            );
            let in_tunnel: Vec<Option<f32>> = on_line.iter().map(|l| l.map(|(level, _)| level)).collect();
            // The road's height: the ground's (near a river, over its
            // water), evened out over about 25 m, and the settlements'
            // level in and near them.
            // Over a river's water at least `BRIDGE_OVER_WATER` over it.
            let lowest: Vec<f32> = points
                .iter()
                .map(|&p| {
                    river_at(p)
                        .filter(|r| r.distance < r.width * 0.5 + 4.0)
                        .map_or(f32::MIN, |r| r.level + BRIDGE_OVER_WATER)
                })
                .collect();
            let raw: Vec<f32> = (0..points.len())
                .map(|k| {
                    let (a, b) = (points[k.saturating_sub(1)], points[(k + 1).min(points.len() - 1)]);
                    let length = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-9);
                    over_rivers(&points[k], [(b[0] - a[0]) / length, (b[1] - a[1]) / length])
                })
                .collect();
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
                    *l = l.clamp(g - MAX_EMBANKMENT, g + MAX_EMBANKMENT).max(lowest[k]);
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
            let way = points.iter().copied().zip(level).collect::<Vec<_>>();
            ways.push(way);
            grounds.push((ground_along, pinned, lowest));
            in_tunnels.push(on_line.iter().map(|l| l.map_or(Line::Road, |(_, line)| line)).collect::<Vec<_>>());
            unders.push(
                points
                    .iter()
                    .map(|p| (Self::level_settlements(&settlements, p[0], p[1], carved(p)).0, water_at(*p).is_some()))
                    .collect::<Vec<_>>(),
            );
        }

        // Where the ground forces both, nearness to it wins over the grade.
        // Then where roads come close to each other (two roads, or two legs
        // of a hairpin) their heights are made to agree, last: there the
        // road may be further off the ground, on a wall as it were.
        for (way, (ground_along, pinned, lowest)) in ways.iter_mut().zip(&grounds) {
            let points: Vec<[f64; 2]> = way.iter().map(|p| p.0).collect();
            let mut level: Vec<f32> = way.iter().map(|p| p.1).collect();
            let reach = MAX_EMBANKMENT + RECONCILED_EMBANKMENT;
            for _ in 0..4 {
                for (k, l) in level.iter_mut().enumerate() {
                    *l = l.clamp(ground_along[k] - reach, ground_along[k] + reach).max(lowest[k]);
                    if let Some(p) = pinned[k] {
                        *l = p;
                    }
                }
                limit_grade(&points, &mut level, MAX_STEEP_GRADE, pinned);
            }
            for (p, l) in way.iter_mut().zip(level) {
                p.1 = l;
            }
        }
        let fixed: Vec<Vec<bool>> = in_tunnels.iter().map(|l| l.iter().map(|&l| l != Line::Road).collect()).collect();
        for _ in 0..3 {
            reconcile(&mut ways, &fixed);
            // Its moves may leave steps along a way: evened out.
            for (way, (_, pinned, lowest)) in ways.iter_mut().zip(&grounds) {
                let points: Vec<[f64; 2]> = way.iter().map(|p| p.0).collect();
                // (Over a river's water as high as it should be, unless that
                // is steeper than the grade.)
                let mut level: Vec<f32> = way.iter().zip(lowest).map(|(p, &low)| p.1.max(low)).collect();
                limit_grade(&points, &mut level, MAX_STEEP_GRADE, pinned);
                for ((p, l), pin) in way.iter_mut().zip(level).zip(pinned) {
                    p.1 = pin.unwrap_or(l);
                }
            }
        }

        // Which pieces of a way run in a tunnel's tube (`covered`) and which
        // on a bridge, and the runs of bridge, from its heights, what its
        // points lie on and the ground under them.
        let classify = |way: &[([f64; 2], f32)], lines: &[Line], under: &[(f32, bool)]| {
            let in_tunnel: Vec<bool> = lines.iter().map(|&l| l == Line::Tunnel).collect();
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
            // Pieces high over the ground (or over a river's water, or on a
            // bridge's shortcut) are on a bridge, as far as it is off the
            // ground.
            let lifted = |k: usize| (way[k].1 - under[k].0).min(way[k + 1].1 - under[k + 1].0);
            let wet = |k: usize| under[k].1 || under[k + 1].1;
            let shortcut = |k: usize| lines[k] == Line::Bridge || lines[k + 1] == Line::Bridge;
            // How deep the ground under each point lies under the ground
            // the road passes before and after it (a gully or a valley it
            // crosses, not a hillside it climbs).
            let mut run = vec![0.0f64; way.len()];
            for k in 1..way.len() {
                run[k] = run[k - 1] + (way[k].0[0] - way[k - 1].0[0]).hypot(way[k].0[1] - way[k - 1].0[1]);
            }
            let dip = |k: usize| {
                let highest = |range: &mut dyn Iterator<Item = usize>| {
                    range
                        .take_while(|&j| (run[j] - run[k]).abs() <= DIP_REACH)
                        .map(|j| under[j].0)
                        .fold(f32::MIN, f32::max)
                };
                highest(&mut (0..k).rev()).min(highest(&mut (k + 1..way.len()))) - under[k].0
            };
            let mut bridged: Vec<bool> = (0..covered.len())
                .map(|k| {
                    !covered[k]
                        && ((lifted(k) >= BRIDGE_HEIGHT && dip(k).max(dip(k + 1)) >= BRIDGE_HEIGHT)
                            || (wet(k) && lifted(k) > 0.0)
                            || (shortcut(k) && lifted(k) >= BRIDGE_LANDS))
                })
                .collect();
            // Short gaps between two bridges bridged too, and each out to
            // where it lands.
            let mut k = 0;
            while k < bridged.len() {
                if bridged[k] {
                    k += 1;
                    continue;
                }
                let start = k;
                while k < bridged.len() && !bridged[k] {
                    k += 1;
                }
                if start > 0 && k < bridged.len() && k - start <= 2 && !covered[start..k].iter().any(|&c| c) {
                    bridged[start..k].iter_mut().for_each(|b| *b = true);
                }
            }
            // On to where the road runs on the ground (not ending in the
            // air: by a river its banks are not filled up to the road).
            let over = |k: usize| way[k].1 - under[k].0;
            for _ in 0..12 {
                let grown: Vec<bool> = (0..bridged.len())
                    .map(|k| {
                        bridged[k]
                            || (!covered[k]
                                && ((k > 0 && bridged[k - 1] && over(k) >= BRIDGE_ENDS)
                                    || (k + 1 < bridged.len() && bridged[k + 1] && over(k + 1) >= BRIDGE_ENDS)))
                    })
                    .collect();
                if grown == bridged {
                    break;
                }
                bridged = grown;
            }
            // Runs of bridge: too short ones over dry ground are left an
            // embankment.
            let mut runs: Vec<(usize, usize)> = Vec::new();
            let mut k = 0;
            while k < bridged.len() {
                if !bridged[k] {
                    k += 1;
                    continue;
                }
                let (mut start, end) = (k, {
                    while k < bridged.len() && bridged[k] {
                        k += 1;
                    }
                    k
                });
                // Its ends where the road runs on the ground (or in a
                // cutting) left to the ground.
                let mut end = end;
                while start < end && !wet(start) && over(start) < BRIDGE_ENDS && over(start + 1) < BRIDGE_ENDS {
                    bridged[start] = false;
                    start += 1;
                }
                while end > start && !wet(end - 1) && over(end) < BRIDGE_ENDS && over(end - 1) < BRIDGE_ENDS {
                    bridged[end - 1] = false;
                    end -= 1;
                }
                if start == end {
                    continue;
                }
                let k = end;
                let length: f64 = (start..k).map(|k| {
                    let s = piece(k);
                    (s.a[0] - s.b[0]).hypot(s.a[1] - s.b[1])
                }).sum();
                if length >= BRIDGE_MIN_LENGTH || (start..k).any(wet) {
                    runs.push((start, k));
                } else {
                    bridged[start..k].iter_mut().for_each(|b| *b = false);
                }
            }
            (covered, bridged, runs)
        };
        // Bridges have straight decks, no steeper than `BRIDGE_MAX_GRADE`:
        // the road on either side climbs to meet them. (Over a few rounds:
        // moved, the road may run on a bridge a little further.)
        for (w, (lines, under)) in (0..3).flat_map(|_| in_tunnels.iter().zip(&unders).enumerate()) {
            let (_, _, runs) = classify(&ways[w], lines, under);
            if runs.is_empty() {
                continue;
            }
            let points: Vec<[f64; 2]> = ways[w].iter().map(|p| p.0).collect();
            let mut level: Vec<f32> = ways[w].iter().map(|p| p.1).collect();
            let mut pinned = grounds[w].1.clone();
            let mut run = vec![0.0f64; points.len()];
            for k in 1..points.len() {
                run[k] = run[k - 1] + (points[k][0] - points[k - 1][0]).hypot(points[k][1] - points[k - 1][1]);
            }
            for &(start, end) in &runs {
                let length = (run[end] - run[start]).max(1e-6);
                let (from, to) = (level[start], level[end]);
                let most = BRIDGE_MAX_GRADE * length as f32;
                // An end held already (a tunnel's portal, a settlement)
                // stays; the other moves.
                let (from, to) = match (pinned[start].is_some(), pinned[end].is_some()) {
                    (true, true) if (to - from).abs() > most => continue,
                    (true, true) => (from, to),
                    (true, false) => (from, from + (to - from).clamp(-most, most)),
                    (false, true) => (to - (to - from).clamp(-most, most), to),
                    (false, false) => {
                        let (middle, rise) = (0.5 * (from + to), (to - from).clamp(-most, most));
                        (middle - 0.5 * rise, middle + 0.5 * rise)
                    }
                };
                for k in start..=end {
                    let t = ((run[k] - run[start]) / length) as f32;
                    level[k] = from + (to - from) * t;
                    pinned[k] = Some(level[k]);
                }
            }
            limit_grade(&points, &mut level, MAX_STEEP_GRADE, &pinned);
            for (p, l) in ways[w].iter_mut().zip(level) {
                p.1 = l;
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
        let mut bridges: Vec<Arc<Bridge>> = Vec::new();
        let mut on_bridges = Vec::new();
        for ((way, lines), under) in ways.iter().zip(&in_tunnels).zip(&unders) {
            let piece = |k: usize| {
                let ((a, la), (b, lb)) = (way[k], way[k + 1]);
                RoadSegment { a, b, level: [la, lb] }
            };
            let (covered, bridged, runs) = classify(way, lines, under);
            for &(start, end) in &runs {
                let carried: Vec<RoadSegment> = (start..end).map(piece).collect();
                // Roads that merged share a bridge: the first road's.
                // (Only where it carries this road too: about where it runs
                // and at its height.)
                let middle = carried[carried.len() / 2];
                let built = bridges.iter().any(|b| {
                    b.pieces.iter().any(|s| {
                        let (d, t) = s.distance(middle.a[0], middle.a[1]);
                        let level = s.level[0] + (s.level[1] - s.level[0]) * t as f32;
                        d < 2.5 && (level - middle.level[0]).abs() < 0.6
                    })
                });
                if built {
                    continue;
                }
                let bridge = Arc::new(Bridge::new(
                    carried,
                    |p| Self::level_settlements(&settlements, p[0], p[1], carved(&p)).0,
                    water_at,
                ));
                let reach = DECK_HALF_WIDTH as f64 + COLUMN_REACH + 2.0;
                for key in squares(bridge.lo(), bridge.hi(), reach) {
                    local.entry(key).or_default().bridges.push(bridge.clone());
                }
                bridges.push(bridge);
            }
            on_bridges.push(
                (0..way.len())
                    .map(|k| (k > 0 && bridged[k - 1]) || (k < bridged.len() && bridged[k]))
                    .collect::<Vec<_>>(),
            );
            // The first and last piece of a bridge shape the ground under
            // them too: its abutments, the ground there at the road's
            // height.
            let abutment = |k: usize| runs.iter().any(|&(start, end)| k == start || k + 1 == end);
            for (k, &covered) in covered.iter().enumerate() {
                let covered = covered || (bridged[k] && !abutment(k));
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
                    .any(|s: &RoadSegment| s.distance(mid[0], mid[1]).0 < 6.0);
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
            bridges,
            on_bridges,
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
        self.in_tunnels[way][k] == Line::Tunnel
    }

    /// Whether point `k` of way `way` (see `ways`) lies on a bridge.
    pub fn on_bridge(&self, way: usize, k: usize) -> bool {
        self.on_bridges[way][k]
    }

    pub fn bridges(&self) -> &[Arc<Bridge>] {
        &self.bridges
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

/// `points` with the tunnels the way finding took (`forced`: from one
/// point to the next, straight through what the roads could not cross):
/// points along each as close as the rest of the way's, so it is smoothed
/// and cut into pieces like the rest. `exact` is the ground at a point (the
/// tunnels' heights at their ends). Returns the new points and each
/// tunnel's line with its height at either end.
#[allow(clippy::type_complexity)]
fn with_jumps(
    points: Vec<[f64; 2]>,
    forced: &[([f64; 2], [f64; 2])],
    exact: impl Fn([f64; 2]) -> f32,
) -> (Vec<[f64; 2]>, Vec<([f64; 2], [f64; 2], f32, f32)>) {
    let mut out = Vec::with_capacity(points.len());
    let mut lines = Vec::new();
    for (k, &p) in points.iter().enumerate() {
        if k > 0 && forced.contains(&(points[k - 1], p)) {
            let a = points[k - 1];
            lines.push((a, p, exact(a), exact(p)));
            let length = (a[0] - p[0]).hypot(a[1] - p[1]);
            let steps = (length / CELL).ceil() as usize;
            for step in 1..steps {
                let t = step as f64 / steps as f64;
                out.push([a[0] + (p[0] - a[0]) * t, a[1] + (p[1] - a[1]) * t]);
            }
        }
        out.push(p);
    }
    (out, lines)
}

/// A shortcut taken (see `shortcuts`): its line, both ends included, the
/// road's height along it, and what it is.
#[derive(Clone, Debug)]
struct Shortcut {
    points: Vec<[f64; 2]>,
    levels: Vec<f32>,
    line: Line,
}

/// How near a road's way must come to both ends of an earlier road's
/// shortcut to take it too, metres.
const REUSE_REACH: f64 = 8.0;

/// What a point of a way lies on, besides the ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Road,
    Tunnel,
    Bridge,
}

/// `points` (`lines`: what each lies on already) with shortcuts where they
/// make the way much cheaper: two points of it, between which the way runs
/// much further (see `SHORTCUT_SAVING`), climbs over or dips under its ends
/// (`CLIMB_COST`) or runs along a steep hillside (`HILLSIDE_COST`), joined
/// by a line straight or bent a little (see `SHORTCUT_BENDS`), no steeper
/// than `TUNNEL_GRADE` from one end's ground to the other's: a tunnel where
/// it runs under the ground with enough rock over it (but for the cuttings
/// toward its ends), a bridge where it runs high over the ground, neither
/// over `excluded` ground. `ground` is the ground at a point. The best
/// first, then the next that does not overlap one taken. Returns the new
/// points and what each lies on, with the road's height there.
#[allow(clippy::type_complexity)]
fn shortcuts(
    points: Vec<[f64; 2]>,
    mut lines: Vec<Option<(f32, Line)>>,
    known: &mut Vec<Shortcut>,
    excluded: impl Fn([f64; 2]) -> bool,
    ground: impl Fn([f64; 2]) -> f32,
) -> (Vec<[f64; 2]>, Vec<Option<(f32, Line)>>) {
    let n = points.len();
    if n < 6 {
        return (points, lines);
    }
    // Shortcuts of the roads before where this one runs the same way
    // round (the roads merged): the same, so they share one tunnel or
    // bridge.
    let mut taken: Vec<(usize, usize, Shortcut)> = Vec::new();
    for shortcut in known.iter() {
        let near = |p: [f64; 2]| {
            (0..n)
                .filter(|&k| lines[k].is_none() && (points[k][0] - p[0]).hypot(points[k][1] - p[1]) < REUSE_REACH)
                .min_by(|&a, &b| {
                    let d = |k: usize| (points[k][0] - p[0]).hypot(points[k][1] - p[1]);
                    d(a).total_cmp(&d(b))
                })
        };
        let (Some(i), Some(j)) = (near(shortcut.points[0]), near(*shortcut.points.last().unwrap())) else {
            continue;
        };
        let mut reused = shortcut.clone();
        let (i, j) = if i < j {
            (i, j)
        } else {
            reused.points.reverse();
            reused.levels.reverse();
            (j, i)
        };
        if j > i + 1 && taken.iter().all(|t| j <= t.0 || i >= t.1) {
            for line in &mut lines[i + 1..j] {
                *line = Some((0.0, reused.line));
            }
            taken.push((i, j, reused));
        }
    }
    let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let mut along = vec![0.0f64; n];
    for k in 1..n {
        along[k] = along[k - 1] + distance(points[k - 1], points[k]);
    }
    let heights: Vec<f32> = points.iter().map(|&p| ground(p)).collect();
    // (The ones reused above are left alone.)
    let reused = |k: usize| taken.iter().any(|t| k >= t.0 && k <= t.1);
    let tangent = |k: usize| {
        let (a, b) = (points[k.saturating_sub(1)], points[(k + 1).min(n - 1)]);
        let length = distance(a, b).max(1e-9);
        [(b[0] - a[0]) / length, (b[1] - a[1]) / length]
    };
    // What the way costs along hillsides, summed up to each point.
    let mut hillside = vec![0.0f64; n];
    for k in 1..n {
        let ([tx, tz], p) = (tangent(k), points[k]);
        let (sx, sz) = (-tz * HILLSIDE_REACH, tx * HILLSIDE_REACH);
        let across = (ground([p[0] + sx, p[1] + sz]) - ground([p[0] - sx, p[1] - sz])).abs() as f64 / (2.0 * HILLSIDE_REACH);
        hillside[k] = hillside[k - 1]
            + HILLSIDE_COST * smoothstep(HILLSIDE_FROM, HILLSIDE_TO, across) * (along[k] - along[k - 1]);
    }
    // The best shortcut from every other point.
    let mut candidates: Vec<(f64, usize, usize, Vec<[f64; 2]>, Line)> = Vec::new();
    for i in (1..n - 1).step_by(2) {
        if lines[i].is_some() || reused(i) {
            continue;
        }
        let (mut high, mut low) = (heights[i], heights[i]);
        let mut best: Option<(f64, usize, Vec<[f64; 2]>, Line)> = None;
        for j in i + 1..n - 1 {
            high = high.max(heights[j]);
            low = low.min(heights[j]);
            if lines[j].is_some() || reused(j) || along[j] - along[i] > SHORTCUT_SEARCH {
                break;
            }
            // Every other point, further away fewer.
            let stride = if along[j] - along[i] < 400.0 { 2 } else { 6 };
            if (j - i) % stride != 0 {
                continue;
            }
            let (a, b) = (points[i], points[j]);
            let chord = distance(a, b);
            if !(SHORTCUT_MIN_LENGTH..=TUNNEL_MAX_LENGTH).contains(&chord) {
                continue;
            }
            let (gi, gj) = (heights[i], heights[j]);
            if ((gj - gi).abs() as f64) > TUNNEL_GRADE as f64 * chord {
                continue;
            }
            let over = (high - gi.max(gj)).max(0.0) + (gi.min(gj) - low).max(0.0);
            let gain = along[j] - along[i] + CLIMB_COST * over as f64 + hillside[j] - hillside[i];
            // (A bent line is longer than the straight one.)
            let needed = |length: f64| SHORTCUT_SAVING + SHORTCUT_SAVING_PER_METRE * length;
            if gain - TUNNEL_COST * chord < needed(chord) || excluded(a) || excluded(b) {
                continue;
            }
            for bend in SHORTCUT_BENDS {
                let Some(curve) = shortcut_curve(a, tangent(i), b, tangent(j), bend) else {
                    continue;
                };
                let length: f64 = curve.windows(2).map(|p| distance(p[0], p[1])).sum();
                let saving = gain - TUNNEL_COST * length;
                if saving < needed(length) || best.as_ref().is_some_and(|b| b.0 >= saving) {
                    continue;
                }
                if let Some(line) = shortcut_kind(&curve, length, gi, gj, &excluded, &ground) {
                    best = Some((saving, j, curve, line));
                }
            }
        }
        if let Some((saving, j, curve, line)) = best {
            candidates.push((saving, i, j, curve, line));
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, i, j, curve, line) in candidates {
        if taken.iter().all(|t| j <= t.0 || i >= t.1) {
            // Its road going evenly from one end's ground to the other's.
            let length: f64 = curve.windows(2).map(|p| distance(p[0], p[1])).sum();
            let (gi, gj) = (heights[i], heights[j]);
            let mut run = 0.0;
            let levels = (0..curve.len())
                .map(|m| {
                    if m > 0 {
                        run += distance(curve[m - 1], curve[m]);
                    }
                    gi + (gj - gi) * (run / length) as f32
                })
                .collect();
            let shortcut = Shortcut { points: curve, levels, line };
            known.push(shortcut.clone());
            taken.push((i, j, shortcut));
        }
    }
    taken.sort_by_key(|t| t.0);
    let mut out = Vec::with_capacity(n);
    let mut on = Vec::with_capacity(n);
    let mut k = 0;
    for (i, j, shortcut) in taken {
        out.extend_from_slice(&points[k..=i]);
        on.extend_from_slice(&lines[k..=i]);
        for m in 1..shortcut.points.len() - 1 {
            out.push(shortcut.points[m]);
            on.push(Some((shortcut.levels[m], shortcut.line)));
        }
        k = j;
    }
    out.extend_from_slice(&points[k..]);
    on.extend_from_slice(&lines[k..]);
    (out, on)
}

/// A shortcut's line from `a` to `b`, leaving `a` in direction `ta` and
/// coming into `b` in direction `tb` for `bend` of its length (a cubic
/// Bézier curve; straight for 0, then with a kink of at most `MAX_KINK` at
/// either end), as points about `SHORTCUT_STEP` apart, both ends included.
/// `None` where it would turn tighter than `MIN_RADIUS`.
fn shortcut_curve(a: [f64; 2], ta: [f64; 2], b: [f64; 2], tb: [f64; 2], bend: f64) -> Option<Vec<[f64; 2]>> {
    let chord = (b[0] - a[0]).hypot(b[1] - a[1]);
    let direction = [(b[0] - a[0]) / chord, (b[1] - a[1]) / chord];
    if bend == 0.0 {
        let kink = |t: [f64; 2]| (t[0] * direction[0] + t[1] * direction[1]).clamp(-1.0, 1.0).acos();
        if kink(ta) > MAX_KINK || kink(tb) > MAX_KINK {
            return None;
        }
    }
    let (c1, c2) = (
        [a[0] + ta[0] * chord * bend, a[1] + ta[1] * chord * bend],
        [b[0] - tb[0] * chord * bend, b[1] - tb[1] * chord * bend],
    );
    let at = |t: f64| {
        let u = 1.0 - t;
        let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        [
            w0 * a[0] + w1 * c1[0] + w2 * c2[0] + w3 * b[0],
            w0 * a[1] + w1 * c1[1] + w2 * c2[1] + w3 * b[1],
        ]
    };
    // Finely, then evenly spaced by the distance along it.
    const FINE: usize = 96;
    let fine: Vec<[f64; 2]> = (0..=FINE).map(|k| at(k as f64 / FINE as f64)).collect();
    let mut run = vec![0.0];
    for p in fine.windows(2) {
        run.push(run.last().unwrap() + (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]));
    }
    let length = *run.last().unwrap();
    let count = (length / SHORTCUT_STEP).ceil().max(2.0) as usize;
    let step = length / count as f64;
    let mut out = Vec::with_capacity(count + 1);
    let mut f = 0;
    for m in 0..=count {
        let s = m as f64 * step;
        while f + 1 < FINE && run[f + 1] < s {
            f += 1;
        }
        let t = ((s - run[f]) / (run[f + 1] - run[f]).max(1e-9)).clamp(0.0, 1.0);
        out.push([fine[f][0] + (fine[f + 1][0] - fine[f][0]) * t, fine[f][1] + (fine[f + 1][1] - fine[f][1]) * t]);
    }
    // No tighter than `MIN_RADIUS`: the turn between neighbouring steps.
    for p in out.windows(3) {
        let (u, v) = ([p[1][0] - p[0][0], p[1][1] - p[0][1]], [p[2][0] - p[1][0], p[2][1] - p[1][1]]);
        let turn = (u[0] * v[1] - u[1] * v[0]).atan2(u[0] * v[0] + u[1] * v[1]).abs();
        if turn * MIN_RADIUS > step {
            return None;
        }
    }
    Some(out)
}

/// What a shortcut along `curve` (`length` metres, its road going evenly
/// from `from` to `to`) would be: a tunnel where the ground lies over it
/// with `TUNNEL_COVER` but toward its ends (a quarter of it, at most
/// `PORTAL_APPROACH`, never over the ground), with a tube at least
/// `TUNNEL_MIN_LENGTH` long; a bridge where it runs over the ground, in its
/// middle at least `BRIDGE_MIN_HEIGHT` over it and nowhere higher than
/// `BRIDGE_MAX_HEIGHT`. Neither over `excluded` ground.
fn shortcut_kind(
    curve: &[[f64; 2]],
    length: f64,
    from: f32,
    to: f32,
    excluded: &impl Fn([f64; 2]) -> bool,
    ground: &impl Fn([f64; 2]) -> f32,
) -> Option<Line> {
    let steps = curve.len() - 1;
    let level = |m: usize| from + (to - from) * (m as f32 / steps as f32);
    let over = |m: usize| ground(curve[m]) - level(m);
    let middle = over(steps / 2);
    let approach = (0.25 * length).clamp(20.0, PORTAL_APPROACH);
    if middle >= TUNNEL_COVER {
        if length - 2.0 * approach < TUNNEL_MIN_LENGTH {
            return None;
        }
        let covered = (1..steps).all(|m| {
            let s = length * m as f64 / steps as f64;
            let needed = if s.min(length - s) < approach { -1.0 } else { TUNNEL_COVER };
            over(m) >= needed && !excluded(curve[m])
        });
        return covered.then_some(Line::Tunnel);
    }
    if middle <= -BRIDGE_MIN_HEIGHT && length <= BRIDGE_MAX_LENGTH {
        let borne = (1..steps).all(|m| {
            let o = over(m);
            o <= BRIDGE_LANDS && o >= -BRIDGE_MAX_HEIGHT && !excluded(curve[m])
        });
        return borne.then_some(Line::Bridge);
    }
    None
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
