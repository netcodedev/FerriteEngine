use std::{collections::HashMap, path::Path};

use russimp_ng::scene::Scene;

use super::{outline, Animation, Channel, Pose};

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

    pub fn get_progression(&self, time: f32) -> f32 {
        time % self.duration / self.duration
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }
}
