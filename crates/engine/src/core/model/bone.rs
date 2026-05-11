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
        let transform = pose.transforms.get(&self.name).or_else(|| {
            let self_base_name = self.name.split(':').last().unwrap_or(&self.name);
            pose.transforms.iter().find_map(|(k, v)| {
                let k_base_name = k.split(':').last().unwrap_or(k);
                if self_base_name == k_base_name {
                    Some(v)
                } else {
                    None
                }
            })
        });

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
        if let Some(children) = &mut self.children {
            for child in children {
                child.apply_pose(pose, false);
            }
        }
        root_motion
    }
}
