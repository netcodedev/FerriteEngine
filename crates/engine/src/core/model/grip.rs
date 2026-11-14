//! How a model is held in a character's hand: a grip, set on the model's
//! asset (`ModelSettings::grip`), and the maths that puts the model into a
//! hand of a posed skeleton. Used by `HeldItem` in the game and by the
//! editor's preview of a held model.
//!
//! The hand is found from the bone names (`RightHand`, its fingers), so it
//! works for any rig named like Mixamo's; the fist is measured from the
//! finger joints, so a grip is given in the item's terms only.

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

fn fist(skeleton: &Skeleton, hand: Hand, root: &Matrix4<f32>) -> Option<Fist> {
    let [wrist, middle, index, pinky] = hand_joints(skeleton, hand)?;
    let positions = skeleton.world_positions(root);
    let knuckles = positions[middle] - positions[wrist];
    let length = knuckles.magnitude();
    let knuckles = knuckles.normalize();
    let across = positions[index] - positions[pinky];
    let thumb = (across - knuckles * across.dot(knuckles)).normalize();
    let palm = palm_of(hand, thumb, knuckles);
    Some(Fist {
        hand: wrist,
        centre: positions[wrist] + knuckles * (length * ALONG_FINGERS) + palm * OUT_OF_PALM,
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
    let fist = fist(&skeleton, hand, &model.root_transform(transform))?;
    Some(place(grip, &fist))
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
    let Some(other_point) = grip.other_hand else {
        return;
    };
    if weight <= 0.0 {
        return;
    }
    let Some(mut skeleton) = model.skeleton() else {
        return;
    };
    let root = model.root_transform(transform);
    let Some(inverse_root) = root.invert() else {
        return;
    };
    let other = hand.other();
    let (Some(held), Some(before)) = (fist(&skeleton, hand, &root), fist(&skeleton, other, &root))
    else {
        return;
    };
    let Some(chain) = skeleton
        .default_chains()
        .into_iter()
        .find(|chain| chain.end == before.hand)
    else {
        return;
    };
    let ChainKind::TwoBone { upper, mid } = chain.kind else {
        return;
    };
    // The other fist faces as the holding one does (both thumbs towards
    // the head); its palm is mirrored.
    let (thumb, knuckles) = (held.thumb, held.knuckles);
    let palm = palm_of(other, thumb, knuckles);
    let centre = place(grip, &held).transform_point(other_point.into());
    let wanted = centre - knuckles * (before.length * ALONG_FINGERS) - palm * OUT_OF_PALM;

    // As far as the arm reaches: beyond its length the hand lets go.
    let positions = skeleton.world_positions(&root);
    let arm = (positions[mid] - positions[upper]).magnitude()
        + (positions[chain.end] - positions[mid]).magnitude();
    let over = (wanted - positions[upper]).magnitude() - arm;
    let reach = (1.0 - over.max(0.0) / (arm * LET_GO_OVER)).clamp(0.0, 1.0);
    let weight = weight.min(1.0) * reach;
    if weight <= 0.0 {
        return;
    }
    let wrist = positions[chain.end] + (wanted - positions[chain.end]) * weight;
    skeleton.solve(&chain, inverse_root.transform_point(wrist));

    // Then turn the hand so its fist faces that way.
    if let Some(now) = fist(&skeleton, other, &root) {
        let have = frame(now.thumb, now.knuckles);
        let want = frame(thumb, knuckles);
        let world = want * have.transpose();
        // Into skeleton space: the root's rotation undone around it.
        let root_rotation = rotation_of(&root);
        let local = Quaternion::from(root_rotation.transpose() * world * root_rotation);
        let turn = Quaternion::new(1.0, 0.0, 0.0, 0.0).slerp(local, weight);
        skeleton.turn(now.hand, turn);
    }
    model.apply_skeleton(&skeleton);
}

#[cfg(test)]
mod tests {
    use super::*;

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
