use std::collections::HashMap;

use russimp::scene::Scene;

use super::{Animation, Channel, Pose};

impl Animation {
    pub fn new(animation: &russimp::animation::Animation) -> Animation {
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

    pub fn from_file(path: &str) -> Result<Animation, Box<dyn std::error::Error>> {
        let scene = Scene::from_file(format!("assets/animations/{path}").as_str(), vec![])?;
        if scene.animations.len() == 0 {
            return Err("No animations found".into());
        }
        Ok(Animation::new(&scene.animations[0]))
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
