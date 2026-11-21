//! Anchors: named frames on a model asset (`ModelSettings::anchors`) where
//! another model goes, such as the driver's seat of a truck or a point on
//! a character's back. An anchor is in the model's own space or relative
//! to one of its bones, so it moves with the bone. Two models meet by
//! putting an anchor of one on an anchor of the other (`place_on`).
//!
//! The clip editor shows references against the animated model with them
//! (see `docs/anchors.md`); the game reads the same anchors.

use cgmath::{
    Deg, EuclideanSpace, InnerSpace, Matrix3, Matrix4, Point3, SquareMatrix, Vector3,
};
use serde::{Deserialize, Serialize};

use crate::core::assets::{self, AssetId};

use super::{
    grip::Hand,
    ik::{base_name, Skeleton},
    Model,
};

/// A named frame on a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub name: String,
    /// The bone it moves with (its full name); in the model's own space
    /// without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    /// Metres, as the model is drawn (after its import scale).
    pub position: [f32; 3],
    /// Degrees about x, then y, then z (of the model or the bone).
    #[serde(default)]
    pub rotation: [f32; 3],
}

impl Anchor {
    pub fn new(name: &str, bone: Option<String>, position: [f32; 3]) -> Self {
        Self {
            name: name.to_string(),
            bone,
            position,
            rotation: [0.0; 3],
        }
    }

    /// The anchor's frame in the space of the model (or its bone).
    pub fn local(&self) -> Matrix4<f32> {
        let finite = |v: [f32; 3]| v.map(|x| if x.is_finite() { x } else { 0.0 });
        Matrix4::from_translation(finite(self.position).into())
            * Matrix4::from(rotation_from_euler(finite(self.rotation)))
    }

    /// Take the turn of `rotation` (a rotation matrix) as the anchor's.
    pub fn set_rotation(&mut self, rotation: &Matrix3<f32>) {
        self.rotation = euler_from_rotation(rotation);
    }
}

/// The rotation of `degrees` about x, then y, then z.
pub fn rotation_from_euler(degrees: [f32; 3]) -> Matrix3<f32> {
    let [x, y, z] = degrees;
    Matrix3::from_angle_z(Deg(z)) * Matrix3::from_angle_y(Deg(y)) * Matrix3::from_angle_x(Deg(x))
}

/// Degrees about x, then y, then z of a rotation matrix (the inverse of
/// `rotation_from_euler`).
pub fn euler_from_rotation(m: &Matrix3<f32>) -> [f32; 3] {
    // m[column][row]; R = Rz * Ry * Rx.
    let sin_y = (-m[0][2]).clamp(-1.0, 1.0);
    let (x, y, z) = if sin_y.abs() < 0.9999 {
        (m[1][2].atan2(m[2][2]), sin_y.asin(), m[0][1].atan2(m[0][0]))
    } else {
        // Gimbal lock: the turn about x and z is the same; put it in z.
        ((0.0f32), sin_y.asin(), (-m[1][0]).atan2(m[1][1]))
    };
    [x, y, z].map(|a| {
        let d = a.to_degrees();
        // Small noise around a whole number from the round trip.
        let r = (d * 1000.0).round() / 1000.0;
        if r == -0.0 { 0.0 } else { r }
    })
}

/// `m` without scale: its axes of unit length (and square to each other),
/// its translation kept.
pub fn unscaled(m: &Matrix4<f32>) -> Matrix4<f32> {
    let x = m.x.truncate();
    let y = m.y.truncate();
    let x = if x.magnitude2() > 1e-20 { x.normalize() } else { Vector3::unit_x() };
    let z = x.cross(y);
    let z = if z.magnitude2() > 1e-20 { z.normalize() } else { m.z.truncate().normalize() };
    let y = z.cross(x);
    let mut out = Matrix4::from(Matrix3::from_cols(x, y, z));
    out.w = m.w;
    out
}

/// The joint of `skeleton` called `bone`: by its full name, else by the
/// name without namespace (rigs exported with another prefix).
pub fn find_bone(skeleton: &Skeleton, bone: &str) -> Option<usize> {
    skeleton.find(bone).or_else(|| {
        let wanted = base_name(bone);
        skeleton
            .joints
            .iter()
            .position(|joint| base_name(&joint.name) == wanted)
    })
}

/// World frame of `anchor` on a model whose bones are posed as `skeleton`
/// and whose bones' space is `root` (`Model::root_transform`), drawn at
/// `transform`. `None` if the anchor's bone is not there.
pub fn anchor_frame_in(
    skeleton: Option<&Skeleton>,
    root: &Matrix4<f32>,
    transform: &Matrix4<f32>,
    anchor: &Anchor,
) -> Option<Matrix4<f32>> {
    let base = match &anchor.bone {
        None => *transform,
        Some(bone) => {
            let skeleton = skeleton?;
            let joint = find_bone(skeleton, bone)?;
            unscaled(&(root * skeleton.globals()[joint]))
        }
    };
    let frame = base * anchor.local();
    frame_is_finite(&frame).then_some(frame)
}

/// World frame of `anchor` on `model` as it is posed now, drawn at
/// `transform`.
pub fn anchor_frame(model: &Model, transform: &Matrix4<f32>, anchor: &Anchor) -> Option<Matrix4<f32>> {
    let skeleton = if anchor.bone.is_some() { model.skeleton() } else { None };
    anchor_frame_in(skeleton.as_ref(), &model.root_transform(transform), transform, anchor)
}

/// The transform to draw a model at so that its `anchor` (taken in the
/// model's own space, or its rest pose for one on a bone: `anchor_local`)
/// lies on `frame`.
pub fn place_on(frame: &Matrix4<f32>, anchor_local: &Matrix4<f32>) -> Option<Matrix4<f32>> {
    let placed = frame * anchor_local.invert()?;
    frame_is_finite(&placed).then_some(placed)
}

/// Where `anchor` is in the model's own space (drawn at the identity),
/// with its bones as `skeleton` has them (their rest pose, for a model
/// that is placed by it); `root` is `Model::root_transform` of the
/// identity.
pub fn anchor_in_model(
    skeleton: Option<&Skeleton>,
    root: &Matrix4<f32>,
    anchor: Option<&Anchor>,
) -> Option<Matrix4<f32>> {
    match anchor {
        None => Some(Matrix4::identity()),
        Some(anchor) => anchor_frame_in(skeleton, root, &Matrix4::identity(), anchor),
    }
}

/// The anchors saved on a model asset.
pub fn anchors_of(asset: AssetId) -> Vec<Anchor> {
    assets::database()
        .get(asset)
        .and_then(|entry| Some(entry.meta.as_model()?.anchors.clone()))
        .unwrap_or_default()
}

/// The anchor called `name` on a model asset.
pub fn find_anchor(asset: AssetId, name: &str) -> Option<Anchor> {
    anchors_of(asset).into_iter().find(|anchor| anchor.name == name)
}

/// A free name for a new anchor among `anchors`: `base`, `base 2`, ...
pub fn free_name(anchors: &[Anchor], base: &str) -> String {
    let taken = |name: &str| anchors.iter().any(|a| a.name == name);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| base.to_string())
}

/// How a reference shown with a clip sits against the clip's model (see
/// `docs/anchors.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceLink {
    /// In a hand of the clip's model, as the reference's grip says (the
    /// way `HeldItem` holds it).
    Held(Hand),
    /// The reference's anchor on the model's anchor as the model is
    /// posed: it moves with the bone the anchor is on.
    Follows,
    /// The reference's anchor on the model's anchor in the model's rest
    /// pose: the reference stands still while the clip moves the body.
    Carries,
}

impl ReferenceLink {
    pub const ALL: [ReferenceLink; 4] = [
        ReferenceLink::Held(Hand::Right),
        ReferenceLink::Held(Hand::Left),
        ReferenceLink::Follows,
        ReferenceLink::Carries,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ReferenceLink::Held(Hand::Right) => "In the right hand (grip)",
            ReferenceLink::Held(Hand::Left) => "In the left hand (grip)",
            ReferenceLink::Follows => "Follows the model",
            ReferenceLink::Carries => "Carries the model",
        }
    }

    /// Whether the anchors say where it goes (not the grip).
    pub fn uses_anchors(self) -> bool {
        !matches!(self, ReferenceLink::Held(_))
    }
}

/// Columns of `m` as world vectors: x, y and z of an anchor frame, and its
/// origin.
pub fn frame_axes(m: &Matrix4<f32>) -> (Point3<f32>, [Vector3<f32>; 3]) {
    (
        Point3::from_vec(m.w.truncate()),
        [m.x.truncate(), m.y.truncate(), m.z.truncate()],
    )
}

/// The rotation part of an unscaled frame.
pub fn rotation_of(m: &Matrix4<f32>) -> Matrix3<f32> {
    Matrix3::from_cols(m.x.truncate(), m.y.truncate(), m.z.truncate())
}

/// The anchor's frame, relative to `base` (the model's transform or its
/// bone's unscaled frame), for a world `frame`: what to save as the
/// anchor's position and rotation.
pub fn relative_to(base: &Matrix4<f32>, frame: &Matrix4<f32>) -> Option<([f32; 3], Matrix3<f32>)> {
    let local = base.invert()? * frame;
    let rotation = rotation_of(&unscaled(&local));
    let p = local.w.truncate();
    let p = [p.x, p.y, p.z];
    p.iter().all(|v| v.is_finite()).then_some((p, rotation))
}

/// An anchor whose world `frame` (relative to `base`) is moved by the
/// world transform `delta`: its new position and rotation.
pub fn moved(base: &Matrix4<f32>, frame: &Matrix4<f32>, delta: &Matrix4<f32>) -> Option<([f32; 3], Matrix3<f32>)> {
    relative_to(base, &(delta * frame))
}

/// The anchor a model is placed by (it lies on the world `frame`; `local`
/// is it in the model's own space, `base` what it is relative to there),
/// changed so that the model moves by the world transform `delta` while
/// the anchor stays on `frame`: dragging a seat's anchor moves the truck.
pub fn moved_placing(
    frame: &Matrix4<f32>,
    local: &Matrix4<f32>,
    base: &Matrix4<f32>,
    delta: &Matrix4<f32>,
) -> Option<([f32; 3], Matrix3<f32>)> {
    // Placed at P = frame * local^-1; wanted delta * P = frame * new^-1.
    let new = local * frame.invert()? * delta.invert()? * frame;
    relative_to(base, &new)
}

/// The frame an anchor is relative to: `transform` for one in model
/// space, the unscaled world frame of its bone otherwise.
pub fn anchor_base(
    skeleton: Option<&Skeleton>,
    root: &Matrix4<f32>,
    transform: &Matrix4<f32>,
    bone: Option<&str>,
) -> Option<Matrix4<f32>> {
    match bone {
        None => Some(*transform),
        Some(bone) => {
            let skeleton = skeleton?;
            let joint = find_bone(skeleton, bone)?;
            Some(unscaled(&(root * skeleton.globals()[joint])))
        }
    }
}

fn frame_is_finite(m: &Matrix4<f32>) -> bool {
    let entries: &[f32; 16] = m.as_ref();
    entries.iter().all(|v| v.is_finite()) && m.determinant().abs() > 1e-12
}

#[cfg(test)]
mod tests {
    use cgmath::{Quaternion, Rad, Rotation3, Transform};

    use super::*;
    use crate::core::model::ik::Joint;

    fn close(a: &Matrix4<f32>, b: &Matrix4<f32>) -> bool {
        (0..4).all(|c| (0..4).all(|r| (a[c][r] - b[c][r]).abs() < 1e-4))
    }

    fn near(a: Point3<f32>, b: Point3<f32>) -> bool {
        (a - b).magnitude() < 1e-4
    }

    /// A Mixamo-like rig at 0.01: hips with a spine 30 cm (3000 units...
    /// in centimetres: 30) above, turnable.
    fn rig() -> (Skeleton, Matrix4<f32>) {
        let joints = vec![
            Joint {
                name: "mixamorig1:Hips".into(),
                parent: None,
                local: Matrix4::from_translation(Vector3::new(0.0, 100.0, 0.0)),
                skinned: true,
            },
            Joint {
                name: "mixamorig1:Spine".into(),
                parent: Some(0),
                local: Matrix4::from_translation(Vector3::new(0.0, 30.0, 0.0)),
                skinned: true,
            },
        ];
        (Skeleton::new(joints), Matrix4::from_scale(0.01))
    }

    #[test]
    fn euler_angles_survive_the_round_trip() {
        for degrees in [[0.0, 0.0, 0.0], [10.0, 20.0, 30.0], [-45.0, 80.0, 170.0], [90.0, 0.0, -90.0]] {
            let back = euler_from_rotation(&rotation_from_euler(degrees));
            let again = rotation_from_euler(back);
            let m = rotation_from_euler(degrees);
            for c in 0..3 {
                for r in 0..3 {
                    assert!((m[c][r] - again[c][r]).abs() < 1e-4, "{degrees:?} -> {back:?}");
                }
            }
        }
        // Gimbal lock still gives the same turn.
        let m = rotation_from_euler([30.0, 90.0, 10.0]);
        let again = rotation_from_euler(euler_from_rotation(&m));
        for c in 0..3 {
            for r in 0..3 {
                assert!((m[c][r] - again[c][r]).abs() < 1e-3);
            }
        }
    }

    // R1
    #[test]
    fn an_anchor_without_bone_is_in_the_models_space() {
        let anchor = Anchor {
            name: "seat".into(),
            bone: None,
            position: [-0.38, 0.72, 0.04],
            rotation: [0.0, 90.0, 0.0],
        };
        let transform = Matrix4::from_translation(Vector3::new(5.0, 0.0, 2.0))
            * Matrix4::from_angle_y(Deg(30.0));
        let frame = anchor_frame_in(None, &transform, &transform, &anchor).unwrap();
        assert!(close(&frame, &(transform * anchor.local())));
    }

    // R2
    #[test]
    fn an_anchor_on_a_bone_follows_the_bone_and_ignores_its_scale() {
        let (mut skeleton, root) = rig();
        let anchor = Anchor::new("back", Some("mixamorig1:Spine".into()), [0.0, 0.0, -0.15]);
        let frame = anchor_frame_in(Some(&skeleton), &root, &Matrix4::identity(), &anchor).unwrap();
        // 1.3 m up, 15 cm behind: metres, not shrunk by the rig's 0.01.
        assert!(near(Point3::from_vec(frame.w.truncate()), Point3::new(0.0, 1.3, -0.15)));
        // Turning the spine half round about y swings the point in front.
        skeleton.turn(1, Quaternion::from_angle_y(Rad(std::f32::consts::PI)));
        let frame = anchor_frame_in(Some(&skeleton), &root, &Matrix4::identity(), &anchor).unwrap();
        assert!(near(Point3::from_vec(frame.w.truncate()), Point3::new(0.0, 1.3, 0.15)));
        // A bone of another namespace is found by its plain name.
        let other = Anchor::new("back", Some("mixamorig:Spine".into()), [0.0; 3]);
        assert!(anchor_frame_in(Some(&skeleton), &root, &Matrix4::identity(), &other).is_some());
        let missing = Anchor::new("x", Some("Tail".into()), [0.0; 3]);
        assert!(anchor_frame_in(Some(&skeleton), &root, &Matrix4::identity(), &missing).is_none());
    }

    // R3
    #[test]
    fn placing_on_a_frame_puts_the_anchor_there() {
        let seat = Anchor {
            name: "seat".into(),
            bone: None,
            position: [0.3, 0.7, -0.2],
            rotation: [5.0, 40.0, -10.0],
        };
        let frame = Matrix4::from_translation(Vector3::new(1.0, 2.0, 3.0))
            * Matrix4::from(rotation_from_euler([0.0, -20.0, 15.0]));
        let placed = place_on(&frame, &seat.local()).unwrap();
        assert!(close(&(placed * seat.local()), &frame));
    }

    // R4 (the editor takes the model's anchor in the rest pose for
    // `Carries`): an anchor in the rest pose does not move with the pose.
    #[test]
    fn a_rest_pose_anchor_stays_when_the_model_is_posed() {
        let (skeleton, root) = rig();
        let mut posed = skeleton.clone();
        posed.turn(0, Quaternion::from_angle_x(Rad(0.7)));
        let anchor = Anchor::new("hips", Some("mixamorig1:Hips".into()), [0.0, 0.0, 0.0]);
        let mut rest = posed.clone();
        rest.reset();
        let at_rest = anchor_in_model(Some(&rest), &root, Some(&anchor)).unwrap();
        let before = anchor_in_model(Some(&skeleton), &root, Some(&anchor)).unwrap();
        assert!(close(&at_rest, &before));
        let now = anchor_in_model(Some(&posed), &root, Some(&anchor)).unwrap();
        assert!(!close(&now, &before));
    }

    #[test]
    fn a_frame_saved_relative_to_its_base_comes_back() {
        let (skeleton, root) = rig();
        let base = anchor_base(Some(&skeleton), &root, &Matrix4::identity(), Some("mixamorig1:Spine")).unwrap();
        let wanted = Matrix4::from_translation(Vector3::new(0.2, 1.1, 0.4))
            * Matrix4::from(rotation_from_euler([10.0, 0.0, 30.0]));
        let (position, rotation) = relative_to(&base, &wanted).unwrap();
        let mut anchor = Anchor::new("a", Some("mixamorig1:Spine".into()), position);
        anchor.set_rotation(&rotation);
        let frame = anchor_frame_in(Some(&skeleton), &root, &Matrix4::identity(), &anchor).unwrap();
        assert!(close(&frame, &wanted));
        assert!(near(frame.transform_point(Point3::origin()), Point3::new(0.2, 1.1, 0.4)));
    }

    #[test]
    fn new_anchors_get_free_names() {
        let anchors = vec![Anchor::new("anchor", None, [0.0; 3]), Anchor::new("anchor 2", None, [0.0; 3])];
        assert_eq!(free_name(&anchors, "anchor"), "anchor 3");
        assert_eq!(free_name(&anchors, "seat"), "seat");
    }

    // R5
    #[test]
    fn sidecars_without_anchors_stay_without() {
        use crate::core::assets::ModelSettings;
        let settings = ModelSettings::default();
        let text = ron::to_string(&settings).unwrap();
        assert!(!text.contains("anchors"));
        let mut with = settings.clone();
        with.anchors.push(Anchor::new("seat", None, [0.0, 0.7, 0.0]));
        let back: ModelSettings = ron::from_str(&ron::to_string(&with).unwrap()).unwrap();
        assert_eq!(back.anchors, with.anchors);
    }

    #[test]
    fn dragging_an_anchor_moves_it_or_the_model_it_places() {
        let delta = Matrix4::from_translation(Vector3::new(0.1, -0.05, 0.3))
            * Matrix4::from_axis_angle(Vector3::unit_y(), Deg(25.0));
        // An anchor on its own moves by the drag.
        let base = Matrix4::from_translation(Vector3::new(0.0, 1.0, 0.0));
        let frame = base * rotation_from_euler_matrix([10.0, 0.0, 0.0]) ;
        let (position, rotation) = moved(&base, &frame, &delta).unwrap();
        let mut anchor = Anchor::new("a", None, position);
        anchor.set_rotation(&rotation);
        assert!(close(&(base * anchor.local()), &(delta * frame)));

        // The seat a truck is placed by: the truck moves by the drag, the
        // seat stays on the character.
        let mut seat = Anchor::new("seat", None, [-0.38, 0.72, 0.04]);
        seat.rotation = [0.0, 180.0, 0.0];
        let on = Matrix4::from_translation(Vector3::new(0.0, 0.95, 0.0));
        let placed = place_on(&on, &seat.local()).unwrap();
        let (position, rotation) =
            moved_placing(&on, &seat.local(), &Matrix4::identity(), &delta).unwrap();
        let mut moved_seat = seat.clone();
        moved_seat.position = position;
        moved_seat.set_rotation(&rotation);
        let now = place_on(&on, &moved_seat.local()).unwrap();
        assert!(close(&now, &(delta * placed)));
    }

    fn rotation_from_euler_matrix(degrees: [f32; 3]) -> Matrix4<f32> {
        Matrix4::from(rotation_from_euler(degrees))
    }
}
