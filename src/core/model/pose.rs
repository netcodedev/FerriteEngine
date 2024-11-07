use std::collections::HashMap;

use cgmath::Matrix4;

use super::{LocalTransform, Pose};

impl LocalTransform {
    pub fn interpolate(&self, other: &LocalTransform, factor: f32) -> LocalTransform {
        LocalTransform {
            translation: (self.translation * factor) + (&other.translation * (1.0 - factor)),
            rotation: self.rotation.slerp(other.rotation, factor),
            scale: (self.scale * factor) + (&other.scale * (1.0 - factor)),
        }
    }

    pub fn to_matrix_4(&self) -> Matrix4<f32> {
        Matrix4::from_translation(self.translation)
            * Matrix4::from(self.rotation)
            * Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z)
    }

    pub fn to_matrix_4_without_translation(&self) -> Matrix4<f32> {
        Matrix4::from(self.rotation)
            * Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z)
    }
}

impl Pose {
    pub fn new() -> Pose {
        Pose {
            transforms: HashMap::new(),
            cycle_completed: false,
        }
    }

    pub fn interpolate(&self, other: &Pose, factor: f32) -> Pose {
        let mut pose = Pose::new();
        for (key, transform) in &self.transforms {
            if let Some(other_transform) = other.transforms.get(key) {
                pose.add_transform(key.clone(), transform.interpolate(other_transform, factor));
            } else {
                pose.add_transform(key.clone(), transform.clone());
            }
        }
        pose
    }

    pub fn add_transform(&mut self, name: String, transform: LocalTransform) {
        self.transforms.insert(name, transform);
    }
}
