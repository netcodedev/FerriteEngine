//! Bridges: where a road crosses a river or a gorge, or a dip in the
//! ground too deep for an embankment, it runs on a stone bridge, made of
//! the ground's own volume (see `Bridge::solid`), so it is drawn, walked on
//! and collided with like the ground.
//!
//! A bridge is a deck with a parapet each side, over arches that spring
//! from piers. The piers stand as far apart as the bridge is high (a low
//! bridge has short spans and flat arches, a high viaduct tall piers and
//! round arches), and out of the water where a span can reach over it:
//! only a river wider than the longest span gets a pier in it.

use super::roads::{RoadSegment, ROAD_HALF_WIDTH};

/// The parapets: how thick (at least a metre, so the ground's 1 m samples
/// always catch one) and how high over the road.
pub const PARAPET_THICKNESS: f32 = 1.0;
pub const PARAPET_HEIGHT: f32 = 1.0;
/// Half the deck's width: the road and a parapet each side.
pub const DECK_HALF_WIDTH: f32 = ROAD_HALF_WIDTH + PARAPET_THICKNESS;
/// From the road down to the crown of an arch.
const DECK_THICKNESS: f32 = 1.4;
/// How far the deck reaches past each end of the bridge, into the
/// ground it lands on.
const END_REACH: f64 = 2.0;
/// A pier's thickness along the bridge at the arches' springing, and how
/// much wider it grows each side per metre further down.
const PIER_THICKNESS: f64 = 2.6;
const BATTER: f32 = 0.04;
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
    /// How far `(s, y)` lies outside the opening (negative inside):
    /// straight sides under the springing, narrowing downward as the piers
    /// widen, and half an ellipse over it.
    fn outside(&self, s: f64, y: f32) -> f32 {
        let half = (0.5 * (self.to - self.from)) as f32;
        let along = (s - 0.5 * (self.from + self.to)) as f32;
        let ds = along.abs();
        let dy = y - self.spring - self.slope * along;
        if dy <= 0.0 {
            return ds - (half + BATTER * dy).max(0.3);
        }
        // An ellipse's implicit function over its gradient: close to the
        // distance near the arch, and as at the sides where it springs.
        let (a, b) = (half, self.rise);
        let f = (ds / a).powi(2) + (dy / b).powi(2) - 1.0;
        let g = 2.0 * ((ds / (a * a)).hypot(dy / (b * b)));
        f / g.max(2.0 / a.max(b))
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
    /// The box around it, `[x, z]`, with the deck's width.
    lo: [f64; 2],
    hi: [f64; 2],
    /// The highest point of its parapets.
    pub top: f32,
}

/// Where a point lies by a bridge: how far along it, how far beside its
/// middle, and the road's height there.
#[derive(Clone, Copy, Debug)]
struct Along {
    s: f64,
    across: f32,
    deck: f32,
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
        let mut best = (f64::INFINITY, 0.0, 0.0f32);
        for (k, p) in self.pieces.iter().enumerate() {
            let (dx, dz) = (p.b[0] - p.a[0], p.b[1] - p.a[1]);
            let run2 = (dx * dx + dz * dz).max(1e-12);
            let mut t = ((x - p.a[0]) * dx + (z - p.a[1]) * dz) / run2;
            let (low, high) = (if k == 0 { f64::MIN } else { 0.0 }, if k == last { f64::MAX } else { 1.0 });
            t = t.clamp(low, high);
            let d = (x - p.a[0] - dx * t).hypot(z - p.a[1] - dz * t);
            if d < best.0 {
                let level = p.level[0] + (p.level[1] - p.level[0]) * t.clamp(0.0, 1.0) as f32;
                best = (d, self.starts[k] + t * run2.sqrt(), level);
            }
        }
        Along {
            s: best.1,
            across: best.0 as f32,
            deck: best.2,
        }
    }

    /// How far world `(x, y, z)` lies outside the bridge's stone (negative
    /// inside), roughly in metres: the deck and its parapets, down to the
    /// ground but for the openings under its arches.
    pub fn solid(&self, x: f64, y: f32, z: f64) -> f32 {
        if !self.in_box(x, z, 0.0) || y > self.top + 1.0 {
            let dx = (self.lo[0] - x).max(x - self.hi[0]).max(0.0);
            let dz = (self.lo[1] - z).max(z - self.hi[1]).max(0.0);
            return (dx.hypot(dz) as f32).max(y - self.top).max(0.5);
        }
        let Along { s, across, deck } = self.along(x, z);
        let ends = ((-END_REACH - s).max(s - self.length - END_REACH)) as f32;
        let side = across - DECK_HALF_WIDTH;
        let road = side.max(y - deck).max(ends);
        let parapet = side.max(ROAD_HALF_WIDTH - across).max(y - deck - PARAPET_HEIGHT).max(ends);
        let mut body = road.min(parapet);
        for span in &self.spans {
            if s > span.from - 4.0 && s < span.to + 4.0 {
                body = body.max(-span.outside(s, y));
            }
        }
        body
    }

    /// Whether world `(x, y, z)` (on the surface, facing up) is on the
    /// road over the bridge.
    pub fn on_deck(&self, x: f64, y: f32, z: f64) -> bool {
        if !self.in_box(x, z, 0.0) {
            return false;
        }
        let a = self.along(x, z);
        a.across < ROAD_HALF_WIDTH + 0.3
            && (y - a.deck).abs() < 0.6
            && a.s > -END_REACH
            && a.s < self.length + END_REACH
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
    fn a_bridge_has_a_deck_parapets_and_open_arches() {
        // 120 m over a valley 30 m deep.
        let bridge = Bridge::new(straight(120.0, 30.0), |_| 0.0, |_| None);
        assert!(!bridge.piers.is_empty() && bridge.spans.len() == bridge.piers.len() + 1);
        // Road: stone under it, air over it up to a lorry's height.
        for s in [5.0, 40.0, 61.0, 100.0] {
            assert!(bridge.solid(s, 29.5, 0.0) < 0.0, "no deck at {s}");
            assert!(bridge.solid(s, 30.3, 1.0) > 0.0 && bridge.solid(s, 33.0, 0.0) > 0.0);
            // Parapets beside it, nothing beyond.
            assert!(bridge.solid(s, 30.5, (ROAD_HALF_WIDTH + 0.5) as f64) < 0.0);
            assert!(bridge.solid(s, 30.5, (DECK_HALF_WIDTH + 0.5) as f64) > 0.0);
        }
        // Under each arch's middle air down to the ground, and the piers
        // stone all the way down.
        for span in &bridge.spans {
            let mid = 0.5 * (span.from + span.to);
            for y in [0.5, 10.0, span.spring + span.rise * 0.5] {
                assert!(bridge.solid(mid, y, 0.0) > 0.0, "no opening at {mid}, {y}");
            }
        }
        for &p in &bridge.piers {
            for y in [0.5, 10.0, 25.0] {
                assert!(bridge.solid(p, y, 0.0) < 0.0, "no pier at {p}, {y}");
            }
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
        // Wider than any span: one pier stands in it.
        let wide = |p: [f64; 2]| ((p[0] - 100.0).abs() < 60.0).then_some(0.0);
        let bridge = Bridge::new(straight(200.0, 12.0), |_| 0.0, wide);
        assert!(bridge.spans.iter().all(|s| s.to - s.from <= MAX_SPAN + 1e-6));
    }
}
