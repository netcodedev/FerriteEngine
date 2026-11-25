//! Sound (docs/audio.md): one-shot sounds and looping emitters, in the
//! world (positional, quieter with distance) or not (menus, music), each
//! on one of the mixer's buses with its own volume. The listener is the
//! scene's camera (`Scene::update` moves it).
//!
//! Built on kira. Without an audio device, on a server (`headless`) and in
//! tests, everything here does nothing.

use std::{
    cell::RefCell,
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

use cgmath::{InnerSpace, Matrix3, Point3, Quaternion, Vector3};
use kira::{
    listener::ListenerHandle,
    sound::{
        static_sound::{StaticSoundData, StaticSoundHandle},
        PlaybackState,
    },
    track::{SpatialTrackBuilder, SpatialTrackDistances, SpatialTrackHandle, TrackBuilder, TrackHandle},
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Easing, PlaybackRate, Tween,
};
use serde::{Deserialize, Serialize};

/// A bus of the mixer: what kind of sound it is, with its own volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Bus {
    /// Things that happen: engines, tools, steps, impacts.
    #[default]
    Effects,
    /// What is always around: wind, fire, birds, rain.
    Ambience,
    Music,
    /// Speech and radio.
    Voice,
}

impl Bus {
    pub const ALL: [Bus; 4] = [Bus::Effects, Bus::Ambience, Bus::Music, Bus::Voice];

    fn index(self) -> usize {
        self as usize
    }
}

/// How a sound is heard in the world: at full volume up to `near` metres
/// from the listener, fading out to nothing at `far`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Range {
    pub near: f32,
    pub far: f32,
}

impl Default for Range {
    fn default() -> Self {
        Self { near: 2.0, far: 60.0 }
    }
}

/// How a sound is played.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub bus: Bus,
    /// Loudness, 0 to 1 (and more, louder than recorded).
    pub volume: f32,
    /// Playback speed and pitch together: 1 as recorded, 2 an octave up.
    pub pitch: f32,
    /// Where in the world, or `None`: not positional (menus, music).
    pub at: Option<Point3<f32>>,
    pub range: Range,
}

impl Default for Play {
    fn default() -> Self {
        Self { bus: Bus::Effects, volume: 1.0, pitch: 1.0, at: None, range: Range::default() }
    }
}

impl Play {
    /// A sound on `bus`, not positional.
    pub fn on(bus: Bus) -> Self {
        Self { bus, ..Self::default() }
    }

    /// A sound at `at` in the world, on the effects bus.
    pub fn at(at: Point3<f32>) -> Self {
        Self { at: Some(at), ..Self::default() }
    }

    pub fn bus(self, bus: Bus) -> Self {
        Self { bus, ..self }
    }

    pub fn volume(self, volume: f32) -> Self {
        Self { volume, ..self }
    }

    pub fn pitch(self, pitch: f32) -> Self {
        Self { pitch, ..self }
    }

    pub fn range(self, near: f32, far: f32) -> Self {
        Self { range: Range { near, far }, ..self }
    }
}

/// Changes of volume and pitch glide over this long, not to click.
const GLIDE: Duration = Duration::from_millis(30);
/// A looping sound stops over this long when its emitter goes.
const FADE_OUT: Duration = Duration::from_millis(120);

fn glide() -> Tween {
    Tween { duration: GLIDE, ..Default::default() }
}

/// A loudness (0 to 1, linear) in decibels as kira takes it.
pub fn decibels(volume: f32) -> Decibels {
    if volume <= 0.001 {
        Decibels::SILENCE
    } else {
        Decibels((20.0 * volume.log10()).max(Decibels::SILENCE.0))
    }
}

/// The listener's orientation as kira takes it (its x axis to the right,
/// y up, looking along -z) for a camera looking along `look`.
pub fn listener_orientation(look: Vector3<f32>) -> Quaternion<f32> {
    let forward = if look.magnitude2() > 1e-8 { look.normalize() } else { Vector3::new(0.0, 0.0, -1.0) };
    let mut right = forward.cross(Vector3::unit_y());
    if right.magnitude2() < 1e-8 {
        // Straight up or down: any right will do.
        right = Vector3::unit_x();
    }
    let right = right.normalize();
    let up = right.cross(forward).normalize();
    Quaternion::from(Matrix3::from_cols(right, up, -forward)).normalize()
}

fn mint_point(p: Point3<f32>) -> mint::Vector3<f32> {
    mint::Vector3 { x: p.x, y: p.y, z: p.z }
}

struct Audio {
    manager: AudioManager<DefaultBackend>,
    listener: ListenerHandle,
    buses: Vec<TrackHandle>,
    /// Loaded sounds by path (`None`: it would not load, said once).
    sounds: HashMap<PathBuf, Option<StaticSoundData>>,
    /// Sounds played since the start, and emitters there are now (for the
    /// editor's status).
    played: u64,
    emitters: usize,
}

impl Audio {
    fn start() -> Option<Self> {
        if crate::core::headless::is_headless() || cfg!(test) {
            return None;
        }
        let mut manager = match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
            Ok(manager) => manager,
            Err(e) => {
                log::warn!("audio: no sound ({e})");
                return None;
            }
        };
        let listener = manager
            .add_listener(mint::Vector3 { x: 0.0, y: 0.0, z: 0.0 }, mint::Quaternion { s: 1.0, v: mint::Vector3 { x: 0.0, y: 0.0, z: 0.0 } })
            .ok()?;
        let mut buses = Vec::new();
        for _ in Bus::ALL {
            buses.push(manager.add_sub_track(TrackBuilder::new()).ok()?);
        }
        log::info!("audio: on");
        Some(Self { manager, listener, buses, sounds: HashMap::new(), played: 0, emitters: 0 })
    }

    fn sound(&mut self, path: &Path) -> Option<StaticSoundData> {
        self.sounds
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let loaded = crate::core::assets::vfs::read(path)
                    .and_then(|bytes| StaticSoundData::from_cursor(Cursor::new(bytes)).map_err(|e| e.to_string()));
                loaded.map_err(|e| log::warn!("audio: {}: {e}", path.display())).ok()
            })
            .clone()
    }

    /// `data` played as `play` says: on a track of its own in the world
    /// (which goes once the sound is over) or on its bus.
    fn start_sound(&mut self, data: StaticSoundData, play: &Play) -> Option<(StaticSoundHandle, Option<SpatialTrackHandle>)> {
        let data = data.volume(decibels(play.volume)).playback_rate(PlaybackRate(play.pitch.max(0.01) as f64));
        let listener = self.listener.id();
        let bus = &mut self.buses[play.bus.index()];
        self.played += 1;
        match play.at {
            Some(at) => {
                let builder = SpatialTrackBuilder::new()
                    .distances(SpatialTrackDistances { min_distance: play.range.near.max(0.01), max_distance: play.range.far.max(play.range.near + 0.01) })
                    .attenuation_function(Some(Easing::OutPowi(2)))
                    .persist_until_sounds_finish(true);
                let mut track = bus.add_spatial_sub_track(listener, mint_point(at), builder).ok()?;
                let sound = track.play(data).ok()?;
                Some((sound, Some(track)))
            }
            None => Some((bus.play(data).ok()?, None)),
        }
    }
}

thread_local! {
    /// The audio of this process: not started yet (`None`), or started,
    /// with sound or without.
    static AUDIO: RefCell<Option<Option<Audio>>> = const { RefCell::new(None) };
}

fn with_audio<R>(f: impl FnOnce(&mut Audio) -> R) -> Option<R> {
    AUDIO.with(|audio| {
        let mut audio = audio.borrow_mut();
        audio.get_or_insert_with(Audio::start).as_mut().map(f)
    })
}

/// `f` on the audio if it was started already (by a sound), else nothing:
/// following the camera alone opens no device.
fn if_started<R>(f: impl FnOnce(&mut Audio) -> R) -> Option<R> {
    AUDIO.with(|audio| audio.borrow_mut().as_mut().and_then(|audio| audio.as_mut()).map(f))
}

/// Whether there is sound (a device, not a server or a test).
pub fn available() -> bool {
    with_audio(|_| ()).is_some()
}

/// The volume (0 to 1) of `bus`, or with `None` of everything.
pub fn set_volume(bus: Option<Bus>, volume: f32) {
    with_audio(|audio| match bus {
        Some(bus) => audio.buses[bus.index()].set_volume(decibels(volume), glide()),
        None => audio.manager.main_track().set_volume(decibels(volume), glide()),
    });
}

/// Where the listener is and where it looks (the engine follows the
/// scene's camera; a script may move it after).
pub fn set_listener(position: Point3<f32>, look: Vector3<f32>) {
    if_started(|audio| {
        let q = listener_orientation(look);
        audio.listener.set_position(mint_point(position), Tween::default());
        audio.listener.set_orientation(mint::Quaternion { s: q.s, v: mint::Vector3 { x: q.v.x, y: q.v.y, z: q.v.z } }, Tween::default());
    });
}

/// Play the sound file `path` (project relative: .ogg, .wav, .flac, .mp3)
/// once, as `play` says.
pub fn play(path: impl AsRef<Path>, play: Play) {
    with_audio(|audio| {
        if let Some(data) = audio.sound(path.as_ref()) {
            audio.start_sound(data, &play);
        }
    });
}

/// A sound playing over and over (an engine, a generator, a fire) for as
/// long as it is kept; it fades out when dropped. Its volume, pitch and
/// place can change every frame.
pub struct Emitter {
    sound: Option<StaticSoundHandle>,
    track: Option<SpatialTrackHandle>,
}

impl Emitter {
    /// The sound file `path` looping, as `play` says. Without sound (or
    /// when the file does not load) an emitter that does nothing.
    pub fn new(path: impl AsRef<Path>, play: Play) -> Self {
        let started = with_audio(|audio| {
            let data = audio.sound(path.as_ref())?.loop_region(..);
            let started = audio.start_sound(data, &play)?;
            audio.emitters += 1;
            Some(started)
        })
        .flatten();
        match started {
            Some((sound, track)) => Self { sound: Some(sound), track },
            None => Self { sound: None, track: None },
        }
    }

    /// Whether it makes a sound (there is audio and its file loaded).
    pub fn is_playing(&self) -> bool {
        self.sound.as_ref().is_some_and(|sound| sound.state() == PlaybackState::Playing)
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(sound) = &mut self.sound {
            sound.set_volume(decibels(volume), glide());
        }
    }

    pub fn set_pitch(&mut self, pitch: f32) {
        if let Some(sound) = &mut self.sound {
            sound.set_playback_rate(PlaybackRate(pitch.max(0.01) as f64), glide());
        }
    }

    /// Where it is (a positional emitter; others are not moved).
    pub fn set_position(&mut self, at: Point3<f32>) {
        if let Some(track) = &mut self.track {
            track.set_position(mint_point(at), glide());
        }
    }
}

impl Drop for Emitter {
    fn drop(&mut self) {
        if let Some(mut sound) = self.sound.take() {
            sound.stop(Tween { duration: FADE_OUT, ..Default::default() });
            with_audio(|audio| audio.emitters = audio.emitters.saturating_sub(1));
        }
    }
}

/// What the audio does now: whether there is sound, how many sounds were
/// played since the start and how many emitters there are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    pub available: bool,
    pub played: u64,
    pub emitters: usize,
}

pub fn status() -> Status {
    with_audio(|audio| Status { available: true, played: audio.played, emitters: audio.emitters }).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::Rotation;

    #[test]
    fn loudness_goes_to_decibels() {
        assert_eq!(decibels(1.0).0, 0.0);
        assert!((decibels(0.5).0 + 6.02).abs() < 0.01);
        assert_eq!(decibels(0.0), Decibels::SILENCE);
    }

    /// Kira hears with the listener's x axis to the right and looks along
    /// -z: a camera looking along +x has its right ear towards +z.
    #[test]
    fn the_listener_looks_where_the_camera_looks() {
        let q = listener_orientation(Vector3::new(1.0, 0.0, 0.0));
        let right = q.rotate_vector(Vector3::unit_x());
        let ahead = q.rotate_vector(-Vector3::unit_z());
        assert!((ahead - Vector3::new(1.0, 0.0, 0.0)).magnitude() < 1e-4, "{ahead:?}");
        assert!((right - Vector3::new(0.0, 0.0, 1.0)).magnitude() < 1e-4, "{right:?}");
        // Straight down still works.
        let q = listener_orientation(Vector3::new(0.0, -1.0, 0.0));
        assert!((q.rotate_vector(-Vector3::unit_z()) - Vector3::new(0.0, -1.0, 0.0)).magnitude() < 1e-4);
    }

    /// Without sound (tests have none) everything is quiet and harmless.
    #[test]
    fn without_sound_nothing_happens() {
        assert!(!available());
        play("nothing.ogg", Play::at(Point3::new(1.0, 2.0, 3.0)));
        let mut emitter = Emitter::new("nothing.ogg", Play::default());
        emitter.set_pitch(1.5);
        assert!(!emitter.is_playing());
        assert_eq!(status(), Status::default());
    }
}
