//! How a model is held in a character's hand: a grip, set on the model's
//! asset (`ModelSettings::grip`), and the maths that puts the model into a
//! hand of a posed skeleton. Used by `HeldItem` in the game and by the
//! editor's preview of a held model.
//!
//! The hand is found from the bone names (`RightHand`, its fingers), so it
//! works for any rig named like Mixamo's; the fist is measured from the
//! finger joints, so a grip is given in the item's terms only.

use std::cell::Cell;

use cgmath::{
    EuclideanSpace, InnerSpace, Matrix, Matrix3, Matrix4, Point3, Quaternion, Rad, Rotation3,
    SquareMatrix, Transform, Vector3,
};
use serde::{Deserialize, Serialize};

use super::{
    ik::{base_name, ChainKind, Skeleton},
    Model,
};

/// Which hand holds an item (the other one takes a second grip).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hand {
    Right,
    Left,
}

impl Hand {
    fn name(self) -> &'static str {
        match self {
            Hand::Right => "Right",
            Hand::Left => "Left",
        }
    }

    pub fn other(self) -> Hand {
        match self {
            Hand::Right => Hand::Left,
            Hand::Left => Hand::Right,
        }
    }
}

/// How a model is held, in the model's own space (metres, as it is drawn,
/// after its import scale).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grip {
    /// Where the hand closes around the model.
    pub point: [f32; 3],
    /// The model's direction that comes out of the fist on the thumb's
    /// side: along a handle, towards the head.
    pub along: [f32; 3],
    /// The model's direction the knuckles face: an axe's edge.
    pub facing: [f32; 3],
    /// Degrees the model is turned in the hand about `along`, after
    /// `facing` (for edges that do not face along an axis).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roll: f32,
    /// Where the other hand holds it, if both hands do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_hand: Option<[f32; 3]>,
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

impl Grip {
    /// A grip at `point`, along the model's x axis, edge up.
    pub fn at(point: [f32; 3]) -> Self {
        Self {
            point,
            along: [1.0, 0.0, 0.0],
            facing: [0.0, 1.0, 0.0],
            roll: 0.0,
            other_hand: None,
        }
    }

    /// This grip, or one at its point (`Grip::at`) when its directions give
    /// no frame (an `along` of zero length, a `facing` along `along`, a
    /// value that is no number), as a grip set in the editor can.
    fn usable(&self) -> Grip {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let along = Vector3::from(self.along);
        let facing = Vector3::from(self.facing);
        let square = along.magnitude2() > 1e-12
            && (facing - along * (facing.dot(along) / along.magnitude2())).magnitude2() > 1e-12;
        let point = if finite(&self.point) { self.point } else { [0.0; 3] };
        if square
            && finite(&self.along)
            && finite(&self.facing)
            && self.roll.is_finite()
            && finite(&self.point)
        {
            return self.clone();
        }
        Grip {
            other_hand: self.other_hand.filter(|p| finite(p)),
            ..Grip::at(point)
        }
    }

    /// The facing direction after the roll, square to `along`.
    fn rolled_facing(&self) -> Vector3<f32> {
        let along = Vector3::from(self.along).normalize();
        let facing = Vector3::from(self.facing);
        let facing = (facing - along * facing.dot(along)).normalize();
        Quaternion::from_axis_angle(along, Rad(self.roll.to_radians())) * facing
    }
}

/// How far into the hand a held handle lies: along the fingers, as a part
/// of the way from the wrist to the knuckles, and out of the palm, metres.
const ALONG_FINGERS: f32 = 0.75;
const OUT_OF_PALM: f32 = 0.025;
/// Beyond the arm's length the other hand lets go, all of it once the
/// grip is this much (a part of the arm's length) further away.
const LET_GO_OVER: f32 = 0.15;
/// A hand turned about the forearm (against how the animation holds it)
/// is turned half at the wrist, there at most `WRIST_TWIST_MAX` radians,
/// and the rest at the forearm, at most `FOREARM_TWIST_MAX` (as an arm
/// turns the hand mostly by turning the forearm): a rig without twist
/// bones pinches a joint twisted far (the skin of a twisted wrist goes
/// thin as an hourglass). Twist beyond both is left out.
const WRIST_TWIST_SHARE: f32 = 0.5;
const WRIST_TWIST_MAX: f32 = 45.0 * std::f32::consts::PI / 180.0;
const FOREARM_TWIST_MAX: f32 = 90.0 * std::f32::consts::PI / 180.0;

/// The fist of one hand in world space: where a handle lies in it, which
/// way the thumb, the knuckles and the palm face.
#[derive(Clone, Copy, Debug)]
struct Fist {
    hand: usize,
    centre: Point3<f32>,
    thumb: Vector3<f32>,
    knuckles: Vector3<f32>,
    /// From the wrist to the knuckles.
    length: f32,
}

/// The joints of a hand: the hand, the middle, index and little finger's
/// first joints.
fn hand_joints(skeleton: &Skeleton, hand: Hand) -> Option<[usize; 4]> {
    let side = hand.name();
    let find = |part: &str| {
        let wanted = format!("{side}{part}");
        skeleton
            .joints
            .iter()
            .position(|joint| base_name(&joint.name) == wanted)
    };
    Some([
        find("Hand")?,
        find("HandMiddle1")?,
        find("HandIndex1")?,
        find("HandPinky1")?,
    ])
}

/// The palm's direction for a fist with these thumb and knuckle
/// directions: down in a T-pose, so mirrored between the hands.
fn palm_of(hand: Hand, thumb: Vector3<f32>, knuckles: Vector3<f32>) -> Vector3<f32> {
    match hand {
        Hand::Right => thumb.cross(knuckles),
        Hand::Left => knuckles.cross(thumb),
    }
}

/// The fist of `hand` of `skeleton`, whose joints are at `positions` (as
/// `Skeleton::world_positions` has them).
fn fist(skeleton: &Skeleton, hand: Hand, positions: &[Point3<f32>]) -> Option<Fist> {
    let [wrist, middle, index, pinky] = hand_joints(skeleton, hand)?;
    let knuckles = positions[middle] - positions[wrist];
    let length = knuckles.magnitude();
    let knuckles = knuckles.normalize();
    let across = positions[index] - positions[pinky];
    let thumb = (across - knuckles * across.dot(knuckles)).normalize();
    let palm = palm_of(hand, thumb, knuckles);
    let centre = positions[wrist] + knuckles * (length * ALONG_FINGERS) + palm * OUT_OF_PALM;
    // Joints on top of each other give no directions.
    let finite = |v: Vector3<f32>| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    if !(finite(thumb) && finite(knuckles) && finite(centre.to_vec())) {
        return None;
    }
    Some(Fist {
        hand: wrist,
        centre,
        thumb,
        knuckles,
        length,
    })
}

/// Columns: x, y and z of a right-handed frame with `x` along `a` and `y`
/// as close to `b` as is square to it.
fn frame(a: Vector3<f32>, b: Vector3<f32>) -> Matrix3<f32> {
    let x = a.normalize();
    let y = (b - x * b.dot(x)).normalize();
    Matrix3::from_cols(x, y, x.cross(y))
}

/// Whether every entry of `m` is a number: a NaN transform must not reach
/// the shaders.
fn is_finite_matrix(m: &Matrix4<f32>) -> bool {
    let entries: &[f32; 16] = m.as_ref();
    entries.iter().all(|v| v.is_finite())
}

/// The rotation without scale of a transform.
fn rotation_of(m: &Matrix4<f32>) -> Matrix3<f32> {
    Matrix3::from_cols(
        m.x.truncate().normalize(),
        m.y.truncate().normalize(),
        m.z.truncate().normalize(),
    )
}

/// Where the model held by `grip` is (its transform in world space) when
/// held in `hand` by the character `model`, drawn at `transform`. `None`
/// if the character has no such hand.
pub fn held_transform(
    model: &Model,
    transform: &Matrix4<f32>,
    grip: &Grip,
    hand: Hand,
) -> Option<Matrix4<f32>> {
    let skeleton = model.skeleton()?;
    let positions = skeleton.world_positions(&model.root_transform(transform));
    let fist = fist(&skeleton, hand, &positions)?;
    let placed = place(&grip.usable(), &fist);
    is_finite_matrix(&placed).then_some(placed)
}

/// The raw bits of a matrix, to tell whether it is exactly the same.
fn matrix_bits(m: &Matrix4<f32>) -> [u32; 16] {
    let entries: &[f32; 16] = m.as_ref();
    entries.map(f32::to_bits)
}

impl Grip {
    /// The raw bits of the grip, to tell whether it is exactly the same.
    fn bits(&self) -> [u32; 14] {
        let [p, a, f] = [self.point, self.along, self.facing].map(|v| v.map(f32::to_bits));
        let other = self.other_hand.map_or([0; 3], |v| v.map(f32::to_bits));
        [
            p[0],
            p[1],
            p[2],
            a[0],
            a[1],
            a[2],
            f[0],
            f[1],
            f[2],
            self.roll.to_bits(),
            self.other_hand.is_some() as u32,
            other[0],
            other[1],
            other[2],
        ]
    }
}

/// What `held_transform` is worked out from: the character's pose
/// (`Model::pose_revision`), its root transform, the grip and the hand.
type PlacementKey = (u64, [u32; 16], [u32; 14], Hand);

/// `held_transform`, remembered for the inputs it was last worked out
/// for: a frame draws a held item in up to about nine passes, all with the
/// same pose.
#[derive(Default)]
pub struct HeldPlacement {
    last: Cell<Option<(PlacementKey, Option<Matrix4<f32>>)>>,
}

impl HeldPlacement {
    /// `held_transform(model, transform, grip, hand)`, worked out again
    /// only when one of them changed.
    pub fn get(
        &self,
        model: &Model,
        transform: &Matrix4<f32>,
        grip: &Grip,
        hand: Hand,
    ) -> Option<Matrix4<f32>> {
        let key = (
            model.pose_revision(),
            matrix_bits(&model.root_transform(transform)),
            grip.bits(),
            hand,
        );
        self.get_or_place(key, || held_transform(model, transform, grip, hand))
    }

    fn get_or_place(
        &self,
        key: PlacementKey,
        place: impl FnOnce() -> Option<Matrix4<f32>>,
    ) -> Option<Matrix4<f32>> {
        if let Some((last, placed)) = self.last.get() {
            if last == key {
                return placed;
            }
        }
        let placed = place();
        self.last.set(Some((key, placed)));
        placed
    }
}

fn place(grip: &Grip, fist: &Fist) -> Matrix4<f32> {
    let item = frame(grip.along.into(), grip.rolled_facing());
    let hand = frame(fist.thumb, fist.knuckles);
    // Item axes onto the fist's: the item's frame inverted (transposed),
    // then the hand's.
    let rotation = hand * item.transpose();
    Matrix4::from_translation(fist.centre.to_vec())
        * Matrix4::from(rotation)
        * Matrix4::from_translation(-Vector3::from(grip.point))
}

/// Two-handed: the other hand of the character `model` (drawn at
/// `transform`) reaches the grip's second point and closes around the
/// handle as the holding hand does, `weight` of the way (0: as the
/// animation has it, 1: fully). Out of the arm's reach it lets go.
pub fn reach_with_other_hand(
    model: &mut Model,
    transform: &Matrix4<f32>,
    grip: &Grip,
    hand: Hand,
    weight: f32,
) {
    let grip = &grip.usable();
    if grip.other_hand.is_none() || weight <= 0.0 {
        return;
    }
    let Some(mut skeleton) = model.skeleton() else {
        return;
    };
    if reach(
        &mut skeleton,
        &model.root_transform(transform),
        grip,
        hand,
        weight,
    ) {
        model.apply_skeleton(&skeleton);
    }
}

/// `reach_with_other_hand` on `skeleton`, drawn at `root` (the model's
/// root transform), with a usable grip. Whether it moved the other hand.
fn reach(
    skeleton: &mut Skeleton,
    root: &Matrix4<f32>,
    grip: &Grip,
    hand: Hand,
    weight: f32,
) -> bool {
    let Some(other_point) = grip.other_hand else {
        return false;
    };
    if weight <= 0.0 {
        return false;
    }
    let Some(inverse_root) = root.invert() else {
        return false;
    };
    let other = hand.other();
    // Both fists, from the pose as the animation left it.
    let positions = skeleton.world_positions(root);
    let (Some(held), Some(before)) = (
        fist(skeleton, hand, &positions),
        fist(skeleton, other, &positions),
    ) else {
        return false;
    };
    // The other fist faces as the holding one does (both thumbs towards
    // the head); its palm is mirrored.
    let centre = place(grip, &held).transform_point(other_point.into());
    close_fist(
        skeleton,
        root,
        &inverse_root,
        &positions,
        other,
        &before,
        centre,
        held.thumb,
        held.knuckles,
        weight,
        false,
    )
}

/// A hand closing around a bar (a steering wheel's rim, a rail), in world
/// space: see `grasp`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grasp {
    /// Where the bar lies in the fist.
    pub centre: Point3<f32>,
    /// The bar's direction out of the fist on the thumb's side.
    pub thumb: Vector3<f32>,
    /// Which way the knuckles face (from the wrist towards the fingers);
    /// only its part square to `thumb` counts.
    pub knuckles: Vector3<f32>,
}

/// How `hand` of the character `model` (drawn at `transform`) is closed
/// as it is posed: the bar its fist would hold, as a `Grasp`. Reaching for
/// this grasp leaves the hand where it is; so a game can take a grip from
/// an animation (a clip posed with the hands on a wheel) and move it.
pub fn grasp_of(model: &Model, transform: &Matrix4<f32>, hand: Hand) -> Option<Grasp> {
    let skeleton = model.skeleton()?;
    let positions = skeleton.world_positions(&model.root_transform(transform));
    let fist = fist(&skeleton, hand, &positions)?;
    Some(Grasp {
        centre: fist.centre,
        thumb: fist.thumb,
        knuckles: fist.knuckles,
    })
}

/// `hand` of the character `model` (drawn at `transform`) reaches for the
/// bar of `grasp` and closes around it, `weight` of the way (0: as the
/// animation has it, 1: fully): the arm bends by two-bone inverse
/// kinematics (the elbow out the way the animation bends it), the hand turns so its fist faces as `grasp` says. Out of the
/// arm's reach it lets go. Run it after the animation has posed the model.
/// Whether it moved the hand.
pub fn grasp(
    model: &mut Model,
    transform: &Matrix4<f32>,
    hand: Hand,
    grasp: &Grasp,
    weight: f32,
) -> bool {
    let Some(mut skeleton) = model.skeleton() else {
        return false;
    };
    let moved = grasp_with(
        &mut skeleton,
        &model.root_transform(transform),
        hand,
        grasp,
        weight,
    );
    if moved {
        model.apply_skeleton(&skeleton);
    }
    moved
}

/// `grasp` on `skeleton`, drawn at `root` (the model's root transform).
fn grasp_with(
    skeleton: &mut Skeleton,
    root: &Matrix4<f32>,
    hand: Hand,
    grasp: &Grasp,
    weight: f32,
) -> bool {
    if !(weight > 0.0) {
        return false;
    }
    let finite = |v: Vector3<f32>| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    let thumb = grasp.thumb.normalize();
    let knuckles = grasp.knuckles - thumb * grasp.knuckles.dot(thumb);
    let knuckles = knuckles.normalize();
    if !(finite(thumb) && finite(knuckles) && finite(grasp.centre.to_vec())) {
        return false;
    }
    let Some(inverse_root) = root.invert() else {
        return false;
    };
    let positions = skeleton.world_positions(root);
    let Some(before) = fist(skeleton, hand, &positions) else {
        return false;
    };
    let Some(forearm) = skeleton.chain_ending_at(before.hand).and_then(|chain| match chain.kind {
        ChainKind::TwoBone { mid, .. } => Some(mid),
        ChainKind::Ccd { .. } => None,
    }) else {
        return false;
    };
    // How the animation holds the hand against the forearm.
    let globals = skeleton.globals();
    let held = Quaternion::from(rotation_of(&globals[forearm])).conjugate()
        * Quaternion::from(rotation_of(&globals[before.hand]));
    let moved = close_fist(
        skeleton,
        root,
        &inverse_root,
        &positions,
        hand,
        &before,
        grasp.centre,
        thumb,
        knuckles,
        weight,
        true,
    );
    if moved {
        spread_twist(skeleton, forearm, before.hand, held);
    }
    moved
}

/// Turn the forearm about itself by part of the hand's twist (about the
/// forearm, against `held`: the hand's rotation relative to the forearm as
/// the animation has it), so the wrist is not twisted alone (see
/// `WRIST_TWIST_SHARE`); the hand keeps its place and, but for twist over
/// the limits, its orientation. In skeleton space.
fn spread_twist(skeleton: &mut Skeleton, forearm: usize, hand: usize, held: Quaternion<f32>) {
    let globals = skeleton.globals();
    let elbow = Point3::from_vec(globals[forearm].w.truncate());
    let wrist = Point3::from_vec(globals[hand].w.truncate());
    let axis = wrist - elbow;
    if axis.magnitude2() < 1e-12 {
        return;
    }
    let axis = axis.normalize();
    let now = Quaternion::from(rotation_of(&globals[hand]));
    let untwisted = Quaternion::from(rotation_of(&globals[forearm])) * held;
    let mut difference = now * untwisted.conjugate();
    if difference.s < 0.0 {
        difference = -difference;
    }
    let twist = 2.0 * difference.v.dot(axis).atan2(difference.s);
    if !twist.is_finite() {
        return;
    }
    let at_wrist = (twist * WRIST_TWIST_SHARE).clamp(-WRIST_TWIST_MAX, WRIST_TWIST_MAX);
    let at_forearm = (twist - at_wrist).clamp(-FOREARM_TWIST_MAX, FOREARM_TWIST_MAX);
    let dropped = twist - at_wrist - at_forearm;
    // The forearm turns about the line to the wrist: the hand stays where
    // it is and turns along; it is turned back by that, less what is
    // dropped.
    skeleton.turn(forearm, Quaternion::from_axis_angle(axis, Rad(at_forearm)));
    skeleton.turn(hand, Quaternion::from_axis_angle(axis, Rad(-at_forearm - dropped)));
}

/// Close the fist of `hand` (`before`, as the pose has it; `positions` are
/// the pose's joints in world space) around `centre`, its thumb and
/// knuckles facing these (unit, square) directions, `weight` of the way.
/// Beyond the arm's length it lets go. Whether it moved the hand.
#[allow(clippy::too_many_arguments)]
fn close_fist(
    skeleton: &mut Skeleton,
    root: &Matrix4<f32>,
    inverse_root: &Matrix4<f32>,
    positions: &[Point3<f32>],
    hand: Hand,
    before: &Fist,
    centre: Point3<f32>,
    thumb: Vector3<f32>,
    knuckles: Vector3<f32>,
    weight: f32,
    keep_elbow: bool,
) -> bool {
    let Some(chain) = skeleton.chain_ending_at(before.hand) else {
        return false;
    };
    let ChainKind::TwoBone { upper, mid } = chain.kind else {
        return false;
    };
    let palm = palm_of(hand, thumb, knuckles);
    let wanted = centre - knuckles * (before.length * ALONG_FINGERS) - palm * OUT_OF_PALM;

    // As far as the arm reaches: beyond its length the hand lets go.
    let arm = (positions[mid] - positions[upper]).magnitude()
        + (positions[chain.end] - positions[mid]).magnitude();
    let over = (wanted - positions[upper]).magnitude() - arm;
    let reach = (1.0 - over.max(0.0) / (arm * LET_GO_OVER)).clamp(0.0, 1.0);
    let weight = weight.min(1.0) * reach;
    if weight <= 0.0 {
        return false;
    }
    let wrist = positions[chain.end] + (wanted - positions[chain.end]) * weight;
    if !(wrist.x.is_finite() && wrist.y.is_finite() && wrist.z.is_finite()) {
        return false;
    }
    // With `keep_elbow` the elbow bends the way the animation has it (not
    // the rig's default pole, elbows back): a clip posed with the elbows
    // out keeps them out. Only for an arm the animation bends clearly.
    let mut chain = chain;
    if keep_elbow {
        let joints = skeleton.positions();
        let (shoulder, elbow, hand_at) = (joints[upper], joints[mid], joints[chain.end]);
        let line = hand_at - shoulder;
        let out = (elbow - shoulder) - line * ((elbow - shoulder).dot(line) / line.magnitude2().max(1e-12));
        let upper_length = (elbow - shoulder).magnitude();
        if out.magnitude() > 0.15 * upper_length {
            chain.pole = out.normalize();
        }
    }
    skeleton.solve(&chain, inverse_root.transform_point(wrist));

    // Then turn the hand so its fist faces that way.
    if let Some(now) = fist(skeleton, hand, &skeleton.world_positions(root)) {
        let have = frame(now.thumb, now.knuckles);
        let want = frame(thumb, knuckles);
        let world = want * have.transpose();
        // Into skeleton space: the root's rotation undone around it.
        let root_rotation = rotation_of(root);
        let local = Quaternion::from(root_rotation.transpose() * world * root_rotation);
        let turn = Quaternion::new(1.0, 0.0, 0.0, 0.0).slerp(local, weight);
        skeleton.turn(now.hand, turn);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::ik::{IkChain, Joint};

    fn joint(name: &str, parent: Option<usize>, offset: [f32; 3], turn: f32) -> Joint {
        Joint {
            name: format!("mixamorig:{name}"),
            parent,
            local: Matrix4::from_translation(offset.into())
                * Matrix4::from(Quaternion::from_axis_angle(
                    Vector3::new(0.2, 1.0, 0.4).normalize(),
                    Rad(turn),
                )),
            skinned: true,
        }
    }

    /// A Mixamo-like rig: spine, head, arms with five fingers of three
    /// joints, legs with toes; slightly turned joints, varied by `seed`.
    fn humanoid(seed: f32) -> Skeleton {
        let mut joints = Vec::new();
        let mut add = |name: &str, parent: Option<usize>, offset: [f32; 3]| {
            let turn = ((joints.len() as f32 + 1.0) * 0.37 + seed).sin() * 0.15;
            joints.push(joint(name, parent, offset, turn));
            joints.len() - 1
        };
        let hips = add("Hips", None, [0.0, 1.0, 0.0]);
        let spine = add("Spine", Some(hips), [0.0, 0.1, 0.0]);
        let chest = add("Spine1", Some(spine), [0.0, 0.12, 0.0]);
        let neck = add("Neck", Some(chest), [0.0, 0.2, 0.0]);
        add("Head", Some(neck), [0.0, 0.1, 0.0]);
        for (side, x) in [("Left", 1.0), ("Right", -1.0)] {
            let shoulder = add(
                &format!("{side}Shoulder"),
                Some(chest),
                [0.06 * x, 0.15, 0.0],
            );
            let arm = add(&format!("{side}Arm"), Some(shoulder), [0.12 * x, 0.0, 0.0]);
            let fore = add(&format!("{side}ForeArm"), Some(arm), [0.27 * x, 0.0, 0.0]);
            let hand = add(&format!("{side}Hand"), Some(fore), [0.25 * x, 0.0, 0.0]);
            for (finger, z) in [
                ("Thumb", 0.04),
                ("Index", 0.025),
                ("Middle", 0.0),
                ("Ring", -0.02),
                ("Pinky", -0.04),
            ] {
                let mut parent = hand;
                for n in 1..=3 {
                    let offset = if n == 1 {
                        [0.09 * x, 0.0, z]
                    } else {
                        [0.03 * x, 0.0, 0.0]
                    };
                    parent = add(&format!("{side}Hand{finger}{n}"), Some(parent), offset);
                }
            }
            let up = add(&format!("{side}UpLeg"), Some(hips), [0.09 * x, -0.05, 0.0]);
            let leg = add(&format!("{side}Leg"), Some(up), [0.0, -0.42, 0.0]);
            let foot = add(&format!("{side}Foot"), Some(leg), [0.0, -0.42, 0.0]);
            let toe = add(&format!("{side}ToeBase"), Some(foot), [0.0, -0.05, 0.12]);
            add(&format!("{side}Toe_End"), Some(toe), [0.0, 0.0, 0.05]);
        }
        Skeleton::new(joints)
    }

    /// How a fist was found before positions were shared.
    fn fist_from_root(skeleton: &Skeleton, hand: Hand, root: &Matrix4<f32>) -> Option<Fist> {
        fist(skeleton, hand, &skeleton.world_positions(root))
    }

    /// `reach` as it was before positions were shared and the chain was
    /// found by itself: the reference the new one must match bit for bit.
    fn reach_before(
        skeleton: &mut Skeleton,
        root: &Matrix4<f32>,
        grip: &Grip,
        hand: Hand,
        weight: f32,
    ) -> bool {
        let Some(other_point) = grip.other_hand else {
            return false;
        };
        if weight <= 0.0 {
            return false;
        }
        let Some(inverse_root) = root.invert() else {
            return false;
        };
        let other = hand.other();
        let (Some(held), Some(before)) = (
            fist_from_root(skeleton, hand, root),
            fist_from_root(skeleton, other, root),
        ) else {
            return false;
        };
        let Some(chain) = skeleton
            .default_chains()
            .into_iter()
            .find(|chain| chain.end == before.hand)
        else {
            return false;
        };
        let ChainKind::TwoBone { upper, mid } = chain.kind else {
            return false;
        };
        let (thumb, knuckles) = (held.thumb, held.knuckles);
        let palm = palm_of(other, thumb, knuckles);
        let centre = place(grip, &held).transform_point(other_point.into());
        let wanted = centre - knuckles * (before.length * ALONG_FINGERS) - palm * OUT_OF_PALM;
        let positions = skeleton.world_positions(root);
        let arm = (positions[mid] - positions[upper]).magnitude()
            + (positions[chain.end] - positions[mid]).magnitude();
        let over = (wanted - positions[upper]).magnitude() - arm;
        let reach = (1.0 - over.max(0.0) / (arm * LET_GO_OVER)).clamp(0.0, 1.0);
        let weight = weight.min(1.0) * reach;
        if weight <= 0.0 {
            return false;
        }
        let wrist = positions[chain.end] + (wanted - positions[chain.end]) * weight;
        if !(wrist.x.is_finite() && wrist.y.is_finite() && wrist.z.is_finite()) {
            return false;
        }
        skeleton.solve(&chain, inverse_root.transform_point(wrist));
        if let Some(now) = fist_from_root(skeleton, other, root) {
            let have = frame(now.thumb, now.knuckles);
            let want = frame(thumb, knuckles);
            let world = want * have.transpose();
            let root_rotation = rotation_of(root);
            let local = Quaternion::from(root_rotation.transpose() * world * root_rotation);
            let turn = Quaternion::new(1.0, 0.0, 0.0, 0.0).slerp(local, weight);
            skeleton.turn(now.hand, turn);
        }
        true
    }

    fn locals_bits(skeleton: &Skeleton) -> Vec<[u32; 16]> {
        skeleton.locals().iter().map(matrix_bits).collect()
    }

    fn chain_bits(chain: &IkChain) -> (String, usize, [u32; 3], Vec<usize>) {
        let joints = match &chain.kind {
            ChainKind::TwoBone { upper, mid } => vec![*upper, *mid],
            ChainKind::Ccd { joints } => joints.clone(),
        };
        let pole = [chain.pole.x, chain.pole.y, chain.pole.z].map(f32::to_bits);
        (chain.label.clone(), chain.end, pole, joints)
    }

    #[test]
    fn the_other_hand_reaches_bit_for_bit_as_before() {
        // The rig is in metres.
        let roots = [
            Matrix4::identity(),
            Matrix4::from_translation(Vector3::new(3.0, 0.5, -2.0))
                * Matrix4::from(Quaternion::from_axis_angle(Vector3::unit_y(), Rad(2.1)))
                * Matrix4::from_scale(1.01),
        ];
        let mut grips = Vec::new();
        // The rig stands in a T-pose: the second grip lies between the
        // hands (against the knuckles, the item's -y), near enough for the
        // other arm, or at the edge of its reach.
        for (point, other) in [
            ([0.0, 0.0, 0.0], [0.0, -0.9, 0.0]),
            ([0.1, 0.02, 0.0], [0.15, -0.8, 0.1]),
            ([0.0, 0.0, 0.0], [0.0, -0.5, 0.2]),
            // Far out of reach: lets go.
            ([0.0, 0.0, 0.0], [90.0, 0.0, 0.0]),
        ] {
            let mut grip = Grip::at(point);
            grip.other_hand = Some(other);
            grip.roll = 20.0;
            grips.push(grip);
        }
        let mut moved = 0;
        for seed in 0..4 {
            for root in &roots {
                for grip in &grips {
                    for hand in [Hand::Right, Hand::Left] {
                        for weight in [0.0, 0.4, 1.0] {
                            let mut before = humanoid(seed as f32);
                            let mut now = before.clone();
                            let a = reach_before(&mut before, root, grip, hand, weight);
                            let b = reach(&mut now, root, grip, hand, weight);
                            assert_eq!(a, b);
                            assert_eq!(locals_bits(&before), locals_bits(&now));
                            moved += a as usize;
                        }
                    }
                }
            }
        }
        // The comparison covered hands that reached, not only early outs.
        assert!(moved >= 16, "{moved}");
    }

    #[test]
    fn a_hand_closes_around_a_bar_where_and_as_it_is_told() {
        let roots = [
            Matrix4::identity(),
            Matrix4::from_translation(Vector3::new(3.0, 0.5, -2.0))
                * Matrix4::from(Quaternion::from_axis_angle(Vector3::unit_y(), Rad(2.1))),
        ];
        for root in &roots {
            for (hand, x) in [(Hand::Left, 1.0), (Hand::Right, -1.0)] {
                // In front of the shoulder, within the arm's reach (the rig
                // faces +z): a wheel's rim at shoulder height.
                let local_centre = Point3::new(0.25 * x, 1.25, 0.45);
                let thumb = root.transform_vector(Vector3::new(0.0, 1.0, 0.0));
                let knuckles = root.transform_vector(Vector3::new(0.3 * x, 0.2, 1.0));
                let target = Grasp {
                    centre: root.transform_point(local_centre),
                    thumb,
                    knuckles,
                };
                let mut skeleton = humanoid(0.5);
                assert!(grasp_with(&mut skeleton, root, hand, &target, 1.0));
                let now = fist(&skeleton, hand, &skeleton.world_positions(root)).unwrap();
                assert!(
                    (now.centre - target.centre).magnitude() < 0.01,
                    "{hand:?}: fist at {:?}, wanted {:?}",
                    now.centre,
                    target.centre
                );
                assert!(now.thumb.dot(thumb.normalize()) > 0.99, "{hand:?} thumb {:?}", now.thumb);
                let square = (knuckles - thumb * knuckles.dot(thumb)).normalize();
                assert!(now.knuckles.dot(square) > 0.99, "{hand:?} knuckles {:?}", now.knuckles);

                // Half the weight: the wrist half of the way there.
                let mut half = humanoid(0.5);
                let before = fist(&half, hand, &half.world_positions(root)).unwrap();
                assert!(grasp_with(&mut half, root, hand, &target, 0.5));
                let wrist = half.world_positions(root)[before.hand];
                let full = skeleton.world_positions(root)[before.hand];
                let from = humanoid(0.5).world_positions(root)[before.hand];
                assert!((wrist - (from + (full - from) * 0.5)).magnitude() < 0.01);
            }
        }
    }

    #[test]
    fn grasping_as_the_hand_is_closed_leaves_it_there() {
        let root = Matrix4::from_translation(Vector3::new(3.0, 0.5, -2.0))
            * Matrix4::from(Quaternion::from_axis_angle(Vector3::unit_y(), Rad(2.1)));
        for (hand, side) in [(Hand::Left, "Left"), (Hand::Right, "Right")] {
            // The elbow bent well out and forward, as a clip at a wheel has it.
            let mut skeleton = humanoid(0.5);
            let fore = skeleton.find(&format!("mixamorig:{side}ForeArm")).unwrap();
            skeleton.turn(fore, Quaternion::from_axis_angle(Vector3::new(0.3, 1.0, 0.2).normalize(), Rad(1.1)));
            let start = skeleton.world_positions(&root);
            let now = fist(&skeleton, hand, &start).unwrap();
            let target = Grasp { centre: now.centre, thumb: now.thumb, knuckles: now.knuckles };
            grasp_with(&mut skeleton, &root, hand, &target, 1.0);
            for (a, b) in start.iter().zip(skeleton.world_positions(&root)) {
                assert!((a - b).magnitude() < 1e-3, "{hand:?}: {a:?} moved to {b:?}");
            }
        }
    }

    /// The twist (radians) of joint `child` about the line from `parent`
    /// to it, in `after` against `before`, relative to `parent`.
    fn twist_between(before: &Skeleton, after: &Skeleton, parent: usize, child: usize) -> f32 {
        let q = |s: &Skeleton, j: usize| Quaternion::from(rotation_of(&s.globals()[j]));
        let held = q(before, parent).conjugate() * q(before, child);
        let p = after.positions();
        let axis = (p[child] - p[parent]).normalize();
        let mut d = q(after, child) * (q(after, parent) * held).conjugate();
        if d.s < 0.0 {
            d = -d;
        }
        2.0 * d.v.dot(axis).atan2(d.s)
    }

    #[test]
    fn a_hand_turned_far_about_the_forearm_is_not_twisted_at_the_wrist_alone() {
        let root = Matrix4::identity();
        let before = humanoid(0.3);
        let positions = before.world_positions(&root);
        let [wrist, ..] = hand_joints(&before, Hand::Left).unwrap();
        let ChainKind::TwoBone { upper, mid } = before.chain_ending_at(wrist).unwrap().kind else {
            panic!("an arm");
        };
        let fist_now = fist(&before, Hand::Left, &positions).unwrap();
        // Where the fist is, its knuckles as they are, but rolled 150°
        // about the forearm.
        let axis = (positions[wrist] - positions[mid]).normalize();
        let roll = Quaternion::from_axis_angle(axis, Rad(150f32.to_radians()));
        let target = Grasp {
            centre: fist_now.centre,
            thumb: roll * fist_now.thumb,
            knuckles: roll * fist_now.knuckles,
        };
        let mut after = before.clone();
        assert!(grasp_with(&mut after, &root, Hand::Left, &target, 1.0));
        let at_wrist = twist_between(&before, &after, mid, wrist);
        let at_forearm = twist_between(&before, &after, upper, mid);
        assert!(at_wrist.abs() <= WRIST_TWIST_MAX + 1e-3, "wrist {}", at_wrist.to_degrees());
        assert!(at_forearm.to_degrees().abs() > 60.0, "forearm {}", at_forearm.to_degrees());
        // The hand stays where it holds.
        let now = fist(&after, Hand::Left, &after.world_positions(&root)).unwrap();
        assert!((now.centre - target.centre).magnitude() < 0.03, "{:?}", now.centre);
    }

    #[test]
    fn a_hand_does_not_grasp_beyond_its_reach_nor_without_directions() {
        let root = Matrix4::identity();
        let far = Grasp {
            centre: Point3::new(0.3, 1.2, 5.0),
            thumb: Vector3::unit_y(),
            knuckles: Vector3::unit_z(),
        };
        let mut skeleton = humanoid(0.0);
        let before = locals_bits(&skeleton);
        assert!(!grasp_with(&mut skeleton, &root, Hand::Left, &far, 1.0));
        let flat = Grasp {
            centre: Point3::new(0.25, 1.25, 0.45),
            thumb: Vector3::unit_y(),
            knuckles: Vector3::unit_y(),
        };
        assert!(!grasp_with(&mut skeleton, &root, Hand::Left, &flat, 1.0));
        assert!(!grasp_with(&mut skeleton, &root, Hand::Left, &Grasp { knuckles: Vector3::unit_z(), ..flat }, 0.0));
        assert_eq!(locals_bits(&skeleton), before);
    }

    #[test]
    fn a_fist_from_shared_positions_places_the_item_as_before() {
        let skeleton = humanoid(1.0);
        let root = Matrix4::from_translation(Vector3::new(1.0, 0.0, 2.0));
        let grip = Grip::at([0.1, 0.0, 0.0]);
        for hand in [Hand::Right, Hand::Left] {
            let a = place(&grip, &fist_from_root(&skeleton, hand, &root).unwrap());
            let positions = skeleton.world_positions(&root);
            let b = place(&grip, &fist(&skeleton, hand, &positions).unwrap());
            assert_eq!(matrix_bits(&a), matrix_bits(&b));
        }
    }

    #[test]
    fn the_chain_ending_at_a_joint_is_the_default_chain_ending_there() {
        let skeleton = humanoid(2.0);
        let chains = skeleton.default_chains();
        assert!(chains.len() >= 5);
        for end in 0..skeleton.joints.len() {
            let wanted = chains.iter().find(|chain| chain.end == end).map(chain_bits);
            assert_eq!(
                skeleton.chain_ending_at(end).as_ref().map(chain_bits),
                wanted
            );
        }
    }

    #[test]
    fn a_placement_is_worked_out_again_only_when_what_it_follows_from_changes() {
        let placement = HeldPlacement::default();
        let grip = Grip::at([0.0; 3]).bits();
        let root = matrix_bits(&Matrix4::identity());
        let calls = Cell::new(0);
        let place = |value: f32| {
            calls.set(calls.get() + 1);
            Some(Matrix4::from_scale(value))
        };
        let key = (1, root, grip, Hand::Right);
        assert_eq!(
            placement.get_or_place(key, || place(1.0)),
            Some(Matrix4::from_scale(1.0))
        );
        // The other passes of the frame.
        for _ in 0..8 {
            assert_eq!(
                placement.get_or_place(key, || place(9.0)),
                Some(Matrix4::from_scale(1.0))
            );
        }
        assert_eq!(calls.get(), 1);
        // A new pose, a moved character, another grip or hand.
        let moved = matrix_bits(&Matrix4::from_translation(Vector3::unit_x()));
        let mut other_grip = Grip::at([0.0; 3]);
        other_grip.roll = -0.0;
        for (i, key) in [
            (2, root, grip, Hand::Right),
            (2, moved, grip, Hand::Right),
            (2, moved, other_grip.bits(), Hand::Right),
            (2, moved, other_grip.bits(), Hand::Left),
        ]
        .into_iter()
        .enumerate()
        {
            let value = i as f32 + 2.0;
            assert_eq!(
                placement.get_or_place(key, || place(value)),
                Some(Matrix4::from_scale(value))
            );
        }
        assert_eq!(calls.get(), 5);
    }

    #[test]
    fn frames_are_square_and_right_handed() {
        let f = frame(Vector3::new(2.0, 0.0, 0.0), Vector3::new(1.0, 1.0, 0.0));
        assert!((f.x - Vector3::unit_x()).magnitude() < 1e-6);
        assert!((f.y - Vector3::unit_y()).magnitude() < 1e-6);
        assert!((f.z - Vector3::unit_z()).magnitude() < 1e-6);
    }

    #[test]
    fn a_grip_puts_its_point_in_the_fist_facing_its_way() {
        let fist = Fist {
            hand: 0,
            centre: Point3::new(1.0, 2.0, 3.0),
            thumb: Vector3::unit_y(),
            knuckles: Vector3::unit_z(),
            length: 0.1,
        };
        // Head along -x, edge along +z.
        let grip = Grip {
            point: [0.2, 0.0, 0.0],
            along: [-1.0, 0.0, 0.0],
            facing: [0.0, 0.0, 1.0],
            roll: 0.0,
            other_hand: None,
        };
        let placed = place(&grip, &fist);
        let point = placed.transform_point(Point3::new(0.2, 0.0, 0.0));
        assert!((point - fist.centre).magnitude() < 1e-5);
        let head = placed.transform_vector(Vector3::new(-1.0, 0.0, 0.0));
        let edge = placed.transform_vector(Vector3::unit_z());
        assert!((head - Vector3::unit_y()).magnitude() < 1e-5);
        assert!((edge - Vector3::unit_z()).magnitude() < 1e-5);
    }

    #[test]
    fn grips_without_a_frame_fall_back_to_one_at_their_point() {
        let mut grip = Grip::at([0.1, 0.2, 0.3]);
        grip.facing = [2.0, 0.0, 0.0]; // along `along`
        grip.other_hand = Some([0.5, 0.0, 0.0]);
        let usable = grip.usable();
        assert_eq!(usable.along, [1.0, 0.0, 0.0]);
        assert_eq!(usable.facing, [0.0, 1.0, 0.0]);
        assert_eq!(usable.point, [0.1, 0.2, 0.3]);
        assert_eq!(usable.other_hand, Some([0.5, 0.0, 0.0]));
        grip.along = [0.0; 3];
        grip.facing = [0.0, 0.0, 1.0];
        assert_eq!(grip.usable().along, [1.0, 0.0, 0.0]);
        let fine = Grip::at([0.0; 3]);
        assert_eq!(fine.usable(), fine);

        let fist = Fist {
            hand: 0,
            centre: Point3::new(1.0, 2.0, 3.0),
            thumb: Vector3::unit_y(),
            knuckles: Vector3::unit_z(),
            length: 0.1,
        };
        assert!(is_finite_matrix(&place(&grip.usable(), &fist)));
    }

    #[test]
    fn rolling_turns_the_edge_about_the_handle() {
        let mut grip = Grip::at([0.0; 3]);
        grip.roll = 90.0;
        let facing = grip.rolled_facing();
        assert!((facing - Vector3::unit_z()).magnitude() < 1e-5);
    }

    #[test]
    fn grips_round_trip_as_ron_and_leave_defaults_out() {
        let grip = Grip::at([0.1, 0.0, 0.0]);
        let text = ron::to_string(&grip).unwrap();
        assert!(!text.contains("roll") && !text.contains("other_hand"));
        assert_eq!(ron::from_str::<Grip>(&text).unwrap(), grip);
    }
}
