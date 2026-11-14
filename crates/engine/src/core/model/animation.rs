use std::{collections::HashMap, f32::consts::PI, path::Path, rc::Rc};

use cgmath::{Quaternion, Rad, Rotation, Rotation3, Vector3};
use russimp_ng::{node::Node, scene::Scene};

use super::{outline, Animation, Channel, Pose};

/// `angle` in (-π, π].
fn wrap(angle: f32) -> f32 {
    let a = (angle + PI).rem_euclid(2.0 * PI) - PI;
    if a <= -PI { a + 2.0 * PI } else { a }
}

/// Yaw (radians about +Y) of a rotation: where it turns +Z to.
fn yaw_of(rotation: Quaternion<f32>) -> f32 {
    let forward = rotation.rotate_vector(Vector3::unit_z());
    forward.x.atan2(forward.z)
}

/// The first node, breadth first, that `is_channel`.
fn topmost(root: &Rc<Node>, is_channel: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut queue = std::collections::VecDeque::from([root.clone()]);
    while let Some(node) = queue.pop_front() {
        if is_channel(&node.name) {
            return Some(node.name.clone());
        }
        queue.extend(node.children.borrow().iter().cloned());
    }
    None
}

impl Animation {
    pub fn new(animation: &russimp_ng::animation::Animation) -> Animation {
        let mut channels = HashMap::<String, Channel>::new();
        for channel in &animation.channels {
            let channel = Channel::new(channel);
            channels.insert(channel.bone_id.clone(), channel);
        }
        Animation {
            name: animation.name.clone(),
            duration: animation.duration as f32,
            ticks_per_second: animation.ticks_per_second as f32,
            channels,
            root_channel: None,
        }
    }

    /// Load the first clip of a file in `assets/animations/`.
    pub fn from_file(name: &str, path: &str) -> Result<Animation, Box<dyn std::error::Error>> {
        Self::load(name, Path::new(&format!("assets/animations/{path}")), None)
    }

    /// Load a clip from a file anywhere: the one called `clip` (as listed
    /// by `outline::clip_names`), or the first without a name.
    pub fn load(
        name: &str,
        source: &Path,
        clip: Option<&str>,
    ) -> Result<Animation, Box<dyn std::error::Error>> {
        if super::clip_file::is_clip_file(source) {
            return Ok(super::clip_file::ClipFile::load(source)?.to_animation(name));
        }
        let scene = Scene::from_file(&source.to_string_lossy(), vec![])?;
        if scene.animations.is_empty() {
            return Err("No animations found".into());
        }
        let index = match clip {
            None => 0,
            Some(wanted) => outline::clip_names(&scene)
                .iter()
                .position(|found| found == wanted)
                .ok_or_else(|| format!("the file has no clip '{wanted}'"))?,
        };
        let mut animation = Animation::new(&scene.animations[index]);
        animation.set_name(name);
        if let Some(root) = &scene.root {
            let channels = &animation.channels;
            animation.root_channel = topmost(root, &|name| channels.contains_key(name));
        }
        Ok(animation)
    }

    pub fn sample(&self, time: f32) -> Pose {
        let mut pose = Pose::new();
        if time > self.duration {
            println!("Cycle completed");
            pose.cycle_completed = true;
        }
        let sample_time = time % self.duration;
        for (bone_id, channel) in &self.channels {
            pose.add_transform(bone_id.to_string(), channel.sample(sample_time));
        }
        pose
    }

    /// Yaw (radians about the up axis) of the root bone at `time` (ticks),
    /// relative to the clip's start.
    pub fn root_yaw(&self, time: f32) -> Option<f32> {
        let channel = self.channels.get(self.root_channel.as_ref()?)?;
        let start = yaw_of(channel.sample(0.0).rotation);
        Some(wrap(yaw_of(channel.sample(time).rotation) - start))
    }

    /// How far the root bone turns about the up axis from `from` to `to`
    /// (ticks); `cycled` when the clip looped in between.
    pub fn root_yaw_change(&self, from: f32, to: f32, cycled: bool) -> f32 {
        let yaw = |t: f32| self.root_yaw(t).unwrap_or(0.0);
        if cycled {
            wrap(yaw(self.duration) - yaw(from)) + wrap(yaw(to) - yaw(0.0))
        } else {
            wrap(yaw(to) - yaw(from))
        }
    }

    /// Like `sample`, with the root bone's turn about the up axis since the
    /// clip's start taken out (it turns the body instead; see
    /// `root_yaw_change`).
    pub fn sample_without_root_yaw(&self, time: f32) -> Pose {
        let mut pose = self.sample(time);
        if let (Some(root), Some(yaw)) = (&self.root_channel, self.root_yaw(time % self.duration)) {
            if let Some(transform) = pose.transforms.get_mut(root) {
                transform.rotation = Quaternion::from_angle_y(Rad(-yaw)) * transform.rotation;
            }
        }
        pose
    }

    pub fn get_progression(&self, time: f32) -> f32 {
        time % self.duration / self.duration
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip whose hips turn from 0 to 90° about the up axis in 10 ticks.
    fn turn() -> Animation {
        let rotation = |deg: f32| Quaternion::from_angle_y(cgmath::Deg(deg));
        let channel = Channel {
            bone_id: "rig:Hips".into(),
            position_keys: vec![(0.0, Vector3::new(0.0, 1.0, 0.0))],
            rotation_keys: vec![(0.0, rotation(0.0)), (10.0, rotation(90.0))],
            scaling_keys: vec![(0.0, Vector3::new(1.0, 1.0, 1.0))],
        };
        Animation {
            name: "turn".into(),
            duration: 10.0,
            ticks_per_second: 30.0,
            channels: HashMap::from([("rig:Hips".to_string(), channel)]),
            root_channel: Some("rig:Hips".into()),
        }
    }

    #[test]
    fn the_root_turn_is_measured_across_a_loop() {
        let clip = turn();
        let deg = |rad: f32| rad.to_degrees();
        assert!((deg(clip.root_yaw_change(0.0, 5.0, false)) - 45.0).abs() < 0.5);
        // From 8 to the end (18°) and from the start to 2 (18°).
        assert!((deg(clip.root_yaw_change(8.0, 2.0, true)) - 36.0).abs() < 0.5);
    }

    #[test]
    fn the_pose_does_not_show_the_root_turn() {
        let clip = turn();
        let pose = clip.sample_without_root_yaw(5.0);
        let yaw = yaw_of(pose.transforms["rig:Hips"].rotation).to_degrees();
        assert!(yaw.abs() < 0.5, "{yaw}");
        let turned = yaw_of(clip.sample(5.0).transforms["rig:Hips"].rotation).to_degrees();
        assert!((turned - 45.0).abs() < 0.5, "{turned}");
    }

    #[test]
    fn angles_wrap_into_a_half_turn_either_way() {
        assert!((wrap(1.5 * PI) + 0.5 * PI).abs() < 1e-5);
        assert!((wrap(-1.5 * PI) - 0.5 * PI).abs() < 1e-5);
        assert!((wrap(0.3) - 0.3).abs() < 1e-6);
    }
}
