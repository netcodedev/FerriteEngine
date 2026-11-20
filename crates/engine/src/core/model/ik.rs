//! Inverse kinematics on a model's skeleton: a flat copy of its bone
//! hierarchy that can be posed by moving end effectors (feet, hands, head),
//! and written back to the model with `Model::apply_skeleton`.
//!
//! All positions are in skeleton space: the space the model's meshes are
//! authored in, before the model's own position and scale
//! (`Model::root_transform`).

use cgmath::{
    EuclideanSpace, InnerSpace, Matrix3, Matrix4, Point3, Quaternion, Rad, Rotation3, SquareMatrix,
    Transform, Vector3,
};

/// One node of the hierarchy below a model's root bone.
#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    /// Index of the parent joint; parents come before their children.
    pub parent: Option<usize>,
    /// Transform relative to the parent.
    pub local: Matrix4<f32>,
    /// Whether vertices are skinned to it (helper nodes are not).
    pub skinned: bool,
}

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
    /// Local transforms of the pose the skeleton was made from.
    rest: Vec<Matrix4<f32>>,
}

/// How a chain is solved.
#[derive(Clone, Debug)]
pub enum ChainKind {
    /// Analytic: `upper` and `mid` rotate so that `end` reaches the target,
    /// bending towards the chain's pole (thigh → shin → foot).
    TwoBone { upper: usize, mid: usize },
    /// Cyclic coordinate descent over these joints, root first, for longer
    /// chains (spine → neck → head).
    Ccd { joints: Vec<usize> },
}

/// A chain of joints moved by dragging its end effector.
#[derive(Clone, Debug)]
pub struct IkChain {
    /// The end effector's name without its namespace, e.g. `LeftFoot`.
    pub label: String,
    /// The joint that follows the target.
    pub end: usize,
    pub kind: ChainKind,
    /// Direction, in skeleton space, the middle joint bends towards (knees
    /// forward, elbows back).
    pub pole: Vector3<f32>,
}

/// Rotation sweeps a CCD step makes towards the target at most.
const CCD_ITERATIONS: usize = 24;
/// Largest rotation (radians) a CCD step gives one joint; keeps the spine
/// from folding in one go.
const CCD_MAX_STEP: f32 = 0.12;

/// The name without the namespace an exporter puts in front
/// (`mixamorig:LeftFoot` → `LeftFoot`).
pub fn base_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Lowercase, without separators, for comparing names of different rigs.
fn normalized(name: &str) -> String {
    base_name(name)
        .chars()
        .filter(|c| !matches!(c, '_' | '.' | ' ' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether a normalized name ends with `part`, maybe followed by a side
/// letter (`foot`, `leftfoot`, `footl`, `foot_r`).
fn ends_with_part(name: &str, part: &str) -> bool {
    name.ends_with(part)
        || name
            .strip_suffix(['l', 'r'])
            .is_some_and(|rest| rest.ends_with(part))
}

/// Nodes the FBX importer inserts between bones for pivots.
fn is_helper(name: &str) -> bool {
    name.contains("$AssimpFbx$")
}

fn translation(m: &Matrix4<f32>) -> Point3<f32> {
    Point3::from_vec(m.w.truncate())
}

impl Skeleton {
    pub fn new(joints: Vec<Joint>) -> Self {
        let rest = joints.iter().map(|j| j.local).collect();
        Self { joints, rest }
    }

    /// Back to the pose the skeleton was made from.
    pub fn reset(&mut self) {
        for (joint, rest) in self.joints.iter_mut().zip(&self.rest) {
            joint.local = *rest;
        }
    }

    pub fn locals(&self) -> Vec<Matrix4<f32>> {
        self.joints.iter().map(|j| j.local).collect()
    }

    pub fn set_locals(&mut self, locals: &[Matrix4<f32>]) {
        for (joint, local) in self.joints.iter_mut().zip(locals) {
            joint.local = *local;
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// Skeleton-space transform of every joint.
    pub fn globals(&self) -> Vec<Matrix4<f32>> {
        let mut out: Vec<Matrix4<f32>> = Vec::with_capacity(self.joints.len());
        for joint in &self.joints {
            let global = match joint.parent {
                Some(parent) => out[parent] * joint.local,
                None => joint.local,
            };
            out.push(global);
        }
        out
    }

    pub fn positions(&self) -> Vec<Point3<f32>> {
        self.globals().iter().map(translation).collect()
    }

    /// The nearest ancestor that is not an importer helper node.
    fn bone_parent(&self, joint: usize) -> Option<usize> {
        let mut current = self.joints[joint].parent?;
        while is_helper(&self.joints[current].name) {
            current = self.joints[current].parent?;
        }
        Some(current)
    }

    /// Rotate `joint` about its own position by `rotation`, given in
    /// skeleton space; everything below it follows.
    pub fn turn(&mut self, joint: usize, rotation: Quaternion<f32>) {
        let mut globals = self.globals();
        self.rotate(joint, rotation, &mut globals);
    }

    /// The toe joint below a foot (`LeftToeBase`), if it has one.
    pub fn toe_of(&self, foot: usize) -> Option<usize> {
        (0..self.joints.len()).find(|&joint| {
            let name = normalized(&self.joints[joint].name);
            self.bone_parent(joint) == Some(foot) && name.contains("toe") && !name.contains("end")
        })
    }

    /// The chains of `default_chains` that end in a foot.
    pub fn foot_chains(&self) -> Vec<IkChain> {
        self.default_chains()
            .into_iter()
            .filter(|chain| ends_with_part(&normalized(&chain.label), "foot"))
            .collect()
    }

    /// Rotate `joint` about its own position by `rotation`, given in
    /// skeleton space; everything below it follows. `globals` must be
    /// current and is kept so.
    fn rotate(&mut self, joint: usize, rotation: Quaternion<f32>, globals: &mut Vec<Matrix4<f32>>) {
        let global = globals[joint];
        let mut turned = Matrix4::from(rotation) * global;
        turned.w = global.w;
        let parent = self.joints[joint]
            .parent
            .map_or(Matrix4::identity(), |p| globals[p]);
        if let Some(inverse) = parent.invert() {
            self.joints[joint].local = inverse * turned;
        }
        *globals = self.globals();
    }

    /// Give `joint` this skeleton-space orientation, keeping its position.
    fn set_orientation(
        &mut self,
        joint: usize,
        orientation: Matrix3<f32>,
        globals: &mut Vec<Matrix4<f32>>,
    ) {
        let mut wanted = Matrix4::from(orientation);
        wanted.w = globals[joint].w;
        let parent = self.joints[joint]
            .parent
            .map_or(Matrix4::identity(), |p| globals[p]);
        if let Some(inverse) = parent.invert() {
            self.joints[joint].local = inverse * wanted;
        }
        *globals = self.globals();
    }

    /// Move the chain's end effector to `target`; the end keeps the
    /// orientation it had.
    pub fn solve(&mut self, chain: &IkChain, target: Point3<f32>) {
        let mut globals = self.globals();
        let end_orientation = linear(&globals[chain.end]);
        match &chain.kind {
            ChainKind::TwoBone { upper, mid } => {
                self.solve_two_bone(*upper, *mid, chain.end, target, chain.pole, &mut globals)
            }
            ChainKind::Ccd { joints } => self.solve_ccd(joints, chain.end, target, &mut globals),
        }
        self.set_orientation(chain.end, end_orientation, &mut globals);
    }

    fn solve_two_bone(
        &mut self,
        upper: usize,
        mid: usize,
        end: usize,
        target: Point3<f32>,
        pole: Vector3<f32>,
        globals: &mut Vec<Matrix4<f32>>,
    ) {
        let a = translation(&globals[upper]);
        let b = translation(&globals[mid]);
        let c = translation(&globals[end]);
        let upper_len = (b - a).magnitude();
        let lower_len = (c - b).magnitude();
        let to_target = target - a;
        let distance = to_target.magnitude();
        if upper_len < 1e-6 || lower_len < 1e-6 || distance < 1e-6 {
            return;
        }
        let axis = to_target / distance;
        // Out of reach: stretch towards it; too close: fold as far as the
        // bones allow.
        let reach = distance.clamp(
            (upper_len - lower_len).abs() + 1e-4 * upper_len,
            (upper_len + lower_len) * 0.9999,
        );
        let cos = ((upper_len * upper_len + reach * reach - lower_len * lower_len)
            / (2.0 * upper_len * reach))
            .clamp(-1.0, 1.0);
        let sin = (1.0 - cos * cos).sqrt();

        // Bend towards the pole; a pole along the reach falls back on how
        // the limb is bent now.
        let mut bend = pole - axis * pole.dot(axis);
        if bend.magnitude2() < 1e-8 {
            let current = b - a;
            bend = current - axis * current.dot(axis);
        }
        if bend.magnitude2() < 1e-8 {
            bend = any_perpendicular(axis);
        }
        let bend = bend.normalize();
        let knee = a + axis * (cos * upper_len) + bend * (sin * upper_len);

        let turn_upper = Quaternion::from_arc((b - a).normalize(), (knee - a).normalize(), None);
        self.rotate(upper, turn_upper, globals);

        let b = translation(&globals[mid]);
        let c = translation(&globals[end]);
        let goal = a + axis * reach;
        if (c - b).magnitude2() > 1e-12 && (goal - b).magnitude2() > 1e-12 {
            let turn_lower = Quaternion::from_arc((c - b).normalize(), (goal - b).normalize(), None);
            self.rotate(mid, turn_lower, globals);
        }
    }

    fn solve_ccd(
        &mut self,
        joints: &[usize],
        end: usize,
        target: Point3<f32>,
        globals: &mut Vec<Matrix4<f32>>,
    ) {
        let tolerance = 1e-4;
        for _ in 0..CCD_ITERATIONS {
            for &joint in joints.iter().rev() {
                let pivot = translation(&globals[joint]);
                let effector = translation(&globals[end]);
                let from = effector - pivot;
                let to = target - pivot;
                if from.magnitude2() < 1e-12 || to.magnitude2() < 1e-12 {
                    continue;
                }
                let (from, to) = (from.normalize(), to.normalize());
                let angle = from.dot(to).clamp(-1.0, 1.0).acos();
                if angle < 1e-5 {
                    continue;
                }
                let turn = if angle > CCD_MAX_STEP {
                    let axis = from.cross(to);
                    if axis.magnitude2() < 1e-12 {
                        continue;
                    }
                    Quaternion::from_axis_angle(axis.normalize(), Rad(CCD_MAX_STEP))
                } else {
                    Quaternion::from_arc(from, to, None)
                };
                self.rotate(joint, turn, globals);
            }
            if (translation(&globals[end]) - target).magnitude() < tolerance {
                break;
            }
        }
    }

    /// Bend the two-bone limb ending in the joint `end` (its parent and
    /// grandparent bones bend: shin and thigh, forearm and upper arm) so
    /// that `end` moves `weight` of the way to `target`, the middle joint
    /// bending towards `pole`; `end` keeps its orientation. All in skeleton
    /// space. Whether `end` has two bones above it.
    pub fn reach_towards(
        &mut self,
        end: usize,
        target: Point3<f32>,
        pole: Vector3<f32>,
        weight: f32,
    ) -> bool {
        let weight = weight.clamp(0.0, 1.0);
        let Some(mid) = self.bone_parent(end) else {
            return false;
        };
        let Some(upper) = self.bone_parent(mid) else {
            return false;
        };
        if !(weight > 0.0) || !finite_point(target) || !finite_vector(pole) {
            return false;
        }
        let from = self.positions()[end];
        let chain = IkChain {
            label: base_name(&self.joints[end].name).to_string(),
            end,
            kind: ChainKind::TwoBone { upper, mid },
            pole,
        };
        self.solve(&chain, from + (target - from) * weight);
        true
    }

    /// The direction the character faces, from its heels to its toes
    /// (+Z if it has no feet with toes).
    fn forward(&self, positions: &[Point3<f32>]) -> Vector3<f32> {
        let mut sum = Vector3::new(0.0, 0.0, 0.0);
        for (i, joint) in self.joints.iter().enumerate() {
            let name = normalized(&joint.name);
            if !(name.contains("toe") && !name.contains("end")) {
                continue;
            }
            if let Some(foot) = self.bone_parent(i) {
                if normalized(&self.joints[foot].name).contains("foot") {
                    let d = positions[i] - positions[foot];
                    sum += Vector3::new(d.x, 0.0, d.z);
                }
            }
        }
        if sum.magnitude2() < 1e-12 {
            Vector3::unit_z()
        } else {
            sum.normalize()
        }
    }

    /// Chains for the feet, hands and head, found from the bone names.
    pub fn default_chains(&self) -> Vec<IkChain> {
        let positions = self.positions();
        let forward = self.forward(&positions);
        (0..self.joints.len())
            .filter_map(|end| self.chain_at(end, forward))
            .collect()
    }

    /// The chain of `default_chains` that ends in `end`, if there is one,
    /// without making the others.
    pub fn chain_ending_at(&self, end: usize) -> Option<IkChain> {
        let positions = self.positions();
        let forward = self.forward(&positions);
        self.chain_at(end, forward)
    }

    /// The chain ending in the joint `end`, if it is a foot, hand or head,
    /// for a character facing `forward`.
    fn chain_at(&self, end: usize, forward: Vector3<f32>) -> Option<IkChain> {
        let joint = &self.joints[end];
        let name = normalized(&joint.name);
        let label = base_name(&joint.name).to_string();
        let limb = if ends_with_part(&name, "foot") {
            Some(forward)
        } else if ends_with_part(&name, "hand") {
            // Elbows bend back and a little down.
            Some((-forward - Vector3::unit_y() * 0.3).normalize())
        } else {
            None
        };
        if let Some(pole) = limb {
            let mid = self.bone_parent(end)?;
            let upper = self.bone_parent(mid)?;
            Some(IkChain {
                label,
                end,
                kind: ChainKind::TwoBone { upper, mid },
                pole,
            })
        } else if ends_with_part(&name, "head") {
            // The neck and spine bones above the hips bend for it.
            let mut joints = Vec::new();
            let mut current = self.bone_parent(end);
            while let Some(j) = current {
                let n = normalized(&self.joints[j].name);
                if !(n.contains("neck") || n.contains("spine") || n.contains("chest")) {
                    break;
                }
                joints.push(j);
                current = self.bone_parent(j);
            }
            if joints.is_empty() {
                return None;
            }
            joints.reverse();
            Some(IkChain {
                label,
                end,
                kind: ChainKind::Ccd { joints },
                pole: forward,
            })
        } else {
            None
        }
    }

    /// Joint positions with the model's root transform applied.
    pub fn world_positions(&self, root: &Matrix4<f32>) -> Vec<Point3<f32>> {
        self.positions()
            .into_iter()
            .map(|p| root.transform_point(p))
            .collect()
    }
}

fn finite_point(p: Point3<f32>) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}

fn finite_vector(v: Vector3<f32>) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn linear(m: &Matrix4<f32>) -> Matrix3<f32> {
    Matrix3::from_cols(m.x.truncate(), m.y.truncate(), m.z.truncate())
}

fn any_perpendicular(v: Vector3<f32>) -> Vector3<f32> {
    let other = if v.x.abs() < 0.9 {
        Vector3::unit_x()
    } else {
        Vector3::unit_y()
    };
    v.cross(other).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joint(name: &str, parent: Option<usize>, offset: [f32; 3]) -> Joint {
        Joint {
            name: name.into(),
            parent,
            local: Matrix4::from_translation(offset.into()),
            skinned: true,
        }
    }

    /// Hips with one straight leg pointing down and a toe pointing +Z.
    fn leg() -> Skeleton {
        Skeleton::new(vec![
            joint("rig:Hips", None, [0.0, 2.0, 0.0]),
            joint("rig:LeftUpLeg", Some(0), [0.2, 0.0, 0.0]),
            joint("rig:LeftLeg", Some(1), [0.0, -1.0, 0.0]),
            joint("rig:LeftFoot", Some(2), [0.0, -1.0, 0.0]),
            joint("rig:LeftToeBase", Some(3), [0.0, 0.0, 0.3]),
        ])
    }

    #[test]
    fn finds_a_two_bone_chain_for_a_foot() {
        let skeleton = leg();
        let chains = skeleton.default_chains();
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].label, "LeftFoot");
        assert!(matches!(chains[0].kind, ChainKind::TwoBone { upper: 1, mid: 2 }));
        assert!((chains[0].pole - Vector3::unit_z()).magnitude() < 1e-5);
    }

    #[test]
    fn the_foot_reaches_the_target_and_the_knee_bends_forward() {
        let mut skeleton = leg();
        let chain = skeleton.default_chains().remove(0);
        let target = Point3::new(0.2, 0.8, 0.3);
        skeleton.solve(&chain, target);
        let p = skeleton.positions();
        assert!((p[3] - target).magnitude() < 1e-3, "foot at {:?}", p[3]);
        assert!(p[2].z > 0.3, "knee at {:?}", p[2]);
        // Bone lengths stay.
        assert!(((p[2] - p[1]).magnitude() - 1.0).abs() < 1e-4);
        assert!(((p[3] - p[2]).magnitude() - 1.0).abs() < 1e-4);
        // The foot keeps pointing forward.
        assert!(((p[4] - p[3]) - Vector3::new(0.0, 0.0, 0.3)).magnitude() < 1e-3);
    }

    #[test]
    fn a_foot_has_its_toe() {
        let skeleton = leg();
        assert_eq!(skeleton.toe_of(3), Some(4));
        assert_eq!(skeleton.toe_of(2), None);
    }

    #[test]
    fn an_unreachable_target_stretches_the_leg_towards_it() {
        let mut skeleton = leg();
        let chain = skeleton.default_chains().remove(0);
        skeleton.solve(&chain, Point3::new(0.2, -5.0, 0.0));
        let p = skeleton.positions();
        assert!((p[3] - Point3::new(0.2, 0.0, 0.0)).magnitude() < 1e-3, "{:?}", p[3]);
    }

    #[test]
    fn a_limb_reaches_with_its_middle_joint_bent_towards_the_pole() {
        let mut skeleton = leg();
        // A sitting leg: the foot ahead of the hips and low, the knee up.
        let target = Point3::new(0.2, 1.2, 1.5);
        assert!(skeleton.reach_towards(3, target, Vector3::unit_y(), 1.0));
        let p = skeleton.positions();
        assert!((p[3] - target).magnitude() < 1e-3, "foot at {:?}", p[3]);
        let line = p[1] + (target - p[1]) * ((p[2] - p[1]).dot(target - p[1]) / (target - p[1]).magnitude2());
        assert!(p[2].y > line.y + 0.1, "knee {:?} above the hip-foot line {line:?}", p[2]);
        assert!(((p[2] - p[1]).magnitude() - 1.0).abs() < 1e-4);
        // The foot keeps pointing forward.
        assert!(((p[4] - p[3]) - Vector3::new(0.0, 0.0, 0.3)).magnitude() < 1e-3);

        // Half the weight: half of the way.
        let mut half = leg();
        let from = half.positions()[3];
        assert!(half.reach_towards(3, target, Vector3::unit_y(), 0.5));
        let wanted = from + (target - from) * 0.5;
        assert!((half.positions()[3] - wanted).magnitude() < 1e-3);

        // No weight, no limb, or no number: nothing moves.
        let mut still = leg();
        let before = still.locals();
        assert!(!still.reach_towards(3, target, Vector3::unit_y(), 0.0));
        assert!(!still.reach_towards(1, target, Vector3::unit_y(), 1.0));
        assert!(!still.reach_towards(3, Point3::new(f32::NAN, 0.0, 0.0), Vector3::unit_y(), 1.0));
        assert_eq!(still.locals(), before);
    }

    #[test]
    fn ccd_moves_the_head_towards_the_target() {
        let mut skeleton = Skeleton::new(vec![
            joint("Hips", None, [0.0, 1.0, 0.0]),
            joint("Spine", Some(0), [0.0, 0.3, 0.0]),
            joint("Spine1", Some(1), [0.0, 0.3, 0.0]),
            joint("Neck", Some(2), [0.0, 0.3, 0.0]),
            joint("Head", Some(3), [0.0, 0.2, 0.0]),
        ]);
        let chain = skeleton.default_chains().remove(0);
        assert!(matches!(&chain.kind, ChainKind::Ccd { joints } if joints == &vec![1, 2, 3]));
        let target = Point3::new(0.3, 2.0, 0.2);
        skeleton.solve(&chain, target);
        let head = skeleton.positions()[4];
        assert!((head - target).magnitude() < 0.01, "head at {head:?}");
    }

    #[test]
    fn names_of_other_rigs_are_recognised() {
        for name in ["foot_l", "Foot.R", "LeftFoot", "mixamorig:RightFoot"] {
            assert!(ends_with_part(&normalized(name), "foot"), "{name}");
        }
        for name in ["LeftHandThumb1", "LeftToeBase", "HeadTop_End"] {
            let n = normalized(name);
            assert!(
                !ends_with_part(&n, "hand") && !ends_with_part(&n, "foot") && !ends_with_part(&n, "head"),
                "{name}"
            );
        }
    }
}
