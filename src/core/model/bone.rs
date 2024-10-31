use std::collections::HashMap;

use cgmath::{Matrix4, Quaternion, SquareMatrix, Vector3, Zero};

use super::{Bone, Channel};

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

    pub fn reset(&mut self) {
        self.current_animations = Vec::new();
        self.current_animation_time = Vec::new();
        if let Some(children) = &mut self.children {
            for child in children {
                child.reset();
            }
        }
    }

    pub fn set_animation_channel(
        &mut self,
        channels: Option<&HashMap<String, Channel>>,
        weight: f32,
        time: f32,
    ) {
        if let Some(channels) = channels {
            if let Some(channel) = channels.get(&self.name) {
                self.current_animations.push((weight, channel.clone()));
                self.current_animation_time.push(0.0);
            } else {
                self.current_animations = Vec::new();
                self.current_animation_time = Vec::new();
            }
            if let Some(children) = &mut self.children {
                for child in children {
                    child.set_animation_channel(Some(channels), weight, time);
                }
            }
        } else {
            self.current_animations = Vec::new();
            self.current_animation_time = Vec::new();
            if let Some(children) = &mut self.children {
                for child in children {
                    child.set_animation_channel(None, 1.0, 0.0);
                }
            }
        }
    }

    fn get_position_index(&self, index: usize, time: f32) -> usize {
        if let Some((_, animation)) = &self.current_animations.get(index) {
            for i in 0..animation.position_keys.len() {
                if animation.position_keys[i].0 > time {
                    return i - 1;
                }
            }
        }
        0
    }

    fn get_rotation_index(&self, index: usize, time: f32) -> usize {
        if let Some((_, animation)) = &self.current_animations.get(index) {
            for i in 0..animation.rotation_keys.len() {
                if animation.rotation_keys[i].0 > time {
                    return i - 1;
                }
            }
        }
        0
    }

    fn get_scaling_index(&self, index: usize, time: f32) -> usize {
        if let Some((_, animation)) = &self.current_animations.get(index) {
            for i in 0..animation.scaling_keys.len() {
                if animation.scaling_keys[i].0 > time {
                    return i - 1;
                }
            }
        }
        0
    }

    fn interpolate_position(&self, index: usize) -> Vector3<f32> {
        let time = self.current_animation_time[index];
        if let Some((_, animation)) = &self.current_animations.get(index) {
            let position_index = self.get_position_index(index, time);
            let next_position_index = position_index + 1;
            if next_position_index >= animation.position_keys.len() {
                return animation.position_keys[position_index].1;
            }
            let delta_time = animation.position_keys[next_position_index].0
                - animation.position_keys[position_index].0;
            let factor = (time - animation.position_keys[position_index].0) / delta_time;
            let start = animation.position_keys[position_index].1;
            let end = animation.position_keys[next_position_index].1;
            start + (end - start) * factor
        } else {
            Vector3::zero()
        }
    }

    fn interpolate_rotation(&self, index: usize) -> Quaternion<f32> {
        let time = self.current_animation_time[index];
        if let Some((_, animation)) = &self.current_animations.get(index) {
            let rotation_index = self.get_rotation_index(index, time);
            let next_rotation_index = rotation_index + 1;
            if next_rotation_index >= animation.rotation_keys.len() {
                return animation.rotation_keys[rotation_index].1;
            }
            let delta_time = animation.rotation_keys[next_rotation_index].0
                - animation.rotation_keys[rotation_index].0;
            let factor = (time - animation.rotation_keys[rotation_index].0) / delta_time;
            let start = animation.rotation_keys[rotation_index].1;
            let end = animation.rotation_keys[next_rotation_index].1;
            Quaternion::slerp(start, end, factor)
        } else {
            Quaternion::zero()
        }
    }

    fn interpolate_scaling(&self, index: usize) -> Vector3<f32> {
        let time = self.current_animation_time[index];
        if let Some((_, animation)) = &self.current_animations.get(index) {
            let scaling_index = self.get_scaling_index(index, time);
            let next_scaling_index = scaling_index + 1;
            if next_scaling_index >= animation.scaling_keys.len() {
                return animation.scaling_keys[scaling_index].1;
            }
            let delta_time = animation.scaling_keys[next_scaling_index].0
                - animation.scaling_keys[scaling_index].0;
            let factor = (time - animation.scaling_keys[scaling_index].0) / delta_time;
            let start = animation.scaling_keys[scaling_index].1;
            let end = animation.scaling_keys[next_scaling_index].1;
            start + (end - start) * factor
        } else {
            Vector3::new(1.0, 1.0, 1.0)
        }
    }

    pub fn update_animation(
        &mut self,
        animation_data: Vec<(f32, f32)>,
        sync: bool,
        is_root: bool,
    ) -> Vector3<f32> {
        let mut final_transform = (
            Vector3::zero(),
            Quaternion::zero(),
            Vector3::new(0.0, 0.0, 0.0),
        );
        let mut progression = 0.0;
        let mut cycle_completed = false;
        for (i, (weight, _)) in self.current_animations.iter().enumerate() {
            if sync && i > 0 {
                self.current_animation_time[i] = progression * animation_data[i].1;
            } else {
                self.current_animation_time[i] += animation_data[i].0;
                if self.current_animation_time[i] >= animation_data[i].1 {
                    cycle_completed = true;
                    self.current_animation_time[i] %= animation_data[i].1;
                }
                progression = self.current_animation_time[i] / animation_data[i].1;
            }
            let translation = self.interpolate_position(i);
            let rotation = self.interpolate_rotation(i);
            let scaling = self.interpolate_scaling(i);
            final_transform.0 += translation * *weight;
            if final_transform.1 == Quaternion::zero() {
                final_transform.1 = rotation;
            } else {
                final_transform.1 = Quaternion::slerp(final_transform.1, rotation, *weight);
            }
            final_transform.2 += scaling * *weight;
        }
        self.current_transform = if is_root {
            Matrix4::identity()
        } else {
            Matrix4::from_translation(final_transform.0)
        } * Matrix4::from(final_transform.1)
            * Matrix4::from_nonuniform_scale(
                final_transform.2.x,
                final_transform.2.y,
                final_transform.2.z,
            );
        if let Some(children) = &mut self.children {
            for child in children.iter_mut() {
                child.update_animation(animation_data.clone(), sync, false);
            }
        }
        if cycle_completed {
            self.last_translation = final_transform.0;
        }
        let delta = final_transform.0 - self.last_translation;
        if !cycle_completed {
            self.last_translation = final_transform.0;
        }
        delta
    }
}
