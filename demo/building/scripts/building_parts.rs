//! What can be built and what it is made of: the blueprints (foundation,
//! wall, wall with a door frame), the parts each one is built from (stones,
//! beams) and their meshes. Plain data and geometry, no drawing: see
//! `builder.rs` for the game side.
//!
//! Every blueprint has its own frame: x to the right along it, y up, z
//! across. A part is a box in that frame (its slot). A part rests on the
//! parts right below it and can only be put in once they are in.

use std::f32::consts::FRAC_PI_2;

use ferrite::core::model::generated::MeshPart;
use ferrite::prelude::*;

/// Side of a foundation, in metres: walls are as wide.
pub const FOUNDATION_SIZE: f32 = 3.0;
/// How far a foundation reaches below its top: into the ground on a slope.
pub const FOUNDATION_DEPTH: f32 = 1.25;
pub const WALL_HEIGHT: f32 = 3.0;
pub const WALL_THICKNESS: f32 = 0.24;
const BEAM_THICKNESS: f32 = 0.2;
const BEAM_HEIGHT: f32 = 0.3;
const BEAM_ROWS: usize = 10;
const DOOR_WIDTH: f32 = 1.0;
const DOOR_ROWS: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    Stone,
    Beam,
}

impl Material {
    pub fn name(self) -> &'static str {
        match self {
            Material::Stone => "Stein",
            Material::Beam => "Balken",
        }
    }

    pub fn plural(self) -> &'static str {
        match self {
            Material::Stone => "Steine",
            Material::Beam => "Balken",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Foundation,
    Wall,
    DoorWall,
}

impl PieceKind {
    pub const ALL: [PieceKind; 3] = [PieceKind::Foundation, PieceKind::Wall, PieceKind::DoorWall];

    pub fn name(self) -> &'static str {
        match self {
            PieceKind::Foundation => "Fundament",
            PieceKind::Wall => "Wand",
            PieceKind::DoorWall => "Türrahmen",
        }
    }

    pub fn is_wall(self) -> bool {
        matches!(self, PieceKind::Wall | PieceKind::DoorWall)
    }

    pub fn material(self) -> Material {
        match self {
            PieceKind::Foundation => Material::Stone,
            PieceKind::Wall | PieceKind::DoorWall => Material::Beam,
        }
    }
}

/// One part of a blueprint: a box in the blueprint's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub material: Material,
    pub center: Vector3<f32>,
    /// Half its size along x, y and z.
    pub half: Vector3<f32>,
    /// Which of a few shapes of its size it gets (stones differ).
    pub variant: u32,
    /// A post at the end of a wall: two walls meeting at a corner share it.
    pub corner_post: bool,
}

impl Slot {
    fn new(material: Material, center: [f32; 3], size: [f32; 3], variant: u32) -> Self {
        Slot {
            material,
            center: center.into(),
            half: Vector3::from(size) * 0.5,
            variant,
            corner_post: false,
        }
    }

    pub fn min(&self) -> Vector3<f32> {
        self.center - self.half
    }

    pub fn max(&self) -> Vector3<f32> {
        self.center + self.half
    }

    /// The mesh it is drawn with (its size and variant).
    pub fn shape(&self) -> ShapeKey {
        ShapeKey {
            material: self.material,
            size_mm: [self.half.x, self.half.y, self.half.z].map(|h| (h * 2000.0).round() as u32),
            variant: self.variant,
        }
    }
}

/// The parts of a blueprint and which ones each part rests on.
#[derive(Clone, Debug)]
pub struct Blueprint {
    pub kind: PieceKind,
    pub slots: Vec<Slot>,
    /// For each slot, the slots right below it that carry it.
    pub supports: Vec<Vec<usize>>,
}

impl Blueprint {
    pub fn of(kind: PieceKind) -> Blueprint {
        let slots = match kind {
            PieceKind::Foundation => foundation_slots(),
            PieceKind::Wall => wall_slots(false),
            PieceKind::DoorWall => wall_slots(true),
        };
        let supports = (0..slots.len()).map(|i| supports_of(&slots, i)).collect();
        Blueprint { kind, slots, supports }
    }
}

/// A foundation, its top at y = 0: nine big blocks below, seven flagstones
/// in staggered rows on them.
fn foundation_slots() -> Vec<Slot> {
    let mut slots = Vec::new();
    let flag = 0.25;
    let block = FOUNDATION_DEPTH - flag;
    let third = FOUNDATION_SIZE / 3.0;
    for (i, (x, z)) in [-1.0f32, 0.0, 1.0]
        .iter()
        .flat_map(|z| [-1.0f32, 0.0, 1.0].map(|x| (x, *z)))
        .enumerate()
    {
        slots.push(Slot::new(
            Material::Stone,
            [x * third, -flag - block * 0.5, z * third],
            [third, block, third],
            i as u32 % 4,
        ));
    }
    // Rows of two long flagstones, and between them a row with a long
    // one in the middle and half ones at its ends.
    let half = FOUNDATION_SIZE / 2.0;
    let rows: [&[(f32, f32)]; 3] = [
        &[(-half / 2.0, half), (half / 2.0, half)],
        &[(-half + half / 4.0, half / 2.0), (0.0, half), (half - half / 4.0, half / 2.0)],
        &[(-half / 2.0, half), (half / 2.0, half)],
    ];
    for (row, z) in rows.iter().zip([-1.0f32, 0.0, 1.0]) {
        for (j, (x, length)) in row.iter().enumerate() {
            slots.push(Slot::new(
                Material::Stone,
                [*x, -flag * 0.5, z * third],
                [*length, flag, third],
                4 + (j as u32 + (z + 1.0) as u32) % 3,
            ));
        }
    }
    slots
}

/// A timber wall standing at y = 0: a post at each end and beams laid
/// between them, row on row. With a door, two door posts frame the opening,
/// short beams fill both sides of it and a lintel spans it.
fn wall_slots(door: bool) -> Vec<Slot> {
    let mut slots = Vec::new();
    let end = FOUNDATION_SIZE / 2.0;
    let post_x = end - WALL_THICKNESS / 2.0;
    for x in [-post_x, post_x] {
        let mut post = Slot::new(
            Material::Beam,
            [x, WALL_HEIGHT / 2.0, 0.0],
            [WALL_THICKNESS, WALL_HEIGHT, WALL_THICKNESS],
            0,
        );
        post.corner_post = true;
        slots.push(post);
    }
    let inner = end - WALL_THICKNESS;
    let door_post = 0.2;
    if door {
        let x = DOOR_WIDTH / 2.0 + door_post / 2.0;
        let height = DOOR_ROWS as f32 * BEAM_HEIGHT;
        for x in [-x, x] {
            slots.push(Slot::new(
                Material::Beam,
                [x, height / 2.0, 0.0],
                [door_post, height, WALL_THICKNESS],
                1,
            ));
        }
    }
    for row in 0..BEAM_ROWS {
        let y = (row as f32 + 0.5) * BEAM_HEIGHT;
        let variant = 2 + row as u32 % 3;
        if door && row < DOOR_ROWS {
            let from = DOOR_WIDTH / 2.0 + door_post;
            let length = inner - from;
            for side in [-1.0, 1.0] {
                slots.push(Slot::new(
                    Material::Beam,
                    [side * (from + length / 2.0), y, 0.0],
                    [length, BEAM_HEIGHT, BEAM_THICKNESS],
                    variant,
                ));
            }
        } else {
            slots.push(Slot::new(
                Material::Beam,
                [0.0, y, 0.0],
                [inner * 2.0, BEAM_HEIGHT, BEAM_THICKNESS],
                variant,
            ));
        }
    }
    slots
}

/// The slots right below slot `i` that it rests on: their tops touch its
/// bottom, and they overlap it seen from above.
fn supports_of(slots: &[Slot], i: usize) -> Vec<usize> {
    let slot = &slots[i];
    let (min, max) = (slot.min(), slot.max());
    slots
        .iter()
        .enumerate()
        .filter(|(j, other)| {
            let (omin, omax) = (other.min(), other.max());
            *j != i
                && (omax.y - min.y).abs() < 0.01
                && omin.x < max.x - 0.01
                && omax.x > min.x + 0.01
                && omin.z < max.z - 0.01
                && omax.z > min.z + 0.01
        })
        .map(|(j, _)| j)
        .collect()
}

/// A blueprint put into the world: where it stands and which parts are in.
#[derive(Clone, Debug)]
pub struct Piece {
    pub kind: PieceKind,
    /// A foundation's top centre, or the foot of a wall in its middle.
    pub position: Vector3<f32>,
    /// Turn about y, in radians.
    pub yaw: f32,
    pub filled: Vec<bool>,
    /// A slot that is another piece's (the post two walls share at a
    /// corner): drawn and filled there.
    pub links: Vec<Option<(usize, usize)>>,
    /// The foundation a wall stands on.
    pub foundation: Option<usize>,
}

impl Piece {
    pub fn new(kind: PieceKind, position: Vector3<f32>, yaw: f32, blueprint: &Blueprint) -> Piece {
        let n = blueprint.slots.len();
        Piece {
            kind,
            position,
            yaw,
            filled: vec![false; n],
            links: vec![None; n],
            foundation: None,
        }
    }

    pub fn rotation(&self) -> Quaternion<f32> {
        Quaternion::from_angle_y(Rad(self.yaw))
    }

    pub fn transform(&self) -> Matrix4<f32> {
        Matrix4::from_translation(self.position) * Matrix4::from(self.rotation())
    }

    /// A point of the piece's frame in the world.
    pub fn to_world(&self, local: Vector3<f32>) -> Vector3<f32> {
        self.position + self.rotation() * local
    }

    pub fn to_local(&self, world: Vector3<f32>) -> Vector3<f32> {
        self.rotation().conjugate() * (world - self.position)
    }

    /// Where the middle of the foundation edge a wall stands on lies.
    pub fn edge_middle(&self) -> Vector3<f32> {
        self.to_world(Vector3::new(0.0, 0.0, WALL_THICKNESS / 2.0))
    }
}

/// A foundation's neighbour cells and the walls along its edges: `k`
/// quarter turns from its own frame.
pub fn quarter(yaw: f32, k: i32) -> f32 {
    yaw + k as f32 * FRAC_PI_2
}

/// Where a wall on edge `k` of `foundation` stands (its foot, in the
/// middle) and its turn. Its outer face is flush with the edge.
pub fn wall_on_edge(foundation: &Piece, k: i32) -> (Vector3<f32>, f32) {
    let yaw = quarter(foundation.yaw, k);
    let inward = FOUNDATION_SIZE / 2.0 - WALL_THICKNESS / 2.0;
    let position = foundation.position + Quaternion::from_angle_y(Rad(yaw)) * Vector3::new(0.0, 0.0, inward);
    (position, yaw)
}

/// Where a foundation next to `foundation` on side `k` would stand.
pub fn foundation_beside(foundation: &Piece, k: i32) -> Vector3<f32> {
    let yaw = quarter(foundation.yaw, k);
    foundation.position + Quaternion::from_angle_y(Rad(yaw)) * Vector3::new(0.0, 0.0, FOUNDATION_SIZE)
}

/// Where a ray (in the frame of a box centred at the origin with
/// half-size `half`) enters it, if it does within `max` metres.
pub fn ray_box(origin: Vector3<f32>, dir: Vector3<f32>, half: Vector3<f32>, max: f32) -> Option<f32> {
    let mut near = 0.0f32;
    let mut far = max;
    for axis in 0..3 {
        let (o, d, h) = (origin[axis], dir[axis], half[axis]);
        if d.abs() < 1e-8 {
            if o.abs() > h {
                return None;
            }
            continue;
        }
        let (mut t0, mut t1) = ((-h - o) / d, (h - o) / d);
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        near = near.max(t0);
        far = far.min(t1);
        if near > far {
            return None;
        }
    }
    Some(near)
}

/// Where a ray from `origin` along `dir` (normalized, world) enters
/// `slot` of `piece`, grown by `grow` on every side.
pub fn ray_slot(piece: &Piece, slot: &Slot, origin: Vector3<f32>, dir: Vector3<f32>, max: f32, grow: f32) -> Option<f32> {
    let local_origin = piece.to_local(origin) - slot.center;
    let local_dir = piece.rotation().conjugate() * dir;
    ray_box(local_origin, local_dir, slot.half + Vector3::new(grow, grow, grow), max)
}

/// The shape of a part's mesh: its size in millimetres and variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShapeKey {
    pub material: Material,
    pub size_mm: [u32; 3],
    pub variant: u32,
}

impl ShapeKey {
    /// The part's colour (sRGB): greys for stones, browns for wood.
    pub fn color(&self) -> [f32; 3] {
        match self.material {
            Material::Stone => [
                [0.52, 0.50, 0.46],
                [0.44, 0.43, 0.42],
                [0.58, 0.55, 0.50],
                [0.48, 0.46, 0.43],
                [0.62, 0.60, 0.56],
                [0.56, 0.55, 0.53],
                [0.66, 0.63, 0.58],
            ][self.variant as usize % 7],
            Material::Beam => [
                [0.36, 0.24, 0.14],
                [0.33, 0.22, 0.13],
                [0.46, 0.31, 0.18],
                [0.42, 0.28, 0.16],
                [0.50, 0.34, 0.20],
            ][self.variant as usize % 5],
        }
    }

    /// The mesh: a box with bevelled edges, rough and a little uneven for
    /// a stone, straight and only hewn for a beam. Centred on the origin;
    /// a stone is a bit smaller than its slot, for the joints.
    pub fn mesh(&self) -> MeshPart {
        let size = Vector3::from(self.size_mm.map(|s| s as f32 / 1000.0));
        let seed = self.variant.wrapping_mul(7919) ^ self.size_mm[0] ^ self.size_mm[1] << 8 ^ self.size_mm[2] << 16;
        match self.material {
            Material::Stone => {
                let gap = 0.02;
                let half = size * 0.5 - Vector3::new(gap, gap, gap);
                let bevel = (half.x.min(half.y).min(half.z) * 0.35).min(0.09);
                rough_box(half, bevel, 0.022, 3, seed)
            }
            Material::Beam => {
                let half = size * 0.5 - Vector3::new(0.004, 0.006, 0.0);
                rough_box(half, 0.035, 0.006, 4, seed)
            }
        }
    }
}

/// Noise in -1..1 from a point and a seed: the same point always moves the
/// same way, so faces meeting at an edge stay closed.
fn hash3(p: Vector3<f32>, seed: u32) -> Vector3<f32> {
    let q = [p.x, p.y, p.z].map(|v| (v * 1000.0).round() as i32 as u32);
    let mut h = seed ^ 0x9e37_79b9;
    let mut out = [0.0f32; 3];
    for (i, value) in out.iter_mut().enumerate() {
        h = h
            .wrapping_add(q[0].wrapping_mul(73_856_093))
            .wrapping_add(q[1].wrapping_mul(19_349_663))
            .wrapping_add(q[2].wrapping_mul(83_492_791))
            .wrapping_add(i as u32 * 0x632b_e5ab);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *value = (h & 0xffff) as f32 / 32_767.5 - 1.0;
    }
    Vector3::from(out)
}

/// The grid along one axis of a face: the corners, the start of the bevel
/// at each end, and `inner` steps between.
fn axis_steps(half: f32, bevel: f32, inner: usize) -> Vec<f32> {
    let mut steps = vec![-half, -half + bevel];
    for i in 1..inner {
        let t = i as f32 / inner as f32;
        steps.push(-half + bevel + t * 2.0 * (half - bevel));
    }
    steps.extend([half - bevel, half]);
    steps
}

/// A box of half-size `half` with edges bevelled by `bevel`, every point
/// pushed up to `rough` metres aside, flat shaded (each triangle its own
/// normal), so a stone shows its facets.
pub fn rough_box(half: Vector3<f32>, bevel: f32, rough: f32, inner: usize, seed: u32) -> MeshPart {
    let inner_half = half - Vector3::new(bevel, bevel, bevel);
    let shape = |p: Vector3<f32>| -> Vector3<f32> {
        let clamped = Vector3::new(
            p.x.clamp(-inner_half.x, inner_half.x),
            p.y.clamp(-inner_half.y, inner_half.y),
            p.z.clamp(-inner_half.z, inner_half.z),
        );
        let out = p - clamped;
        let rounded = if out.magnitude2() > 1e-12 { clamped + out.normalize() * bevel } else { p };
        rounded + hash3(p, seed) * rough
    };
    let mut mesh = MeshPart::default();
    // Each face: its normal axis and sign, and the two axes spanning it
    // (counter-clockwise seen from outside).
    let faces: [(usize, f32, usize, usize); 6] = [
        (0, 1.0, 2, 1),
        (0, -1.0, 1, 2),
        (1, 1.0, 0, 2),
        (1, -1.0, 2, 0),
        (2, 1.0, 1, 0),
        (2, -1.0, 0, 1),
    ];
    for (n_axis, sign, u_axis, v_axis) in faces {
        // u x v must point out of the face: swap where it does not.
        let (u_axis, v_axis) = {
            let mut u = Vector3::zero();
            u[u_axis] = 1.0;
            let mut v = Vector3::zero();
            v[v_axis] = 1.0;
            if u.cross(v)[n_axis] * sign > 0.0 {
                (u_axis, v_axis)
            } else {
                (v_axis, u_axis)
            }
        };
        let us = axis_steps(half[u_axis], bevel, inner);
        let vs = axis_steps(half[v_axis], bevel, inner);
        let point = |a: f32, b: f32| {
            let mut p = Vector3::zero();
            p[n_axis] = sign * half[n_axis];
            p[u_axis] = a;
            p[v_axis] = b;
            shape(p)
        };
        for i in 0..us.len() - 1 {
            for j in 0..vs.len() - 1 {
                let a = point(us[i], vs[j]);
                let b = point(us[i + 1], vs[j]);
                let c = point(us[i + 1], vs[j + 1]);
                let d = point(us[i], vs[j + 1]);
                for [p, q, r] in [[a, b, c], [a, c, d]] {
                    let normal = (q - p).cross(r - p);
                    if normal.magnitude2() < 1e-14 {
                        continue;
                    }
                    let normal = normal.normalize();
                    let uv = |v: Vector3<f32>| [v[u_axis], v[v_axis]];
                    let (i0, i1, i2) = (
                        mesh.vertex(p, normal, uv(p)),
                        mesh.vertex(q, normal, uv(q)),
                        mesh.vertex(r, normal, uv(r)),
                    );
                    mesh.triangles.push([i0, i1, i2]);
                }
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_foundation_is_nine_blocks_and_seven_flagstones() {
        let blueprint = Blueprint::of(PieceKind::Foundation);
        assert_eq!(blueprint.slots.len(), 16);
        // The blocks rest on the ground, every flagstone on blocks.
        for (i, supports) in blueprint.supports.iter().enumerate() {
            if i < 9 {
                assert!(supports.is_empty(), "block {i}");
            } else {
                assert!(!supports.is_empty() && supports.iter().all(|s| *s < 9), "flagstone {i}");
            }
        }
        // The top is flat at y = 0 and covers the whole square.
        let area: f32 = blueprint.slots[9..].iter().map(|s| s.half.x * s.half.z * 4.0).sum();
        assert!((area - FOUNDATION_SIZE * FOUNDATION_SIZE).abs() < 1e-3);
        assert!(blueprint.slots[9..].iter().all(|s| s.max().y.abs() < 1e-5));
    }

    #[test]
    fn wall_beams_rest_on_the_row_below() {
        let wall = Blueprint::of(PieceKind::Wall);
        assert_eq!(wall.slots.len(), 2 + BEAM_ROWS);
        for (i, slot) in wall.slots.iter().enumerate() {
            let bottom = slot.min().y;
            if bottom < 1e-4 {
                assert!(wall.supports[i].is_empty());
            } else {
                assert_eq!(wall.supports[i].len(), 1, "beam {i}");
            }
        }
        assert!((wall.slots.iter().map(|s| s.max().y).fold(0.0, f32::max) - WALL_HEIGHT).abs() < 1e-4);
    }

    #[test]
    fn a_door_wall_leaves_the_opening_free_and_spans_it_with_a_lintel() {
        let wall = Blueprint::of(PieceKind::DoorWall);
        let door_top = DOOR_ROWS as f32 * BEAM_HEIGHT;
        // Nothing in the opening.
        for slot in &wall.slots {
            let in_opening = slot.min().x < DOOR_WIDTH / 2.0 - 1e-4
                && slot.max().x > -DOOR_WIDTH / 2.0 + 1e-4
                && slot.min().y < door_top - 1e-4;
            assert!(!in_opening, "{slot:?}");
        }
        // The lintel rests on both door posts and the beams beside them.
        let lintel = wall
            .slots
            .iter()
            .position(|s| (s.min().y - door_top).abs() < 1e-4)
            .unwrap();
        assert_eq!(wall.supports[lintel].len(), 4);
    }

    #[test]
    fn walls_on_the_edges_of_a_foundation_meet_at_its_corners() {
        let blueprint = Blueprint::of(PieceKind::Foundation);
        let foundation = Piece::new(PieceKind::Foundation, Vector3::new(10.0, 5.0, -3.0), 0.3, &blueprint);
        let wall_print = Blueprint::of(PieceKind::Wall);
        let (p0, y0) = wall_on_edge(&foundation, 0);
        let (p1, y1) = wall_on_edge(&foundation, 1);
        let a = Piece::new(PieceKind::Wall, p0, y0, &wall_print);
        let b = Piece::new(PieceKind::Wall, p1, y1, &wall_print);
        let posts = |w: &Piece| -> Vec<Vector3<f32>> {
            wall_print.slots.iter().filter(|s| s.corner_post).map(|s| w.to_world(s.center)).collect()
        };
        let shared = posts(&a)
            .iter()
            .filter(|p| posts(&b).iter().any(|q| (*p - q).magnitude() < 1e-3))
            .count();
        assert_eq!(shared, 1);
        // The wall's outer face is flush with the edge.
        let edge = foundation.to_world(Vector3::new(0.0, 0.0, FOUNDATION_SIZE / 2.0));
        assert!((a.edge_middle() - edge).magnitude() < 1e-4);
    }

    #[test]
    fn a_foundation_beside_another_touches_it() {
        let blueprint = Blueprint::of(PieceKind::Foundation);
        let a = Piece::new(PieceKind::Foundation, Vector3::new(0.0, 0.0, 0.0), 0.7, &blueprint);
        for k in 0..4 {
            let p = foundation_beside(&a, k);
            assert!(((p - a.position).magnitude() - FOUNDATION_SIZE).abs() < 1e-4);
        }
    }

    #[test]
    fn rays_find_boxes() {
        let half = Vector3::new(0.5, 0.5, 0.5);
        let t = ray_box(Vector3::new(-3.0, 0.0, 0.0), Vector3::unit_x(), half, 10.0);
        assert!((t.unwrap() - 2.5).abs() < 1e-5);
        assert_eq!(ray_box(Vector3::new(-3.0, 2.0, 0.0), Vector3::unit_x(), half, 10.0), None);
        assert_eq!(ray_box(Vector3::new(-3.0, 0.0, 0.0), Vector3::unit_x(), half, 2.0), None);
    }

    #[test]
    fn part_meshes_are_closed_and_inside_their_slot() {
        for kind in PieceKind::ALL {
            for slot in Blueprint::of(kind).slots {
                let mesh = slot.shape().mesh();
                assert!(!mesh.triangles.is_empty());
                for p in &mesh.positions {
                    for axis in 0..3 {
                        assert!(p[axis].abs() <= slot.half[axis] + 0.03, "{kind:?} {p:?}");
                    }
                }
            }
        }
    }
}
