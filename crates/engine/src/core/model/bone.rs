use std::{cell::OnceCell, collections::HashMap};

use cgmath::{Transform, Vector3};

use super::{ik::base_name, Bone, LocalTransform, Pose};

/// The transforms of a pose, looked up by bone name, or else by the name
/// without namespace (`mixamorig:Hips` for a bone `mixamorig1:Hips`): the
/// first of the pose's names that has it, in the order of the pose's map.
/// That index is made once, when a bone first needs it, for every bone
/// and mesh posed with it.
pub(super) struct PoseChannels<'a> {
    pose: &'a Pose,
    by_base_name: OnceCell<HashMap<&'a str, &'a LocalTransform>>,
}

impl<'a> PoseChannels<'a> {
    pub(super) fn new(pose: &'a Pose) -> Self {
        Self {
            pose,
            by_base_name: OnceCell::new(),
        }
    }

    fn get(&self, name: &str) -> Option<&'a LocalTransform> {
        let pose = self.pose;
        pose.transforms.get(name).or_else(|| {
            self.by_base_name
                .get_or_init(|| {
                    let mut index = HashMap::with_capacity(pose.transforms.len());
                    for (key, transform) in &pose.transforms {
                        index.entry(base_name(key)).or_insert(transform);
                    }
                    index
                })
                .get(base_name(name))
                .copied()
        })
    }
}

impl Bone {
    pub fn get_as_vec(&self) -> Vec<Bone> {
        let mut bones = Vec::<Bone>::new();
        bones.push(self.clone());
        if let Some(children) = &self.children {
            for child in children {
                bones.extend(child.get_as_vec());
            }
        }
        bones
    }

    pub(super) fn apply_pose(&mut self, channels: &PoseChannels, is_root: bool) -> Vector3<f32> {
        let pose = channels.pose;
        let mut root_motion = Vector3::new(0.0, 0.0, 0.0);
        let transform = channels.get(&self.name);

        if let Some(transform) = transform {
            if is_root {
                let mut current_translation = transform.translation;
                if let Some(err) = pose.translation_errors.get(&self.name) {
                    current_translation -= *err;
                }

                if pose.cycle_completed || pose.transition_finished {
                    self.last_translation = current_translation;
                }

                let delta = current_translation - self.last_translation;
                self.last_translation = current_translation;
                root_motion = delta;
                self.current_transform = transform.to_matrix_4_without_translation();
            } else {
                self.current_transform = transform.to_matrix_4();
            }
        }
        // A root that is not a bone and not animated (a Blender "Armature"
        // node above the hips) passes the root on to the bones below it.
        let pass_root = is_root && transform.is_none() && self.id == usize::MAX;
        if let Some(children) = &mut self.children {
            for child in children {
                let motion = child.apply_pose(channels, pass_root);
                if pass_root {
                    root_motion += self.current_transform.transform_vector(motion);
                }
            }
        }
        root_motion
    }
}

#[cfg(test)]
mod tests {
    use cgmath::{Deg, InnerSpace, Matrix4, Quaternion, Rotation3, SquareMatrix, Zero};

    use super::*;

    /// How bones were posed before the base name index: a search through
    /// the pose for every bone the pose has no channel of by its name.
    fn apply_pose_by_search(bone: &mut Bone, pose: &Pose, is_root: bool) -> Vector3<f32> {
        let mut root_motion = Vector3::new(0.0, 0.0, 0.0);
        let transform = pose.transforms.get(&bone.name).or_else(|| {
            let self_base_name = bone.name.split(':').last().unwrap_or(&bone.name);
            pose.transforms.iter().find_map(|(k, v)| {
                let k_base_name = k.split(':').last().unwrap_or(k);
                (self_base_name == k_base_name).then_some(v)
            })
        });
        if let Some(transform) = transform {
            if is_root {
                let mut current_translation = transform.translation;
                if let Some(err) = pose.translation_errors.get(&bone.name) {
                    current_translation -= *err;
                }
                if pose.cycle_completed || pose.transition_finished {
                    bone.last_translation = current_translation;
                }
                let delta = current_translation - bone.last_translation;
                bone.last_translation = current_translation;
                root_motion = delta;
                bone.current_transform = transform.to_matrix_4_without_translation();
            } else {
                bone.current_transform = transform.to_matrix_4();
            }
        }
        let pass_root = is_root && transform.is_none() && bone.id == usize::MAX;
        if let Some(children) = &mut bone.children {
            for child in children {
                let motion = apply_pose_by_search(child, pose, pass_root);
                if pass_root {
                    root_motion += bone.current_transform.transform_vector(motion);
                }
            }
        }
        root_motion
    }

    fn bone(id: usize, name: String, children: Vec<Bone>) -> Bone {
        Bone {
            id,
            name,
            offset_matrix: Matrix4::identity(),
            weights: Vec::new(),
            children: (!children.is_empty()).then_some(children),
            current_transform: Matrix4::from_scale(0.5),
            last_translation: Vector3::zero(),
        }
    }

    /// An "Armature" node over a rig of 65 bones in `namespace`, five
    /// limbs of 13 bones each below the first.
    fn rig(namespace: &str) -> Bone {
        let mut id = 0;
        let mut limbs = Vec::new();
        for limb in 0..5 {
            let mut chain: Option<Bone> = None;
            for joint in (0..13).rev() {
                id += 1;
                let name = format!("{namespace}Limb{limb}Joint{joint}");
                chain = Some(bone(id, name, chain.into_iter().collect()));
            }
            limbs.extend(chain);
        }
        bone(
            usize::MAX,
            "Armature".into(),
            vec![bone(0, format!("{namespace}Hips"), limbs)],
        )
    }

    fn pose(frame: u32) -> Pose {
        let mut pose = Pose::new();
        let mut add = |name: String, i: u32| {
            let t = (frame * 7 + i) as f32;
            pose.add_transform(
                name,
                LocalTransform {
                    translation: Vector3::new(t * 0.01, t.sin(), t.cos() * 0.3),
                    rotation: Quaternion::from_axis_angle(
                        Vector3::new(0.3, 1.0, 0.2).normalize(),
                        Deg(t * 3.7),
                    ),
                    scale: Vector3::new(1.0, 1.0 + t * 1e-3, 1.0),
                },
            )
        };
        add("mixamorig:Hips".into(), 0);
        for limb in 0..5 {
            // One joint has no channel, one has two of the same base name.
            for joint in 0..12 {
                add(
                    format!("mixamorig:Limb{limb}Joint{joint}"),
                    limb * 13 + joint + 1,
                );
            }
            add(format!("other:Limb{limb}Joint3"), 100 + limb);
        }
        pose.translation_errors
            .insert("mixamorig1:Hips".into(), Vector3::new(0.1, 0.0, 0.0));
        pose.cycle_completed = frame == 1;
        pose
    }

    fn collect(bone: &Bone, out: &mut Vec<u32>) {
        let m: &[f32; 16] = bone.current_transform.as_ref();
        out.extend(m.iter().map(|v| v.to_bits()));
        out.extend(
            [
                bone.last_translation.x,
                bone.last_translation.y,
                bone.last_translation.z,
            ]
            .map(f32::to_bits),
        );
        for child in bone.children.iter().flatten() {
            collect(child, out);
        }
    }

    #[test]
    fn bones_of_another_namespace_are_posed_bit_for_bit_as_by_searching() {
        let (mut searched, mut indexed) = (rig("mixamorig1:"), rig("mixamorig1:"));
        for frame in 0..4 {
            let pose = pose(frame);
            let a = apply_pose_by_search(&mut searched, &pose, true);
            let b = indexed.apply_pose(&PoseChannels::new(&pose), true);
            assert_eq!(a.x.to_bits(), b.x.to_bits());
            assert_eq!(a.y.to_bits(), b.y.to_bits());
            assert_eq!(a.z.to_bits(), b.z.to_bits());
            let (mut x, mut y) = (Vec::new(), Vec::new());
            collect(&searched, &mut x);
            collect(&indexed, &mut y);
            assert_eq!(x, y, "frame {frame}");
        }
        // The same names as the pose's: posed by name, as before.
        let (mut searched, mut indexed) = (rig("mixamorig:"), rig("mixamorig:"));
        let pose = pose(2);
        apply_pose_by_search(&mut searched, &pose, true);
        indexed.apply_pose(&PoseChannels::new(&pose), true);
        let (mut x, mut y) = (Vec::new(), Vec::new());
        collect(&searched, &mut x);
        collect(&indexed, &mut y);
        assert_eq!(x, y);
    }
}
