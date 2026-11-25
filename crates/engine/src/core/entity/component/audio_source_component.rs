//! A sound in a scene (docs/audio.md): a sound file playing where its
//! entity is (a campfire's crackle, a river, a generator), looping or once
//! when the game starts. Silent while the scene is edited.

use cgmath::Point3;
use glfw::{Glfw, WindowEvent};
use serde::{Deserialize, Serialize};

use crate::core::{
    assets::{self, AssetKind},
    audio::{self, Bus, Emitter, Play, Range},
    entity::{
        component::{
            property::{Property, PropertyValue},
            Component,
        },
        Entity,
    },
    scene::{scene_file::ComponentData, Scene},
};

/// What an audio source plays and how: kept in the scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioSourceSettings {
    /// The sound file, project relative.
    #[serde(default)]
    pub sound: String,
    #[serde(default)]
    pub bus: Bus,
    #[serde(default = "one")]
    pub volume: f32,
    #[serde(default = "one")]
    pub pitch: f32,
    #[serde(default)]
    pub range: Range,
    /// Over and over (else once when the game starts).
    #[serde(default = "yes")]
    pub looping: bool,
    /// Heard from where the entity is (else everywhere alike: music).
    #[serde(default = "yes")]
    pub positional: bool,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

impl Default for AudioSourceSettings {
    fn default() -> Self {
        Self {
            sound: String::new(),
            bus: Bus::Ambience,
            volume: 1.0,
            pitch: 1.0,
            range: Range::default(),
            looping: true,
            positional: true,
        }
    }
}

impl AudioSourceSettings {
    fn play(&self, at: Point3<f32>) -> Play {
        Play {
            bus: self.bus,
            volume: self.volume,
            pitch: self.pitch,
            at: self.positional.then_some(at),
            range: self.range,
        }
    }
}

pub struct AudioSource {
    pub settings: AudioSourceSettings,
    emitter: Option<Emitter>,
    /// A sound played once was played.
    played: bool,
}

impl AudioSource {
    pub fn new(settings: AudioSourceSettings) -> Self {
        Self { settings, emitter: None, played: false }
    }

    /// Start again with the settings as they are now.
    fn restart(&mut self) {
        self.emitter = None;
        self.played = false;
    }
}

/// The sound files of the project, project relative, for the inspector.
fn sound_files() -> Vec<String> {
    let here = std::env::current_dir().unwrap_or_default();
    let database = assets::database();
    let mut files: Vec<String> = database
        .iter()
        .filter(|entry| matches!(entry.meta.kind, AssetKind::Sound(_)))
        .map(|entry| {
            let path = entry.source_path();
            path.strip_prefix(&here).unwrap_or(&path).to_string_lossy().replace('\\', "/")
        })
        .collect();
    files.sort();
    files
}

const BUS_NAMES: [&str; 4] = ["Effects", "Ambience", "Music", "Voice"];

impl Component for AudioSource {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, _: f64) {
        if self.settings.sound.is_empty() {
            return;
        }
        let at = scene
            .get_world_transform(&entity.id)
            .map_or(entity.get_position(), |m| Point3::new(m.w.x, m.w.y, m.w.z));
        if self.settings.looping {
            let settings = &self.settings;
            let emitter = self.emitter.get_or_insert_with(|| Emitter::new(&settings.sound, settings.play(at)));
            if self.settings.positional {
                emitter.set_position(at);
            }
        } else if !self.played {
            self.played = true;
            audio::play(&self.settings.sound, self.settings.play(at));
        }
    }

    fn update_in_editor(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {
        // Edited: quiet (a game that stopped leaves nothing playing).
        self.restart();
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        "Audio source".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(ComponentData::AudioSource { settings: self.settings.clone() })
    }

    fn properties(&self) -> Vec<Property> {
        let s = &self.settings;
        let mut sounds = sound_files();
        if !s.sound.is_empty() && !sounds.contains(&s.sound) {
            sounds.insert(0, s.sound.clone());
        }
        sounds.insert(0, String::new());
        vec![
            Property::choice("sound", s.sound.clone(), sounds),
            Property::choice("bus", BUS_NAMES[s.bus as usize].into(), BUS_NAMES.iter().map(|b| b.to_string()).collect()),
            Property::ranged("volume", s.volume, 0.0, 2.0),
            Property::ranged("pitch", s.pitch, 0.25, 4.0),
            Property::bool("looping", s.looping),
            Property::bool("positional", s.positional),
            Property::ranged("near", s.range.near, 0.0, 50.0),
            Property::ranged("far", s.range.far, 1.0, 1000.0),
        ]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let s = &mut self.settings;
        match name {
            "sound" => {
                if let Some(sound) = value.as_text() {
                    s.sound = sound.to_string();
                }
            }
            "bus" => {
                if let Some(i) = value.as_text().and_then(|b| BUS_NAMES.iter().position(|n| n.eq_ignore_ascii_case(b))) {
                    s.bus = Bus::ALL[i];
                }
            }
            "volume" => s.volume = value.as_f32().unwrap_or(s.volume).max(0.0),
            "pitch" => s.pitch = value.as_f32().unwrap_or(s.pitch).max(0.01),
            "looping" => s.looping = value.as_bool().unwrap_or(s.looping),
            "positional" => s.positional = value.as_bool().unwrap_or(s.positional),
            "near" => s.range.near = value.as_f32().unwrap_or(s.range.near).max(0.0),
            "far" => s.range.far = value.as_f32().unwrap_or(s.range.far).max(s.range.near + 0.1),
            _ => return,
        }
        self.restart();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An audio source goes into the scene file and comes back the same;
    /// left-out fields take their defaults.
    #[test]
    fn it_round_trips_through_the_scene_file() {
        let settings = AudioSourceSettings { sound: "assets/sounds/fire.ogg".into(), volume: 0.6, ..Default::default() };
        let data = ComponentData::AudioSource { settings: settings.clone() };
        let text = ron::to_string(&data).unwrap();
        assert_eq!(ron::from_str::<ComponentData>(&text).unwrap(), data);
        let short: ComponentData = ron::from_str(r#"AudioSource(settings: (sound: "a.ogg"))"#).unwrap();
        let ComponentData::AudioSource { settings } = short else { panic!() };
        assert!(settings.looping && settings.positional && settings.volume == 1.0);
    }

    #[test]
    fn the_inspector_changes_it() {
        let mut source = AudioSource::new(AudioSourceSettings::default());
        source.set_property("bus", PropertyValue::Text("Music".into()));
        source.set_property("volume", PropertyValue::Float(0.25));
        source.set_property("looping", PropertyValue::Bool(false));
        assert_eq!(source.settings.bus, Bus::Music);
        assert_eq!(source.settings.volume, 0.25);
        assert!(!source.settings.looping);
    }
}
