//! Trees grown from a handful of numbers: a trunk, levels of branches
//! that fork off it, and leaves on the last of them, as cards (a twig of
//! leaves each) or as fronds (long strips, as on a palm or the flat sprigs
//! of a fir). The same parameters make a fir, an oak, a birch or a palm
//! (see `TreeParams::preset`); the same seed makes the same tree.

use std::f32::consts::{PI, TAU};

use cgmath::{InnerSpace, Vector3};
use serde::{Deserialize, Serialize};

/// The outline of the crown: which branches up the trunk are the longest.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrownShape {
    /// Longest at the bottom (a fir).
    Conical,
    /// Longest in the middle (an oak).
    Round,
    /// About as long all the way up (a poplar).
    Columnar,
    /// Longest at the top (a palm, an acacia).
    Umbrella,
}

impl CrownShape {
    pub const ALL: [CrownShape; 4] = [Self::Conical, Self::Round, Self::Columnar, Self::Umbrella];

    pub fn name(self) -> &'static str {
        match self {
            Self::Conical => "conical",
            Self::Round => "round",
            Self::Columnar => "columnar",
            Self::Umbrella => "umbrella",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// How long a branch is at `t` (0 at the lowest branch, 1 at the top),
    /// relative to the longest.
    fn length_at(self, t: f32) -> f32 {
        match self {
            Self::Conical => 1.0 - 0.85 * t,
            Self::Round => 0.35 + 0.65 * (PI * (0.15 + 0.75 * t)).sin(),
            Self::Columnar => 0.85 + 0.15 * (PI * t).sin(),
            Self::Umbrella => 0.4 + 0.6 * t,
        }
    }
}

/// What grows on the last level of branches.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foliage {
    None,
    /// Square cards, each a twig of leaves (or needles) from the leaf
    /// texture, along the twigs.
    Cards,
    /// The last branches themselves are strips of leaf, folded a little
    /// along their middle, the texture running along them.
    Fronds,
}

impl Foliage {
    pub const ALL: [Foliage; 3] = [Self::None, Self::Cards, Self::Fronds];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Cards => "cards",
            Self::Fronds => "fronds",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// Everything a tree is made from. Lengths in metres, angles in degrees.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TreeParams {
    pub seed: u64,
    /// The trunk: how tall, how thick at its foot, how much thinner at its
    /// top (share of its foot), how much wider it flares at the ground
    /// (share of its radius), how far it leans and how much it wanders.
    pub height: f32,
    pub trunk_radius: f32,
    pub taper: f32,
    pub flare: f32,
    pub lean: f32,
    pub bend: f32,
    /// Levels of branches (0: a bare pole with leaves at its top).
    pub levels: u32,
    /// Branches on the trunk, and on each branch of a further level.
    pub branches: u32,
    pub twigs: u32,
    /// Where up the trunk the branches start (share of its height).
    pub branch_start: f32,
    /// Angle between a branch and what it grows from.
    pub branch_angle: f32,
    /// The longest branch (share of the trunk's height), and each twig
    /// (share of the branch it grows from).
    pub branch_length: f32,
    pub twig_length: f32,
    pub crown: CrownShape,
    /// How much branches bend down toward their tips (negative: up).
    pub droop: f32,
    /// A branch's radius, as a share of what it grows from there.
    pub branch_radius: f32,
    pub foliage: Foliage,
    /// A leaf card's size, and how many grow on each metre of twig.
    pub leaf_size: f32,
    pub leaf_density: f32,
    /// A frond's greatest width.
    pub frond_width: f32,
    /// How many metres of bark one repeat of its texture covers.
    pub bark_tile: f32,
}

impl Default for TreeParams {
    fn default() -> Self {
        Self::preset("oak").unwrap()
    }
}

impl TreeParams {
    pub const PRESETS: [&'static str; 5] = ["fir", "oak", "birch", "palm", "bush"];

    /// A tree of a well-known kind.
    pub fn preset(name: &str) -> Option<Self> {
        let base = Self {
            seed: 1,
            height: 8.0,
            trunk_radius: 0.35,
            taper: 0.2,
            flare: 0.3,
            lean: 0.0,
            bend: 0.15,
            levels: 2,
            branches: 8,
            twigs: 6,
            branch_start: 0.3,
            branch_angle: 50.0,
            branch_length: 0.6,
            twig_length: 0.5,
            crown: CrownShape::Round,
            droop: 0.1,
            branch_radius: 0.55,
            foliage: Foliage::Cards,
            leaf_size: 0.8,
            leaf_density: 4.0,
            frond_width: 0.5,
            bark_tile: 1.5,
        };
        Some(match name {
            "fir" => Self {
                height: 16.0,
                trunk_radius: 0.32,
                taper: 0.04,
                flare: 0.25,
                bend: 0.04,
                // Each branch a flat spray of needles.
                levels: 1,
                branches: 90,
                twigs: 0,
                branch_start: 0.1,
                branch_angle: 98.0,
                branch_length: 0.3,
                crown: CrownShape::Conical,
                droop: 0.25,
                branch_radius: 0.3,
                foliage: Foliage::Fronds,
                frond_width: 1.5,
                ..base
            },
            "oak" => Self {
                height: 9.0,
                trunk_radius: 0.45,
                taper: 0.3,
                flare: 0.35,
                bend: 0.25,
                levels: 3,
                branches: 7,
                twigs: 5,
                branch_start: 0.4,
                branch_angle: 52.0,
                branch_length: 0.75,
                twig_length: 0.5,
                crown: CrownShape::Round,
                droop: -0.05,
                branch_radius: 0.55,
                foliage: Foliage::Cards,
                leaf_size: 0.9,
                leaf_density: 6.0,
                ..base
            },
            "birch" => Self {
                height: 13.0,
                trunk_radius: 0.2,
                taper: 0.1,
                flare: 0.15,
                bend: 0.12,
                levels: 2,
                branches: 18,
                twigs: 7,
                branch_start: 0.35,
                branch_angle: 40.0,
                branch_length: 0.32,
                twig_length: 0.55,
                crown: CrownShape::Columnar,
                droop: 0.45,
                branch_radius: 0.4,
                foliage: Foliage::Cards,
                leaf_size: 0.6,
                leaf_density: 9.0,
                ..base
            },
            "palm" => Self {
                height: 10.0,
                trunk_radius: 0.22,
                taper: 0.65,
                flare: 0.5,
                lean: 8.0,
                bend: 0.2,
                levels: 1,
                branches: 16,
                twigs: 0,
                branch_start: 0.97,
                branch_angle: 65.0,
                branch_length: 0.38,
                crown: CrownShape::Umbrella,
                droop: 0.9,
                branch_radius: 0.25,
                foliage: Foliage::Fronds,
                frond_width: 1.3,
                bark_tile: 0.8,
                ..base
            },
            "bush" => Self {
                height: 0.6,
                trunk_radius: 0.08,
                taper: 0.5,
                flare: 0.0,
                bend: 0.3,
                levels: 2,
                branches: 9,
                twigs: 5,
                branch_start: 0.0,
                branch_angle: 35.0,
                branch_length: 2.0,
                twig_length: 0.5,
                crown: CrownShape::Columnar,
                droop: 0.1,
                branch_radius: 0.6,
                foliage: Foliage::Cards,
                leaf_size: 0.45,
                leaf_density: 7.0,
                ..base
            },
            _ => return None,
        })
    }

    /// The same numbers within the ranges the generator handles.
    pub fn sanitized(&self) -> Self {
        Self {
            height: self.height.clamp(0.1, 80.0),
            trunk_radius: self.trunk_radius.clamp(0.01, 4.0),
            taper: self.taper.clamp(0.0, 1.0),
            flare: self.flare.clamp(0.0, 2.0),
            lean: self.lean.clamp(-60.0, 60.0),
            bend: self.bend.clamp(0.0, 1.0),
            levels: self.levels.min(MAX_LEVELS),
            branches: self.branches.min(200),
            twigs: self.twigs.min(40),
            branch_start: self.branch_start.clamp(0.0, 1.0),
            branch_angle: self.branch_angle.clamp(0.0, 170.0),
            branch_length: self.branch_length.clamp(0.0, 3.0),
            twig_length: self.twig_length.clamp(0.0, 2.0),
            droop: self.droop.clamp(-2.0, 2.0),
            branch_radius: self.branch_radius.clamp(0.05, 1.0),
            leaf_size: self.leaf_size.clamp(0.05, 5.0),
            leaf_density: self.leaf_density.clamp(0.0, 40.0),
            frond_width: self.frond_width.clamp(0.02, 5.0),
            bark_tile: self.bark_tile.clamp(0.1, 20.0),
            ..self.clone()
        }
    }
}

/// Most levels of branches.
pub const MAX_LEVELS: u32 = 3;
/// Most triangles a tree may have, so a slider pulled too far cannot
/// stall the editor.
const MAX_TRIANGLES: usize = 400_000;
/// How far apart a leaf card's middle sits from its twig, as a share of
/// its size.
const CARD_REACH: f32 = 0.45;
/// How much a frond is folded along its middle, radians.
const FROND_FOLD: f32 = 0.35;

/// A triangle mesh: positions, normals, texture coordinates.
#[derive(Default, Debug, Clone)]
pub struct MeshPart {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
}

impl MeshPart {
    fn vertex(&mut self, p: Vector3<f32>, n: Vector3<f32>, uv: [f32; 2]) -> u32 {
        self.positions.push(p.into());
        self.normals.push(n.into());
        self.uvs.push(uv);
        (self.positions.len() - 1) as u32
    }

    /// A quad seen from both sides (foliage): each side faces its own way
    /// for the winding, but both carry the normal `n`, so a leaf is lit
    /// the same from either side.
    fn two_sided(&mut self, corners: [u32; 4]) {
        let [a, b, c, d] = corners;
        self.triangles.extend([[a, b, c], [a, c, d], [a, c, b], [a, d, c]]);
    }
}

/// A grown tree: its bark and its leaves.
#[derive(Default, Debug, Clone)]
pub struct TreeMesh {
    pub bark: MeshPart,
    pub leaves: MeshPart,
}

impl TreeMesh {
    pub fn triangle_count(&self) -> usize {
        self.bark.triangles.len() + self.leaves.triangles.len()
    }

    /// The box around it, `(min, max)`.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in self.bark.positions.iter().chain(&self.leaves.positions) {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        (min, max)
    }
}

/// One stem (the trunk, a branch or a twig) as points along its middle
/// with its radius at each.
struct Stem {
    points: Vec<Vector3<f32>>,
    radii: Vec<f32>,
    level: u32,
}

impl Stem {
    fn length(&self) -> f32 {
        self.points.windows(2).map(|p| (p[1] - p[0]).magnitude()).sum()
    }

    /// The point, direction and radius `t` of the way along it.
    fn at(&self, t: f32) -> (Vector3<f32>, Vector3<f32>, f32) {
        let n = self.points.len() - 1;
        let f = (t.clamp(0.0, 1.0) * n as f32).min(n as f32 - 1e-4);
        let i = f.floor() as usize;
        let k = f - i as f32;
        let (a, b) = (self.points[i], self.points[i + 1]);
        let direction = (b - a).normalize();
        (a + (b - a) * k, direction, self.radii[i] + (self.radii[i + 1] - self.radii[i]) * k)
    }
}

/// A small, fast, seeded random source (splitmix64).
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..1`.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `-1..1`.
    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// Some direction square to `v` (of any length).
fn perpendicular(v: Vector3<f32>) -> Vector3<f32> {
    let v = v.normalize();
    let other = if v.y.abs() < 0.9 { Vector3::unit_y() } else { Vector3::unit_x() };
    v.cross(other).normalize()
}

/// `v` turned by `angle` about the unit axis `axis` (Rodrigues).
fn rotate(v: Vector3<f32>, axis: Vector3<f32>, angle: f32) -> Vector3<f32> {
    let (s, c) = angle.sin_cos();
    v * c + axis.cross(v) * s + axis * axis.dot(v) * (1.0 - c)
}

/// Grow the tree `params` describes.
pub fn grow(params: &TreeParams) -> TreeMesh {
    let p = params.sanitized();
    let mut random = Random(p.seed.wrapping_mul(0x2545F4914F6CDD1D) ^ 0x5DEECE66D);
    let mut stems: Vec<Stem> = Vec::new();

    // The trunk.
    let lean_axis = rotate(Vector3::unit_x(), Vector3::unit_y(), random.unit() * TAU);
    let up = rotate(Vector3::unit_y(), lean_axis, p.lean.to_radians());
    let trunk_steps = 14;
    stems.push(grow_stem(
        &mut random,
        Vector3::new(0.0, 0.0, 0.0),
        up,
        p.height,
        p.trunk_radius,
        p.taper,
        p.bend,
        0.0,
        trunk_steps,
        0,
    ));
    // Flared at its foot.
    if let Some(trunk) = stems.first_mut() {
        let n = trunk.radii.len() - 1;
        for (i, r) in trunk.radii.iter_mut().enumerate() {
            let s = i as f32 / n as f32;
            *r *= 1.0 + p.flare * (1.0 - (s * 6.0).min(1.0)).powi(2);
        }
    }

    // The branches, level by level.
    let golden = PI * (3.0 - 5f32.sqrt());
    let mut level_start = 0;
    for level in 1..=p.levels {
        let level_end = stems.len();
        let count = if level == 1 { p.branches } else { p.twigs };
        for parent in level_start..level_end {
            let parent_length = stems[parent].length();
            let parent_radius = stems[parent].radii[0];
            let start = if level == 1 { p.branch_start } else { 0.25 };
            let mut azimuth = random.unit() * TAU;
            for i in 0..count {
                // Evenly up the parent, a little jittered.
                let k = (i as f32 + 0.5 + 0.35 * random.signed()) / count.max(1) as f32;
                let t = start + (1.0 - start) * k.clamp(0.0, 1.0);
                let (origin, tangent, radius_there) = stems[parent].at(t);
                let length = if level == 1 {
                    p.height * p.branch_length * p.crown.length_at(k) * (0.85 + 0.3 * random.unit())
                } else {
                    // Twigs shorter toward their branch's tip.
                    parent_length * p.twig_length * (1.0 - 0.6 * t) * (0.8 + 0.4 * random.unit())
                };
                if length < 0.05 {
                    continue;
                }
                azimuth += golden * (1.0 + 0.1 * random.signed());
                let side = rotate(perpendicular(tangent), tangent, azimuth);
                let angle = (p.branch_angle + 8.0 * random.signed()).to_radians();
                let direction = (tangent * angle.cos() + side * angle.sin()).normalize();
                let radius = (radius_there * p.branch_radius)
                    .min(parent_radius * 0.9)
                    .max(0.004);
                let steps = match level {
                    1 => 8,
                    2 => 6,
                    _ => 4,
                };
                // Fronds need more points to curve smoothly.
                let fronds = p.foliage == Foliage::Fronds && level == p.levels;
                let steps = if fronds { 12 } else { steps };
                stems.push(grow_stem(
                    &mut random,
                    origin,
                    direction,
                    length,
                    radius,
                    0.12,
                    p.bend * 0.6,
                    p.droop,
                    steps,
                    level,
                ));
            }
        }
        level_start = level_end;
    }

    // The mesh: bark round every stem but fronds, leaves on the last level.
    let mut mesh = TreeMesh::default();
    let last = p.levels;
    for stem in &stems {
        let frond = p.foliage == Foliage::Fronds && stem.level == last && last > 0;
        if frond {
            add_frond(&mut mesh.leaves, stem, p.frond_width);
        } else {
            let sides = match stem.level {
                0 => 12,
                1 => 7,
                2 => 5,
                _ => 4,
            };
            add_tube(&mut mesh.bark, stem, sides, p.bark_tile, p.trunk_radius);
        }
        if p.foliage == Foliage::Cards && stem.level == last {
            // On a bare pole only toward its top.
            let from = if last == 0 { 0.75 } else { 0.2 };
            add_cards(&mut mesh.leaves, stem, from, p.leaf_size, p.leaf_density, &mut random);
        }
        if mesh.triangle_count() > MAX_TRIANGLES {
            log::warn!("tree: more than {MAX_TRIANGLES} triangles, the rest left out");
            break;
        }
    }
    mesh
}

/// A stem from `origin` growing `direction`, `length` long in `steps`,
/// thinning from `radius` to `radius * tip`, wandering by `bend` and
/// bending down by `droop` toward its tip.
#[allow(clippy::too_many_arguments)]
fn grow_stem(
    random: &mut Random,
    origin: Vector3<f32>,
    direction: Vector3<f32>,
    length: f32,
    radius: f32,
    tip: f32,
    bend: f32,
    droop: f32,
    steps: usize,
    level: u32,
) -> Stem {
    let step = length / steps as f32;
    let mut points = vec![origin];
    let mut radii = vec![radius];
    let mut direction = direction.normalize();
    let mut at = origin;
    for i in 1..=steps {
        let s = i as f32 / steps as f32;
        // Wander a little, and droop: more toward the tip.
        let wander = Vector3::new(random.signed(), random.signed() * 0.3, random.signed()) * (bend * 0.35);
        direction = (direction + wander - Vector3::unit_y() * (droop * 2.2 * s / steps as f32)).normalize();
        at += direction * step;
        points.push(at);
        radii.push(radius * (1.0 + (tip - 1.0) * s));
    }
    Stem { points, radii, level }
}

/// A tube round `stem`, `sides` around, the bark texture repeating every
/// `tile` metres along it and as often round it as fits the trunk.
fn add_tube(part: &mut MeshPart, stem: &Stem, sides: usize, tile: f32, trunk_radius: f32) {
    let first = part.positions.len() as u32;
    // Round the trunk this many repeats (the same on every stem, so the
    // texture keeps its size on thinner ones only roughly).
    let around = ((TAU * trunk_radius / tile).round()).max(1.0);
    let mut normal = perpendicular(stem.points[1] - stem.points[0]);
    let mut run = 0.0;
    let n = stem.points.len();
    for i in 0..n {
        let tangent = if i + 1 < n {
            (stem.points[i + 1] - stem.points[i]).normalize()
        } else {
            (stem.points[i] - stem.points[i - 1]).normalize()
        };
        // Parallel transport: the ring keeps its twist from point to point.
        normal = (normal - tangent * normal.dot(tangent)).normalize();
        let binormal = tangent.cross(normal);
        if i > 0 {
            run += (stem.points[i] - stem.points[i - 1]).magnitude();
        }
        for j in 0..=sides {
            let a = j as f32 / sides as f32 * TAU;
            let out = normal * a.cos() + binormal * a.sin();
            part.vertex(
                stem.points[i] + out * stem.radii[i],
                out,
                [j as f32 / sides as f32 * around, run / tile],
            );
        }
    }
    let row = (sides + 1) as u32;
    for i in 0..(n - 1) as u32 {
        for j in 0..sides as u32 {
            let a = first + i * row + j;
            let (b, c, d) = (a + 1, a + row + 1, a + row);
            // Counter-clockwise seen from outside.
            part.triangles.push([a, c, d]);
            part.triangles.push([a, b, c]);
        }
    }
}

/// Leaf cards along `stem` from `from` of the way to its tip, `density` per
/// metre, each `size` across, turned every way round it and up a little.
fn add_cards(part: &mut MeshPart, stem: &Stem, from: f32, size: f32, density: f32, random: &mut Random) {
    let count = (stem.length() * (1.0 - from) * density).round() as usize;
    // And one at the tip.
    for i in 0..=count {
        let t = if i == count { 1.0 } else { from + (1.0 - from) * (i as f32 + random.unit()) / count.max(1) as f32 };
        let (at, tangent, _) = stem.at(t);
        let side = rotate(perpendicular(tangent), tangent, random.unit() * TAU);
        // Out from the twig, toward its tip and a little up.
        let out = (side + tangent * 0.6 + Vector3::unit_y() * 0.3).normalize();
        let across = out.cross(tangent).normalize();
        let across = if across.x.is_finite() { across } else { perpendicular(out) };
        let size = size * (0.75 + 0.5 * random.unit());
        let (base, top) = (at - out * (size * (0.5 - CARD_REACH)), at + out * (size * (0.5 + CARD_REACH)));
        let half = across * (size * 0.5);
        // Lit as if it faced out from the twig and up: a crown of cards
        // shades as one round mass.
        let normal = (out + Vector3::unit_y() * 0.5).normalize();
        let a = part.vertex(base - half, normal, [0.0, 0.0]);
        let b = part.vertex(base + half, normal, [1.0, 0.0]);
        let c = part.vertex(top + half, normal, [1.0, 1.0]);
        let d = part.vertex(top - half, normal, [0.0, 1.0]);
        part.two_sided([a, b, c, d]);
    }
}

/// `stem` as a frond: a strip `width` across at its widest, folded down a
/// little along its middle, the texture running from its foot (0) to its
/// tip (1) and across it from 0 to 1.
fn add_frond(part: &mut MeshPart, stem: &Stem, width: f32) {
    let n = stem.points.len();
    let first = part.positions.len() as u32;
    for i in 0..n {
        let s = i as f32 / (n - 1) as f32;
        let tangent = if i + 1 < n {
            (stem.points[i + 1] - stem.points[i]).normalize()
        } else {
            (stem.points[i] - stem.points[i - 1]).normalize()
        };
        // Flat across, as level as the frond lets it be.
        let mut across = tangent.cross(Vector3::unit_y());
        if across.magnitude2() < 1e-6 {
            across = perpendicular(tangent);
        }
        let across = across.normalize();
        let up = across.cross(tangent).normalize();
        let up = if up.y < 0.0 { -up } else { up };
        // Widest a third of the way, narrowing to a point at the tip.
        let half = 0.5 * width * (PI * (0.08 + 0.92 * s)).sin().max(0.0).powf(0.7);
        let (fold_cos, fold_sin) = (FROND_FOLD.cos(), FROND_FOLD.sin());
        let left = stem.points[i] + (-across * fold_cos - up * fold_sin) * half;
        let right = stem.points[i] + (across * fold_cos - up * fold_sin) * half;
        part.vertex(left, up, [0.0, s]);
        part.vertex(stem.points[i], up, [0.5, s]);
        part.vertex(right, up, [1.0, s]);
    }
    for i in 0..(n - 1) as u32 {
        let row = first + i * 3;
        let next = row + 3;
        part.two_sided([row, row + 1, next + 1, next]);
        part.two_sided([row + 1, row + 2, next + 2, next + 1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_grows_bark_and_leaves() {
        for name in TreeParams::PRESETS {
            let params = TreeParams::preset(name).unwrap();
            let mesh = grow(&params);
            assert!(!mesh.bark.triangles.is_empty(), "{name}: no bark");
            assert!(!mesh.leaves.triangles.is_empty(), "{name}: no leaves");
            let (min, max) = mesh.bounds();
            // About as tall as asked, with its crown on top (a bush is
            // mostly branches).
            let crown = params.height * params.branch_length + params.leaf_size;
            assert!(max[1] > params.height * 0.8 && max[1] < params.height * 1.3 + crown, "{name}: {max:?}");
            assert!(min[1] > -2.0, "{name}: reaches {min:?} under the ground");
            for part in [&mesh.bark, &mesh.leaves] {
                let n = part.positions.len() as u32;
                assert!(part.triangles.iter().flatten().all(|&i| i < n));
                assert_eq!(part.normals.len(), part.positions.len());
                assert!(part.positions.iter().flatten().all(|v| v.is_finite()));
            }
        }
    }

    #[test]
    fn the_same_seed_grows_the_same_tree_and_another_does_not() {
        let oak = TreeParams::preset("oak").unwrap();
        assert_eq!(grow(&oak).bark.positions, grow(&oak).bark.positions);
        let other = TreeParams { seed: 2, ..oak };
        assert_ne!(grow(&other).bark.positions, grow(&TreeParams::preset("oak").unwrap()).bark.positions);
    }

    #[test]
    fn a_fir_is_widest_at_the_bottom_and_an_oak_in_the_middle() {
        let width_at = |mesh: &TreeMesh, low: f32, high: f32| {
            mesh.leaves
                .positions
                .iter()
                .filter(|p| p[1] >= low && p[1] < high)
                .map(|p| p[0].hypot(p[2]))
                .fold(0.0f32, f32::max)
        };
        let fir = grow(&TreeParams::preset("fir").unwrap());
        assert!(width_at(&fir, 3.0, 6.0) > 1.5 * width_at(&fir, 11.0, 14.0));
        let oak = TreeParams::preset("oak").unwrap();
        let mesh = grow(&oak);
        let (_, max) = mesh.bounds();
        let third = max[1] / 3.0;
        assert!(width_at(&mesh, third, 2.0 * third) > width_at(&mesh, 2.0 * third, max[1]));
    }

    #[test]
    fn bark_faces_outward() {
        // Each triangle wound counter-clockwise seen from where its
        // normals point, or the outside of the trunk is culled.
        for name in TreeParams::PRESETS {
            let bark = grow(&TreeParams::preset(name).unwrap()).bark;
            let v = |i: u32| Vector3::from(bark.positions[i as usize]);
            let (mut outward, mut total) = (0, 0);
            for &[a, b, c] in &bark.triangles {
                let face = (v(b) - v(a)).cross(v(c) - v(a));
                if face.magnitude2() < 1e-12 {
                    continue;
                }
                let normal = Vector3::from(bark.normals[a as usize]);
                total += 1;
                outward += (face.dot(normal) > 0.0) as usize;
            }
            assert!(outward * 100 >= total * 99, "{name}: {outward} of {total} face out");
        }
    }

    #[test]
    fn a_straight_trunk_is_a_trunk() {
        // No wandering: the trunk runs straight up, which once left it
        // without a direction square to it (and its vertices not numbers).
        for name in TreeParams::PRESETS {
            let params = TreeParams { bend: 0.0, lean: 0.0, ..TreeParams::preset(name).unwrap() };
            let mesh = grow(&params);
            for part in [&mesh.bark, &mesh.leaves] {
                assert!(part.positions.iter().chain(&part.normals).flatten().all(|c| c.is_finite()), "{name}");
            }
        }
    }

    #[test]
    fn wild_numbers_stay_within_bounds() {
        let params = TreeParams {
            levels: 99,
            branches: 10_000,
            twigs: 10_000,
            leaf_density: 1e9,
            height: -5.0,
            ..TreeParams::default()
        };
        let mesh = grow(&params);
        assert!(mesh.triangle_count() <= MAX_TRIANGLES + 20_000);
    }
}
