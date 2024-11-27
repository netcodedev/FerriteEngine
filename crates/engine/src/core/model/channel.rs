use cgmath::{Quaternion, Vector3};
use russimp::animation::NodeAnim;

use super::{Channel, LocalTransform};

impl Channel {
    pub fn new(channel: &NodeAnim) -> Channel {
        let mut position_keys = Vec::<(f32, Vector3<f32>)>::new();
        for key in &channel.position_keys {
            position_keys.push((
                key.time as f32,
                Vector3::new(key.value.x, key.value.y, key.value.z),
            ));
        }
        let mut rotation_keys = Vec::<(f32, Quaternion<f32>)>::new();
        for key in &channel.rotation_keys {
            rotation_keys.push((
                key.time as f32,
                Quaternion::new(key.value.w, key.value.x, key.value.y, key.value.z),
            ));
        }
        let mut scaling_keys = Vec::<(f32, Vector3<f32>)>::new();
        for key in &channel.scaling_keys {
            scaling_keys.push((
                key.time as f32,
                Vector3::new(key.value.x, key.value.y, key.value.z),
            ));
        }
        Channel {
            bone_id: channel.name.clone(),
            position_keys,
            rotation_keys,
            scaling_keys,
        }
    }

    pub fn sample(&self, time: f32) -> LocalTransform {
        let position = self.interpolate_position(time);
        let rotation = self.interpolate_rotation(time);
        let scaling = self.interpolate_scaling(time);
        LocalTransform {
            translation: position,
            rotation,
            scale: scaling,
        }
    }

    fn get_position_index(&self, time: f32) -> usize {
        for i in 0..self.position_keys.len() {
            if self.position_keys[i].0 > time {
                return i - 1;
            }
        }
        0
    }

    fn get_rotation_index(&self, time: f32) -> usize {
        for i in 0..self.rotation_keys.len() {
            if self.rotation_keys[i].0 > time {
                return i - 1;
            }
        }
        0
    }

    fn get_scaling_index(&self, time: f32) -> usize {
        for i in 0..self.scaling_keys.len() {
            if self.scaling_keys[i].0 > time {
                return i - 1;
            }
        }
        0
    }

    fn interpolate_position(&self, time: f32) -> Vector3<f32> {
        let position_index = self.get_position_index(time);
        let next_position_index = position_index + 1;
        if next_position_index >= self.position_keys.len() {
            return self.position_keys[position_index].1;
        }
        let delta_time =
            self.position_keys[next_position_index].0 - self.position_keys[position_index].0;
        let factor = (time - self.position_keys[position_index].0) / delta_time;
        let start = self.position_keys[position_index].1;
        let end = self.position_keys[next_position_index].1;
        start + (end - start) * factor
    }

    fn interpolate_rotation(&self, time: f32) -> Quaternion<f32> {
        let rotation_index = self.get_rotation_index(time);
        let next_rotation_index = rotation_index + 1;
        if next_rotation_index >= self.rotation_keys.len() {
            return self.rotation_keys[rotation_index].1;
        }
        let delta_time =
            self.rotation_keys[next_rotation_index].0 - self.rotation_keys[rotation_index].0;
        let factor = (time - self.rotation_keys[rotation_index].0) / delta_time;
        let start = self.rotation_keys[rotation_index].1;
        let end = self.rotation_keys[next_rotation_index].1;
        Quaternion::slerp(start, end, factor)
    }

    fn interpolate_scaling(&self, time: f32) -> Vector3<f32> {
        let scaling_index = self.get_scaling_index(time);
        let next_scaling_index = scaling_index + 1;
        if next_scaling_index >= self.scaling_keys.len() {
            return self.scaling_keys[scaling_index].1;
        }
        let delta_time =
            self.scaling_keys[next_scaling_index].0 - self.scaling_keys[scaling_index].0;
        let factor = (time - self.scaling_keys[scaling_index].0) / delta_time;
        let start = self.scaling_keys[scaling_index].1;
        let end = self.scaling_keys[next_scaling_index].1;
        start + (end - start) * factor
    }
}
