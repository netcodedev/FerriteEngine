use cgmath::{Quaternion, Vector3};

use super::Channel;


impl Channel {
    pub fn new(channel: &russimp::animation::NodeAnim) -> Channel {
        let mut position_keys = Vec::<(f32, cgmath::Vector3<f32>)>::new();
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
        let mut scaling_keys = Vec::<(f32, cgmath::Vector3<f32>)>::new();
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
}
