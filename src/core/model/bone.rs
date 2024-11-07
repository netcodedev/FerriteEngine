use cgmath::Vector3;

use super::{Bone, Pose};

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

    pub fn apply_pose(&mut self, pose: &Pose, is_root: bool) -> Vector3<f32> {
        let mut root_motion = Vector3::new(0.0, 0.0, 0.0);
        if let Some(transform) = pose.transforms.get(&self.name) {
            if is_root {
                if pose.cycle_completed {
                    self.last_translation = transform.translation;
                }
                let delta = transform.translation - self.last_translation;
                self.last_translation = transform.translation;
                root_motion = delta;
                self.current_transform = transform.to_matrix_4_without_translation();
            } else {
                self.current_transform = transform.to_matrix_4();
            }
        }
        if let Some(children) = &mut self.children {
            for child in children {
                child.apply_pose(pose, false);
            }
        }
        root_motion
    }
}
