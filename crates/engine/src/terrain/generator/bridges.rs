//! Bridges: where a road crosses a river or a gorge, or a dip in the
//! ground too deep for an embankment, it runs on a stone bridge. Each is a
//! mesh of its own (see `Bridge::mesh`), drawn and collided with apart from
//! the ground, so its edges stay crisp and its road and stone keep their
//! own textures.
//!
//! A bridge is a deck with a parapet each side and a cornice under them,
//! over arches that spring from piers. The piers stand as far apart as the
//! bridge is high (a low bridge has short spans and flat arches, a high
//! viaduct tall piers and round arches), and out of the water where a span
//! can reach over it: only a river wider than the longest span gets a pier
//! in it.

use std::f32::consts::PI;

use cgmath::{InnerSpace, Vector3};

use super::roads::{RoadSegment, ROAD_HALF_WIDTH};
use crate::core::model::generated::MeshPart;

/// The parapets: how thick and how high over the road.
pub const PARAPET_THICKNESS: f32 = 0.5;
pub const PARAPET_HEIGHT: f32 = 1.0;
/// Half the deck's width: the road and a parapet each side.
pub const DECK_HALF_WIDTH: f32 = ROAD_HALF_WIDTH + PARAPET_THICKNESS;
/// The cornice under the parapets: how far it stands out of them, and
/// from how far under the road to how far.
const CORNICE_OUT: f32 = 0.15;
const CORNICE_TOP: f32 = 0.2;
const CORNICE_BOTTOM: f32 = 0.55;
/// How far the walls under the cornice (spandrels, piers) stand in from
/// the parapets.
const WALL_IN: f32 = 0.1;
/// From the road down to the crown of an arch.
const DECK_THICKNESS: f32 = 1.4;
/// How far the deck reaches past each end of the bridge (none: beyond, the
/// road goes on at its own grade), and how far over the road it lies (so
/// the two do not flicker where the ground under its ends is the road's).
const END_REACH: f64 = 0.0;
const DECK_RAISE: f32 = 0.03;
/// Toward each end the road dips this far under the road beyond, over
/// this many metres: its edge lies in the ground, not on it.
const END_DIVE: f32 = 0.15;
const END_RAMP: f64 = 1.5;
/// The kerb between the road and each parapet: how wide and how high.
const KERB_WIDTH: f32 = 0.3;
const KERB_HEIGHT: f32 = 0.15;
/// The coping on the parapets: how far it stands out each side, how thick.
const COPING_OUT: f32 = 0.06;
const COPING_HEIGHT: f32 = 0.15;
/// The posts at the bridge's four corners: how long along it, how far they
/// stand out of the parapet each way, how much higher, how far under the
/// cornice they reach.
const POST_LENGTH: f32 = 1.1;
const POST_OUT: f32 = 0.2;
const POST_HIGHER: f32 = 0.4;
const POST_UNDER: f32 = 0.35;
/// The ring of stones round each arch: how far it stands out of the wall,
/// and how wide it is at most.
const RING_OUT: f32 = 0.12;
const RING_WIDTH: f32 = 0.75;
/// A pier's thickness along the bridge.
const PIER_THICKNESS: f64 = 2.6;
/// How far piers and abutments reach into the ground under them (its
/// lowest across their width, before a road's cutting beside them may
/// lower it): they go on under it, so wherever the ground lies, they stand
/// in it.
const FOOTING: f32 = 8.0;
/// Most metres between two of the mesh's sections along the bridge.
const SECTION: f64 = 4.0;
/// An arch rises at most this share of half its span (a little under a
/// half circle), and at least this many metres (or the deck is thinner).
const ARCH_RISE: f32 = 0.92;
const MIN_RISE: f32 = 0.8;
/// Over too low an opening for an arch: how thick the deck is, and how
/// high the opening under it at least (into the water, if need be).
const THIN_DECK: f32 = 0.9;
const LOWEST_OPENING: f32 = 0.4;
/// Arches spring at least this far over the water.
const WATER_CLEARANCE: f32 = 0.6;
/// Spans: the shortest, the longest (over water too wide for it a pier
/// stands in the water), and the one the piers are set apart by over
/// ground `h` metres under the road (`SPAN_BASE + SPAN_PER_HEIGHT * h`).
const MIN_SPAN: f64 = 9.0;
pub const MAX_SPAN: f64 = 48.0;
const SPAN_BASE: f64 = 14.0;
const SPAN_PER_HEIGHT: f64 = 0.8;
const MAX_TARGET_SPAN: f64 = 36.0;
/// How far from the water a pier's face keeps.
pub const PIER_FROM_WATER: f64 = 1.5;
/// Metres one repeat of the stone's texture covers (the bridge texture's
/// own size: its bricks as large as real ones).
const STONE_TILE: f32 = 2.0;
/// (The road's texture is laid out without repeats over this many metres;
/// see `bridge_view`.)
pub const ROAD_TILE: f32 = 8.0;

/// The opening under one arch, between the faces of its piers (or the
/// bridge's ends), metres along the bridge.
#[derive(Clone, Copy, Debug)]
struct Span {
    from: f64,
    to: f64,
    /// Height where the arch springs from the piers, in its middle: on a
    /// bridge that climbs, the arch climbs with it (`slope`, metres per
    /// metre along it).
    spring: f32,
    rise: f32,
    slope: f32,
}

impl Span {
    fn middle(&self) -> f64 {
        0.5 * (self.from + self.to)
    }

    fn half(&self) -> f32 {
        (0.5 * (self.to - self.from)) as f32
    }

    /// The springing's height `s` metres along the bridge.
    fn spring_at(&self, s: f64) -> f32 {
        self.spring + self.slope * (s - self.middle()) as f32
    }
}

/// One bridge: the pieces of road it carries, end to end, and its arches.
#[derive(Debug)]
pub struct Bridge {
    pub pieces: Vec<RoadSegment>,
    /// Metres along the bridge where each piece starts.
    starts: Vec<f64>,
    length: f64,
    spans: Vec<Span>,
    /// Where the piers stand, metres along the bridge (their middles).
    piers: Vec<f64>,
    /// The ground under the middle every metre along it.
    ground: Vec<f32>,
    /// The box around it, `[x, z]`, with the deck's width.
    lo: [f64; 2],
    hi: [f64; 2],
    /// The highest point of its parapets.
    pub top: f32,
}

/// Where a point lies by a bridge: how far along it and how far beside its
/// middle.
#[derive(Clone, Copy, Debug)]
struct Along {
    s: f64,
    across: f32,
}

impl Bridge {
    /// The bridge carrying `pieces`, over ground `ground` (at a point,
    /// without the bridge), where `water` is the level of the water a pier
    /// should keep out of (`None` where there is none).
    pub fn new(pieces: Vec<RoadSegment>, ground: impl Fn([f64; 2]) -> f32, water: impl Fn([f64; 2]) -> Option<f32>) -> Self {
        let mut starts = Vec::with_capacity(pieces.len());
        let mut length = 0.0;
        for p in &pieces {
            starts.push(length);
            length += (p.b[0] - p.a[0]).hypot(p.b[1] - p.a[1]);
        }
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        let mut top = f32::MIN;
        for p in &pieces {
            for q in [p.a, p.b] {
                for i in 0..2 {
                    lo[i] = lo[i].min(q[i]);
                    hi[i] = hi[i].max(q[i]);
                }
            }
            top = top.max(p.level[0].max(p.level[1]) + PARAPET_HEIGHT);
        }
        let reach = DECK_HALF_WIDTH as f64 + END_REACH;
        let mut bridge = Self {
            pieces,
            starts,
            length,
            spans: Vec::new(),
            piers: Vec::new(),
            ground: Vec::new(),
            lo: lo.map(|v| v - reach),
            hi: hi.map(|v| v + reach),
            top,
        };
        bridge.place_piers(&ground, &water);
        bridge
    }

    /// The point `s` metres along the middle, and the road's height there.
    fn point(&self, s: f64) -> ([f64; 2], f32) {
        let k = self.starts.partition_point(|&start| start <= s).saturating_sub(1);
        let p = &self.pieces[k];
        let run = (p.b[0] - p.a[0]).hypot(p.b[1] - p.a[1]).max(1e-9);
        let t = ((s - self.starts[k]) / run).clamp(0.0, 1.0);
        (
            [p.a[0] + (p.b[0] - p.a[0]) * t, p.a[1] + (p.b[1] - p.a[1]) * t],
            p.level[0] + (p.level[1] - p.level[0]) * t as f32,
        )
    }

    /// Where the piers stand and the arches between them.
    fn place_piers(&mut self, ground: &impl Fn([f64; 2]) -> f32, water: &impl Fn([f64; 2]) -> Option<f32>) {
        let length = self.length;
        // Every metre: the ground, the road over it, and the water.
        let metres = length.ceil() as usize;
        let samples: Vec<(f32, f32, Option<f32>)> = (0..=metres)
            .map(|m| {
                let (p, deck) = self.point(m as f64);
                (ground(p), deck, water(p))
            })
            .collect();
        let at = |s: f64| samples[(s.round().max(0.0) as usize).min(metres)];
        let wet = |s: f64| {
            let half = PIER_THICKNESS * 0.5 + PIER_FROM_WATER;
            let (from, to) = ((s - half).floor().max(0.0) as usize, ((s + half).ceil() as usize).min(metres));
            samples[from..=to].iter().any(|v| v.2.is_some())
        };
        let height: f64 = samples.iter().map(|v| (v.1 - v.0).max(0.0) as f64).sum::<f64>() / samples.len() as f64;
        let target = (SPAN_BASE + SPAN_PER_HEIGHT * height).min(MAX_TARGET_SPAN);
        let mut piers = Vec::new();
        let mut previous = 0.0;
        loop {
            let rest = length - previous;
            if rest <= target * 1.35 {
                break;
            }
            let count = (rest / target).round().max(2.0);
            let ideal = previous + rest / count;
            let (low, high) = (previous + MIN_SPAN, (previous + MAX_SPAN).min(length - MIN_SPAN));
            // The dry place nearest the ideal one, within reach of the
            // last support; in the water only where none is.
            let mut best: Option<f64> = None;
            let mut s = low;
            while s <= high {
                if !wet(s) && best.is_none_or(|b| (s - ideal).abs() < (b - ideal).abs()) {
                    best = Some(s);
                }
                s += 1.0;
            }
            // (None at all where one span reaches over the rest.)
            if best.is_none() && rest <= MAX_SPAN * 1.2 {
                break;
            }
            let pier = best.unwrap_or(ideal.clamp(low, high.max(low)));
            piers.push(pier);
            previous = pier;
        }
        // The arches between the supports.
        let mut supports = vec![(0.0, 0.0)];
        supports.extend(piers.iter().map(|&p| (p, PIER_THICKNESS * 0.5)));
        supports.push((length, 0.0));
        self.spans = supports
            .windows(2)
            .filter_map(|pair| {
                let (from, to) = (pair[0].0 + pair[0].1, pair[1].0 - pair[1].1);
                if to - from < 2.0 {
                    return None;
                }
                let half = (0.5 * (to - from)) as f32;
                // The road over it, as a straight line from end to end
                // (lowered to where it dips in between).
                let (start, end) = (at(from).1, at(to).1);
                let slope = (end - start) / (to - from) as f32;
                let middle = at(0.5 * (from + to)).1.min(0.5 * (start + end));
                // (Its lowest end, under which the arch must fit.)
                let road = middle - slope.abs() * half;
                // At least on the lower support's ground, and over the
                // water under the span.
                let mut floor = at(pair[0].0).0.min(at(pair[1].0).0);
                let mut s = from;
                while s <= to {
                    if let Some(level) = at(s).2 {
                        floor = floor.max(level + WATER_CLEARANCE);
                    }
                    s += 1.0;
                }
                // Where no arch fits under the deck, a thinner slab over a
                // low opening (the water still runs under it).
                let crown = if road - DECK_THICKNESS - floor >= MIN_RISE {
                    road - DECK_THICKNESS
                } else {
                    road - THIN_DECK
                };
                let rise = (half * ARCH_RISE).min(crown - floor).max(LOWEST_OPENING);
                Some(Span {
                    from,
                    to,
                    spring: crown - rise + slope.abs() * half,
                    rise,
                    slope,
                })
            })
            .collect();
        self.piers = piers;
        // The lowest ground across the bridge's width every metre: on a
        // hillside the walls' feet stand lower on its downhill side.
        let reach = (DECK_HALF_WIDTH + 1.0) as f64;
        self.ground = (0..=metres)
            .map(|m| {
                let frame = self.frame(m as f64);
                let across = frame.across.cast::<f64>().unwrap();
                let (x, z) = (frame.middle.x as f64, frame.middle.z as f64);
                [-reach, -0.5 * reach, 0.5 * reach, reach]
                    .iter()
                    .map(|&u| ground([x + across.x * u, z + across.z * u]))
                    .fold(samples[m].0, f32::min)
            })
            .collect();
    }

    /// Where the piers stand, as world `[x, z]`.
    pub fn pier_points(&self) -> Vec<[f64; 2]> {
        self.piers.iter().map(|&s| self.point(s).0).collect()
    }

    /// The corners of the box around it (with the deck's width).
    pub fn lo(&self) -> [f64; 2] {
        self.lo
    }

    pub fn hi(&self) -> [f64; 2] {
        self.hi
    }

    pub fn length(&self) -> f64 {
        self.length
    }

    /// Whether world `(x, z)` lies within `margin` metres of the deck's box.
    pub fn in_box(&self, x: f64, z: f64, margin: f64) -> bool {
        x >= self.lo[0] - margin && x <= self.hi[0] + margin && z >= self.lo[1] - margin && z <= self.hi[1] + margin
    }

    /// Where world `(x, z)` lies by the bridge. Beyond its ends the line
    /// goes on straight (so the deck ends square).
    fn along(&self, x: f64, z: f64) -> Along {
        let last = self.pieces.len() - 1;
        let mut best = (f64::INFINITY, 0.0);
        for (k, p) in self.pieces.iter().enumerate() {
            let (dx, dz) = (p.b[0] - p.a[0], p.b[1] - p.a[1]);
            let run2 = (dx * dx + dz * dz).max(1e-12);
            let mut t = ((x - p.a[0]) * dx + (z - p.a[1]) * dz) / run2;
            let (low, high) = (if k == 0 { f64::MIN } else { 0.0 }, if k == last { f64::MAX } else { 1.0 });
            t = t.clamp(low, high);
            let d = (x - p.a[0] - dx * t).hypot(z - p.a[1] - dz * t);
            if d < best.0 {
                best = (d, self.starts[k] + t * run2.sqrt());
            }
        }
        Along {
            s: best.1,
            across: best.0 as f32,
        }
    }

    /// Whether world `(x, z)` lies under the bridge or within `margin` of
    /// its sides.
    pub fn covers(&self, x: f64, z: f64, margin: f32) -> bool {
        if !self.in_box(x, z, margin as f64) {
            return false;
        }
        let a = self.along(x, z);
        a.across < DECK_HALF_WIDTH + margin && a.s > -END_REACH && a.s < self.length + END_REACH
    }
}

/// A bridge as meshes: its stone, and the road over it.
#[derive(Default, Debug)]
pub struct BridgeMesh {
    pub stone: MeshPart,
    pub road: MeshPart,
}

/// A section across a bridge: how far along it, how low its stone reaches
/// there (the arch's underside, or the footing under a pier), and, under an
/// arch, which way that underside faces, `[along, up]`.
#[derive(Clone, Copy, Debug)]
struct Section {
    s: f64,
    bottom: f32,
    underside: Option<[f32; 2]>,
    /// Under a pier or an abutment (down to its footing), not an arch.
    on_ground: bool,
}

/// The middle of a section, the way along and across the bridge there,
/// and the road's height.
struct Frame {
    middle: Vector3<f32>,
    along: Vector3<f32>,
    across: Vector3<f32>,
    deck: f32,
}

/// `corners` as a quad facing `normal`, wound to suit it.
fn face(part: &mut MeshPart, corners: [Vector3<f32>; 4], normal: Vector3<f32>, uvs: [[f32; 2]; 4]) {
    let geometric = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
    let order: [usize; 4] = if geometric.dot(normal) >= 0.0 { [0, 1, 2, 3] } else { [0, 3, 2, 1] };
    let indices = order.map(|i| part.vertex(corners[i], normal, uvs[i]));
    part.quad(indices);
}

impl Bridge {
    /// The frame `s` metres along the bridge (beyond its ends, on in the
    /// direction of its first or last piece).
    fn frame(&self, s: f64) -> Frame {
        let last = self.pieces.len() - 1;
        let k = self.starts.partition_point(|&start| start <= s).saturating_sub(1).min(last);
        let p = &self.pieces[k];
        let run = (p.b[0] - p.a[0]).hypot(p.b[1] - p.a[1]).max(1e-9);
        let t = (s - self.starts[k]) / run;
        let t = if k == 0 { t.min(1.0) } else { t.max(0.0) };
        let t = if k == last { t } else { t.min(1.0) };
        let deck = p.level[0] + (p.level[1] - p.level[0]) * t.clamp(0.0, 1.0) as f32;
        let along = Vector3::new((p.b[0] - p.a[0]) / run, 0.0, (p.b[1] - p.a[1]) / run).cast::<f32>().unwrap();
        Frame {
            middle: Vector3::new(
                (p.a[0] + (p.b[0] - p.a[0]) * t) as f32,
                deck,
                (p.a[1] + (p.b[1] - p.a[1]) * t) as f32,
            ),
            along,
            across: Vector3::new(along.z, 0.0, -along.x),
            deck,
        }
    }

    /// The lowest ground under the bridge from `from` to `to` metres along
    /// it.
    fn lowest_ground(&self, from: f64, to: f64) -> f32 {
        let last = self.ground.len().saturating_sub(1);
        let index = |s: f64| (s.max(0.0).round() as usize).min(last);
        self.ground[index(from)..=index(to).max(index(from))]
            .iter()
            .copied()
            .fold(f32::MAX, f32::min)
    }

    /// The sections the mesh is made of, in order along the bridge: under
    /// piers and the abutments down to their footing, under each arch along
    /// its curve. Where the two meet a section comes twice, with either
    /// bottom (a pier's face).
    fn sections(&self) -> Vec<Section> {
        let mut out = Vec::new();
        let solid = |out: &mut Vec<Section>, from: f64, to: f64| {
            let bottom = self.lowest_ground(from, to) - FOOTING;
            let steps = ((to - from) / SECTION).ceil().max(1.0) as usize;
            let mut at: Vec<f64> = (0..=steps).map(|i| from + (to - from) * i as f64 / steps as f64).collect();
            // Where the road's dip toward an end begins.
            for s in [END_RAMP, self.length - END_RAMP] {
                if s > from + 0.05 && s < to - 0.05 {
                    at.push(s);
                }
            }
            at.sort_by(|a, b| a.total_cmp(b));
            for s in at {
                out.push(Section {
                    s,
                    bottom,
                    underside: None,
                    on_ground: true,
                });
            }
        };
        let mut previous = -END_REACH;
        for span in &self.spans {
            solid(&mut out, previous, span.from);
            let (half, rise) = (span.half(), span.rise);
            let steps = (half / 1.2).clamp(8.0, 28.0) as usize;
            for i in 0..=steps {
                // From the springing at its start (pi) over to its end (0).
                let angle = std::f32::consts::PI * (1.0 - i as f32 / steps as f32);
                let s = span.middle() + (half * angle.cos()) as f64;
                let bottom = span.spring_at(s) + rise * angle.sin();
                // Into the opening: against the ellipse's outward normal.
                let normal = Vector3::new(-angle.cos() / half, -angle.sin() / rise, 0.0).normalize();
                out.push(Section {
                    s,
                    bottom,
                    underside: Some([normal.x, normal.y]),
                    on_ground: false,
                });
            }
            previous = span.to;
        }
        solid(&mut out, previous, self.length + END_REACH);
        out
    }

    /// The bridge as meshes, in world metres. The stone's texture lies on
    /// it as on a wall built of it: its courses level and running on from
    /// one face to the next.
    pub fn mesh(&self) -> BridgeMesh {
        let mut mesh = BridgeMesh::default();
        let sections = self.sections();
        let frames: Vec<Frame> = sections.iter().map(|section| self.frame(section.s)).collect();
        let up = Vector3::unit_y();
        let wall = DECK_HALF_WIDTH - WALL_IN;
        let road_half = ROAD_HALF_WIDTH - KERB_WIDTH;
        let tile = |u: f32, v: f32| [u / STONE_TILE, v / STONE_TILE];
        // The outline across one side, from the road's edge over the kerb
        // and the parapet with its coping, out round the cornice and down
        // to the wall: `(out, up)` from the road's middle and height.
        let outline = [
            (road_half, 0.0),
            (road_half, KERB_HEIGHT),
            (ROAD_HALF_WIDTH, KERB_HEIGHT),
            (ROAD_HALF_WIDTH, PARAPET_HEIGHT),
            (ROAD_HALF_WIDTH - COPING_OUT, PARAPET_HEIGHT),
            (ROAD_HALF_WIDTH - COPING_OUT, PARAPET_HEIGHT + COPING_HEIGHT),
            (DECK_HALF_WIDTH + COPING_OUT, PARAPET_HEIGHT + COPING_HEIGHT),
            (DECK_HALF_WIDTH + COPING_OUT, PARAPET_HEIGHT),
            (DECK_HALF_WIDTH, PARAPET_HEIGHT),
            (DECK_HALF_WIDTH, -CORNICE_TOP),
            (DECK_HALF_WIDTH + CORNICE_OUT, -CORNICE_TOP),
            (DECK_HALF_WIDTH + CORNICE_OUT, -CORNICE_BOTTOM),
            (wall, -CORNICE_BOTTOM),
        ];
        let at = |frame: &Frame, out: f32, y: f32| frame.middle + frame.across * out + up * (y - frame.middle.y);
        // Where the stone ends under a section (a pier's or an abutment's
        // footing, an arch), and where the wall ends: there, under the
        // cornice at the highest (`deck` the road's height there).
        let foot = |_: &Frame, _: f32, section: &Section| section.bottom;
        let bottom = |frame: &Frame, u: f32, section: &Section, deck: f32| foot(frame, u, section).min(deck - CORNICE_BOTTOM);
        // How far round the arch above so far.
        let mut arc = 0.0f32;
        for i in 0..sections.len() - 1 {
            let (a, b) = (&sections[i], &sections[i + 1]);
            let (fa, fb) = (&frames[i], &frames[i + 1]);
            if b.s - a.s < 1e-6 {
                // A pier's face, facing into the opening beside it, from the
                // ground up to the springing.
                let opening = if a.on_ground { b } else { a };
                // (Before an opening it faces on along the bridge, after one
                // back.)
                let into = if a.on_ground { fa.along } else { -fa.along };
                let (low, high) = (a.bottom.min(b.bottom), opening.bottom);
                face(
                    &mut mesh.stone,
                    [at(fa, -wall, low), at(fa, wall, low), at(fa, wall, high), at(fa, -wall, high)],
                    into,
                    [tile(-wall, low), tile(wall, low), tile(wall, high), tile(-wall, high)],
                );
                continue;
            }
            let (sa, sb) = (a.s as f32, b.s as f32);
            let (da, db) = (fa.deck + DECK_RAISE, fb.deck + DECK_RAISE);
            // The road (and the foot of its kerbs), dipping at the ends.
            let (ra, rb) = (da - self.dive(a.s), db - self.dive(b.s));
            // The road.
            let road = |u: f32, s: f32| [u / ROAD_TILE, s / ROAD_TILE];
            face(
                &mut mesh.road,
                [at(fa, -road_half, ra), at(fa, road_half, ra), at(fb, road_half, rb), at(fb, -road_half, rb)],
                up,
                [road(-road_half, sa), road(road_half, sa), road(road_half, sb), road(-road_half, sb)],
            );
            // Each side: kerb, parapet, cornice, and the wall down to the
            // bottom.
            for side in [-1.0f32, 1.0] {
                let points: Vec<(f32, f32, f32)> = outline
                    .iter()
                    .enumerate()
                    .map(|(k, &(u, y))| if k == 0 { (u, ra, rb) } else { (u, da + y, db + y) })
                    .chain(std::iter::once((wall, bottom(fa, side * wall, a, da), bottom(fb, side * wall, b, db))))
                    .collect();
                for pair in points.windows(2) {
                    let ((u0, ya0, yb0), (u1, ya1, yb1)) = (pair[0], pair[1]);
                    let (du, dy) = (u1 - u0, ya1 - ya0);
                    let length = du.hypot(dy);
                    if length < 1e-4 && (yb1 - yb0).abs() < 1e-4 {
                        continue;
                    }
                    // Square to the outline, away from the stone.
                    let (nu, ny) = (-dy / length.max(1e-6), du / length.max(1e-6));
                    let normal = (fa.across * (side * nu) + up * ny).normalize();
                    // Upright faces by height, flat ones by how far out.
                    let uv = |s: f32, u: f32, y: f32| if dy.abs() >= du.abs() { tile(s, y) } else { tile(s, side * u) };
                    face(
                        &mut mesh.stone,
                        [
                            at(fa, side * u0, ya0),
                            at(fa, side * u1, ya1),
                            at(fb, side * u1, yb1),
                            at(fb, side * u0, yb0),
                        ],
                        normal,
                        [uv(sa, u0, ya0), uv(sa, u1, ya1), uv(sb, u1, yb1), uv(sb, u0, yb0)],
                    );
                }
            }
            // Under an arch, its underside: its courses running across the
            // bridge, as the vault is laid.
            if let (Some([na, ya]), Some(_)) = (a.underside, b.underside) {
                let normal = (fa.along * na + up * ya).normalize();
                let step = (b.bottom - a.bottom).hypot(sb - sa);
                // Where the ground lies over it, on the ground.
                let corners = [(fa, -wall, a), (fa, wall, a), (fb, wall, b), (fb, -wall, b)];
                face(
                    &mut mesh.stone,
                    corners.map(|(f, u, s)| at(f, u, foot(f, u, s))),
                    normal,
                    [tile(-wall, arc), tile(wall, arc), tile(wall, arc + step), tile(-wall, arc + step)],
                );
                arc += step;
            } else {
                arc = 0.0;
            }
        }
        self.add_rings(&mut mesh.stone);
        self.add_ends(&mut mesh.stone, &sections, &frames);
        mesh
    }

    /// How far the road dips `s` metres along the bridge (see
    /// `END_DIVE`).
    fn dive(&self, s: f64) -> f32 {
        let from_end = s.min(self.length - s);
        let t = (1.0 - from_end / END_RAMP).clamp(0.0, 1.0) as f32;
        END_DIVE * t * t * (3.0 - 2.0 * t)
    }

    /// A ring of stones round each arch on either side, standing out of
    /// the wall.
    fn add_rings(&self, stone: &mut MeshPart) {
        let up = Vector3::unit_y();
        let wall = DECK_HALF_WIDTH - WALL_IN;
        for span in &self.spans {
            let (half, rise, middle) = (span.half(), span.rise, span.middle());
            // As wide as fits under the cornice.
            let deck = self.frame(middle).deck;
            let room = deck - CORNICE_BOTTOM - 0.1 - (span.spring_at(middle) + rise);
            let width = RING_WIDTH.min(room);
            if width < 0.25 {
                continue;
            }
            let steps = (half / 1.2).clamp(8.0, 28.0) as usize;
            // Points on the inner and outer curve, and the ellipse's normal
            // `[along, up]` there.
            let curve = |angle: f32, grow: f32| {
                let s = middle + ((half + grow) * angle.cos()) as f64;
                let y = span.spring_at(s) + (rise + grow) * angle.sin();
                let n = Vector3::new(angle.cos() / (half + grow), angle.sin() / (rise + grow), 0.0).normalize();
                (s, y, [n.x, n.y])
            };
            for side in [-1.0f32, 1.0] {
                let (front, back) = (side * (wall + RING_OUT), side * wall);
                let mut run = 0.0;
                for i in 0..steps {
                    let angles = [i, i + 1].map(|k| PI * (1.0 - k as f32 / steps as f32));
                    let [(si0, yi0, n0), (si1, yi1, n1)] = angles.map(|a| curve(a, 0.0));
                    let [(so0, yo0, _), (so1, yo1, m1)] = angles.map(|a| curve(a, width));
                    let (fi0, fi1, fo0, fo1) = (self.frame(si0), self.frame(si1), self.frame(so0), self.frame(so1));
                    let step = (yi1 - yi0).hypot((si1 - si0) as f32);
                    // Voussoirs: the texture's courses across the ring.
                    let (v0, v1) = (run / STONE_TILE, (run + step) / STONE_TILE);
                    run += step;
                    let point = |f: &Frame, out: f32, y: f32| f.middle + f.across * out + up * (y - f.middle.y);
                    face(
                        stone,
                        [point(&fi0, front, yi0), point(&fo0, front, yo0), point(&fo1, front, yo1), point(&fi1, front, yi1)],
                        fi0.across * side,
                        [[0.0, v0], [width / STONE_TILE, v0], [width / STONE_TILE, v1], [0.0, v1]],
                    );
                    // Its outer edge, out of the wall.
                    let outward = (fo1.along * m1[0] + up * m1[1]).normalize();
                    face(
                        stone,
                        [point(&fo0, back, yo0), point(&fo0, front, yo0), point(&fo1, front, yo1), point(&fo1, back, yo1)],
                        outward,
                        [[0.0, v0], [RING_OUT / STONE_TILE, v0], [RING_OUT / STONE_TILE, v1], [0.0, v1]],
                    );
                    // Its inner edge, over the opening.
                    let inward = -(fi1.along * (0.5 * (n0[0] + n1[0])) + up * (0.5 * (n0[1] + n1[1]))).normalize();
                    face(
                        stone,
                        [point(&fi0, back, yi0), point(&fi0, front, yi0), point(&fi1, front, yi1), point(&fi1, back, yi1)],
                        inward,
                        [[0.0, v0], [RING_OUT / STONE_TILE, v0], [RING_OUT / STONE_TILE, v1], [0.0, v1]],
                    );
                }
                // Its feet, flat on the springing.
                for angle in [PI, 0.0] {
                    let ((si, yi, _), (so, yo, _)) = (curve(angle, 0.0), curve(angle, width));
                    let (fi, fo) = (self.frame(si), self.frame(so));
                    let point = |f: &Frame, out: f32, y: f32| f.middle + f.across * out + up * (y - f.middle.y);
                    face(
                        stone,
                        [point(&fi, back, yi), point(&fo, back, yo), point(&fo, front, yo), point(&fi, front, yi)],
                        -up,
                        [[0.0, 0.0], [width / STONE_TILE, 0.0], [width / STONE_TILE, RING_OUT / STONE_TILE], [0.0, RING_OUT / STONE_TILE]],
                    );
                }
            }
        }
    }

    /// The bridge's ends: closed across (down to the footing, the cornice
    /// and the road's height), and a post at each corner.
    fn add_ends(&self, stone: &mut MeshPart, sections: &[Section], frames: &[Frame]) {
        let up = Vector3::unit_y();
        let wall = DECK_HALF_WIDTH - WALL_IN;
        let tile = |u: f32, v: f32| [u / STONE_TILE, v / STONE_TILE];
        for (index, outward) in [(0, -1.0f32), (sections.len() - 1, 1.0)] {
            let (section, frame) = (&sections[index], &frames[index]);
            let normal = frame.along * outward;
            let deck = frame.deck + DECK_RAISE;
            let at = |out: f32, y: f32| frame.middle + frame.across * out + up * (y - frame.middle.y);
            // Across, in pieces that do not overlap: under the cornice,
            // the cornice, and up to the road.
            // Across, in pieces that do not overlap: under the cornice down
            // to the footing, the cornice, and up to the road.
            for (half, low, high) in [
                (wall, section.bottom, deck - CORNICE_BOTTOM),
                (DECK_HALF_WIDTH + CORNICE_OUT, deck - CORNICE_BOTTOM, deck - CORNICE_TOP),
                (DECK_HALF_WIDTH, deck - CORNICE_TOP, deck - END_DIVE),
            ] {
                face(
                    stone,
                    [at(-half, low), at(half, low), at(half, high), at(-half, high)],
                    normal,
                    [tile(-half, low), tile(half, low), tile(half, high), tile(-half, high)],
                );
            }
            // The posts: boxes over the kerb, the parapet and the coping.
            for side in [-1.0f32, 1.0] {
                let (inner, outer) = (ROAD_HALF_WIDTH - KERB_WIDTH - 0.05, DECK_HALF_WIDTH + POST_OUT);
                let (low, high) = (deck - CORNICE_BOTTOM - POST_UNDER, deck + PARAPET_HEIGHT + POST_HIGHER);
                // From just past the end (no face shared with it) inward.
                let (s0, s1) = if outward < 0.0 {
                    (section.s as f32 - 0.03, section.s as f32 + POST_LENGTH)
                } else {
                    (section.s as f32 + 0.03, section.s as f32 - POST_LENGTH)
                };
                let (f0, f1) = (self.frame(s0 as f64), self.frame(s1 as f64));
                let corner = |f: &Frame, out: f32, y: f32| f.middle + f.across * (side * out) + up * (y - f.middle.y);
                let (u0, u1) = (side * inner, side * outer);
                let sides = [
                    // End, far end, inner, outer, top.
                    ([corner(&f0, inner, low), corner(&f0, outer, low), corner(&f0, outer, high), corner(&f0, inner, high)], normal, [tile(u0, low), tile(u1, low), tile(u1, high), tile(u0, high)]),
                    ([corner(&f1, inner, low), corner(&f1, outer, low), corner(&f1, outer, high), corner(&f1, inner, high)], -normal, [tile(u0, low), tile(u1, low), tile(u1, high), tile(u0, high)]),
                    ([corner(&f0, inner, low), corner(&f1, inner, low), corner(&f1, inner, high), corner(&f0, inner, high)], f0.across * -side, [tile(s0, low), tile(s1, low), tile(s1, high), tile(s0, high)]),
                    ([corner(&f0, outer, low), corner(&f1, outer, low), corner(&f1, outer, high), corner(&f0, outer, high)], f0.across * side, [tile(s0, low), tile(s1, low), tile(s1, high), tile(s0, high)]),
                    ([corner(&f0, inner, high), corner(&f1, inner, high), corner(&f1, outer, high), corner(&f0, outer, high)], up, [tile(s0, u0), tile(s1, u0), tile(s1, u1), tile(s0, u1)]),
                    ([corner(&f0, inner, low), corner(&f1, inner, low), corner(&f1, outer, low), corner(&f0, outer, low)], -up, [tile(s0, u0), tile(s1, u0), tile(s1, u1), tile(s0, u1)]),
                ];
                for (corners, normal, uvs) in sides {
                    face(stone, corners, normal, uvs);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight(length: f64, level: f32) -> Vec<RoadSegment> {
        let n = (length / 8.0).ceil() as usize;
        (0..n)
            .map(|k| RoadSegment {
                a: [k as f64 * length / n as f64, 0.0],
                b: [(k + 1) as f64 * length / n as f64, 0.0],
                level: [level; 2],
            })
            .collect()
    }

    #[test]
    fn a_high_bridge_has_piers_and_round_arches() {
        // 120 m over a valley 30 m deep.
        let bridge = Bridge::new(straight(120.0, 30.0), |_| 0.0, |_| None);
        assert!(!bridge.piers.is_empty() && bridge.spans.len() == bridge.piers.len() + 1);
        for span in &bridge.spans {
            // Nearly half circles, springing well over the ground.
            assert!(span.rise > 0.8 * span.half(), "{span:?}");
            assert!(span.spring > 5.0, "{span:?}");
            assert!(span.spring + span.rise < 30.0 - DECK_THICKNESS + 0.01);
        }
    }

    #[test]
    fn piers_keep_out_of_a_river_a_span_can_reach_over() {
        // 90 m, 12 m high, a river 24 m wide in the middle.
        let water = |p: [f64; 2]| ((p[0] - 45.0).abs() < 12.0).then_some(0.0);
        let bridge = Bridge::new(straight(90.0, 12.0), |_| 0.0, water);
        assert!(!bridge.piers.is_empty());
        for &p in &bridge.piers {
            assert!((p - 45.0).abs() > 12.0 + PIER_THICKNESS * 0.5, "a pier in the river at {p}");
        }
        // Wider than any span: no span longer than the longest.
        let wide = |p: [f64; 2]| ((p[0] - 100.0).abs() < 60.0).then_some(0.0);
        let bridge = Bridge::new(straight(200.0, 12.0), |_| 0.0, wide);
        assert!(bridge.spans.iter().all(|s| s.to - s.from <= MAX_SPAN + 1e-6));
    }

    #[test]
    fn the_mesh_has_a_flat_road_between_parapets_and_faces_outward() {
        let bridge = Bridge::new(straight(120.0, 30.0), |_| 0.0, |_| None);
        let mesh = bridge.mesh();
        assert!(!mesh.road.is_empty() && !mesh.stone.is_empty());
        // The road: flat at the deck's height, within its width, facing up.
        for (p, n) in mesh.road.positions.iter().zip(&mesh.road.normals) {
            let dive = if p[0] < END_RAMP as f32 || p[0] > 120.0 - END_RAMP as f32 { END_DIVE } else { 0.0 };
            assert!(p[1] <= 30.0 + DECK_RAISE + 1e-3 && p[1] >= 30.0 + DECK_RAISE - dive - 1e-3, "{p:?}");
            assert!(p[2].abs() <= ROAD_HALF_WIDTH - KERB_WIDTH + 1e-3);
            assert!(n[1] > 0.99);
        }
        // Nothing of the stone over the corner posts or out past them.
        let top = 30.0 + DECK_RAISE + PARAPET_HEIGHT + POST_HIGHER;
        for p in &mesh.stone.positions {
            assert!(p[1] <= top + 1e-3 && p[2].abs() <= DECK_HALF_WIDTH + POST_OUT + 1e-3, "{p:?}");
        }
        // Over the parapets only the posts, at the ends.
        let coping = 30.0 + DECK_RAISE + PARAPET_HEIGHT + COPING_HEIGHT;
        for p in &mesh.stone.positions {
            assert!(p[1] <= coping + 1e-3 || p[0] < POST_LENGTH + 0.1 || p[0] > 120.0 - POST_LENGTH - 0.1, "{p:?}");
        }
        // Every triangle wound to face the way its normals do.
        for part in [&mesh.stone, &mesh.road] {
            let v = |i: u32| Vector3::from(part.positions[i as usize]);
            for &[a, b, c] in &part.triangles {
                let geometric = (v(b) - v(a)).cross(v(c) - v(a));
                if geometric.magnitude2() > 1e-8 {
                    assert!(geometric.dot(Vector3::from(part.normals[a as usize])) > 0.0);
                }
            }
        }
        // Air under each arch's middle: no stone between the ground and its
        // underside.
        for span in &bridge.spans {
            let mid = span.middle() as f32;
            let crown = span.spring + span.rise;
            let under = mesh
                .stone
                .positions
                .iter()
                .filter(|p| (p[0] - mid).abs() < 0.5 && p[1] < crown - 0.1)
                .count();
            assert_eq!(under, 0, "stone under the arch at {mid}");
        }
    }
}
