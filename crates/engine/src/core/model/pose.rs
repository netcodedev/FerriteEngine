use std::collections::{HashMap, HashSet};

use cgmath::{InnerSpace, Matrix3, Matrix4, Quaternion, Vector3};

use super::{ik::base_name, LocalTransform, Pose};

impl LocalTransform {
    /// Translation, rotation and scale of a transform without shear.
    pub fn from_matrix(m: &Matrix4<f32>) -> LocalTransform {
        let x = m.x.truncate();
        let y = m.y.truncate();
        let z = m.z.truncate();
        let scale = Vector3::new(x.magnitude(), y.magnitude(), z.magnitude());
        let rotation = Quaternion::from(Matrix3::from_cols(
            x / scale.x.max(f32::EPSILON),
            y / scale.y.max(f32::EPSILON),
            z / scale.z.max(f32::EPSILON),
        ))
        .normalize();
        LocalTransform {
            translation: m.w.truncate(),
            rotation,
            scale,
        }
    }

    pub fn interpolate(&self, other: &LocalTransform, factor: f32) -> LocalTransform {
        LocalTransform {
            translation: (self.translation * (1.0 - factor)) + (other.translation * factor),
            rotation: self.rotation.slerp(other.rotation, factor),
            scale: (self.scale * (1.0 - factor)) + (other.scale * factor),
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
            transition_finished: false,
            translation_errors: HashMap::new(),
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
        for (key, transform) in &other.transforms {
            if !self.transforms.contains_key(key) {
                pose.add_transform(key.clone(), transform.clone());
            }
        }
        pose.cycle_completed = self.cycle_completed || other.cycle_completed;
        pose
    }

    pub fn add_transform(&mut self, name: String, transform: LocalTransform) {
        self.transforms.insert(name, transform);
    }

    /// Add the bones of `rest` (by name without namespace: full name and
    /// transform) this pose does not move, as they are there, so every
    /// bone is posed: nothing another clip or the inverse kinematics left
    /// on a bone lingers.
    pub fn fill_from(&mut self, rest: &HashMap<String, (String, LocalTransform)>) {
        let moved: HashSet<String> = self
            .transforms
            .keys()
            .map(|key| base_name(key).to_string())
            .collect();
        for (base, (name, transform)) in rest {
            if !moved.contains(base) {
                self.transforms.insert(name.clone(), transform.clone());
            }
        }
    }

    /// Blend `other` over this pose by `weight` (0: this pose, 1: `other`),
    /// only for the bones whose names without namespace
    /// (`ik::base_name`) are in `bones`: e.g. a swing of the arms over the
    /// legs that walk on. Bones are matched by that name, so poses of rigs
    /// exported with different namespaces mix.
    pub fn overlay(&mut self, other: &Pose, weight: f32, bones: &HashSet<String>) {
        let weight = weight.clamp(0.0, 1.0);
        if weight <= 0.0 {
            return;
        }
        let keys: HashMap<String, String> = self
            .transforms
            .keys()
            .map(|key| (base_name(key).to_string(), key.clone()))
            .collect();
        for (name, transform) in &other.transforms {
            let base = base_name(name);
            if !bones.contains(base) {
                continue;
            }
            match keys.get(base) {
                Some(key) => {
                    let blended = self.transforms[key].interpolate(transform, weight);
                    self.transforms.insert(key.clone(), blended);
                }
                None => {
                    self.transforms.insert(name.clone(), transform.clone());
                }
            }
        }
    }
}
