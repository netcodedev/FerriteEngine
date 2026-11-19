//! Animation clips made in the editor (`*.clip.ron`): keyframes per bone,
//! stored as data so tools can write them and the engine can play them like
//! any clip from a model file.

use std::{collections::HashMap, error::Error, fs, path::Path};

use cgmath::{InnerSpace, Matrix3, Matrix4, Quaternion, Vector3};
use serde::{Deserialize, Serialize};

use crate::core::assets::AssetId;

use super::{Animation, Channel};

pub const CLIP_FILE_VERSION: u32 = 1;
/// What the name of a clip file ends with.
pub const CLIP_SUFFIX: &str = ".clip.ron";
/// Keys closer together than this (seconds) are the same key.
pub const KEY_EPSILON: f32 = 1e-3;
/// Shortest a clip can be (seconds).
pub const MIN_DURATION: f32 = 0.05;

/// Whether `path` names a clip file.
pub fn is_clip_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase().ends_with(CLIP_SUFFIX))
}

/// The clip's name: its file name without `.clip.ron`.
pub fn clip_name(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match name.to_lowercase().rfind(CLIP_SUFFIX) {
        Some(at) if at > 0 => name[..at].to_string(),
        _ => name,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipFile {
    pub version: u32,
    /// The model asset the clip was posed on (for the editor's preview).
    #[serde(default)]
    pub model: Option<AssetId>,
    /// Length in seconds; the clip loops after it.
    pub duration: f32,
    #[serde(default)]
    pub tracks: Vec<BoneTrack>,
    /// The pose of bones without keys (the model's rest pose when saved),
    /// so the clip poses the whole body.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rest: Vec<BonePose>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BoneTrack {
    pub bone: String,
    /// Sorted by time.
    pub keys: Vec<BoneKey>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BonePose {
    pub bone: String,
    pub pose: Transform,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct BoneKey {
    /// Seconds from the start of the clip.
    pub time: f32,
    pub pose: Transform,
}

/// A bone's transform relative to its parent.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Quaternion as (x, y, z, w).
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Transform {
    pub fn from_matrix(m: &Matrix4<f32>) -> Self {
        let x = m.x.truncate();
        let y = m.y.truncate();
        let z = m.z.truncate();
        let scale = [x.magnitude(), y.magnitude(), z.magnitude()];
        let unit = |v: Vector3<f32>, s: f32| if s > 1e-8 { v / s } else { v };
        let rotation = Matrix3::from_cols(unit(x, scale[0]), unit(y, scale[1]), unit(z, scale[2]));
        let q = Quaternion::from(rotation).normalize();
        Self {
            translation: [m.w.x, m.w.y, m.w.z],
            rotation: [q.v.x, q.v.y, q.v.z, q.s],
            scale,
        }
    }

    fn is_finite(&self) -> bool {
        self.translation
            .iter()
            .chain(&self.rotation)
            .chain(&self.scale)
            .all(|v| v.is_finite())
    }

    /// The rotation scaled to unit length; none at all without length.
    fn normalize_rotation(&mut self) {
        let q = self.quaternion();
        let q = if q.magnitude2() > 1e-12 {
            q.normalize()
        } else {
            Quaternion::new(1.0, 0.0, 0.0, 0.0)
        };
        self.rotation = [q.v.x, q.v.y, q.v.z, q.s];
    }

    pub fn quaternion(&self) -> Quaternion<f32> {
        let [x, y, z, w] = self.rotation;
        Quaternion::new(w, x, y, z)
    }

    pub fn to_matrix(&self) -> Matrix4<f32> {
        let [tx, ty, tz] = self.translation;
        let [sx, sy, sz] = self.scale;
        Matrix4::from_translation(Vector3::new(tx, ty, tz))
            * Matrix4::from(self.quaternion())
            * Matrix4::from_nonuniform_scale(sx, sy, sz)
    }

    /// Rotations are slerped (the shorter way), the rest is blended
    /// linearly.
    pub fn interpolate(&self, other: &Transform, factor: f32) -> Transform {
        let lerp3 = |a: [f32; 3], b: [f32; 3]| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * factor);
        let q = self
            .quaternion()
            .slerp(other.quaternion(), factor)
            .normalize();
        Transform {
            translation: lerp3(self.translation, other.translation),
            rotation: [q.v.x, q.v.y, q.v.z, q.s],
            scale: lerp3(self.scale, other.scale),
        }
    }
}

impl BoneTrack {
    /// The pose at `time`: between keys interpolated, before the first and
    /// after the last key held. `None` without keys.
    pub fn sample(&self, time: f32) -> Option<Transform> {
        let first = self.keys.first()?;
        if time <= first.time {
            return Some(first.pose);
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if time < b.time {
                let span = (b.time - a.time).max(1e-6);
                return Some(a.pose.interpolate(&b.pose, (time - a.time) / span));
            }
        }
        self.keys.last().map(|key| key.pose)
    }

    pub fn key_at(&self, time: f32) -> Option<usize> {
        self.keys
            .iter()
            .position(|key| (key.time - time).abs() < KEY_EPSILON)
    }
}

impl ClipFile {
    pub fn new(model: Option<AssetId>, duration: f32) -> Self {
        Self {
            version: CLIP_FILE_VERSION,
            model,
            duration: duration.max(MIN_DURATION),
            tracks: Vec::new(),
            rest: Vec::new(),
        }
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let clip = Self::from_ron(&fs::read_to_string(path)?)?;
        if clip.version > CLIP_FILE_VERSION {
            return Err(format!(
                "clip version {} is newer than supported version {CLIP_FILE_VERSION}",
                clip.version
            )
            .into());
        }
        let mut clip = clip;
        clip.sanitize();
        Ok(clip)
    }

    /// Make a file edited by hand or merged playable: keys sorted by time,
    /// keys and rest poses that are no numbers dropped, rotations of unit
    /// length (others shear or scale the bone).
    fn sanitize(&mut self) {
        if !self.duration.is_finite() {
            self.duration = MIN_DURATION;
        }
        for track in &mut self.tracks {
            track.keys.retain(|key| key.time.is_finite() && key.pose.is_finite());
            track.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            for key in &mut track.keys {
                key.pose.normalize_rotation();
            }
        }
        self.tracks.retain(|track| !track.keys.is_empty());
        self.rest.retain(|rest| rest.pose.is_finite());
        for rest in &mut self.rest {
            rest.pose.normalize_rotation();
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        fs::write(path, self.to_ron()?)?;
        Ok(())
    }

    pub fn track(&self, bone: &str) -> Option<&BoneTrack> {
        self.tracks.iter().find(|t| t.bone == bone)
    }

    /// Set (or replace) the key of `bone` at `time`.
    pub fn set_key(&mut self, bone: &str, time: f32, pose: Transform) {
        let time = time.clamp(0.0, self.duration);
        let index = match self.tracks.iter().position(|t| t.bone == bone) {
            Some(index) => index,
            None => {
                self.tracks.push(BoneTrack {
                    bone: bone.to_string(),
                    keys: Vec::new(),
                });
                self.tracks.len() - 1
            }
        };
        let track = &mut self.tracks[index];
        match track.key_at(time) {
            Some(at) => track.keys[at].pose = pose,
            None => {
                let at = track.keys.partition_point(|key| key.time < time);
                track.keys.insert(at, BoneKey { time, pose });
            }
        }
    }

    /// Remove the key of `bone` at `time`; a track left without keys goes.
    /// Returns whether there was one.
    pub fn remove_key(&mut self, bone: &str, time: f32) -> bool {
        let Some(index) = self.tracks.iter().position(|t| t.bone == bone) else {
            return false;
        };
        let track = &mut self.tracks[index];
        let Some(at) = track.key_at(time) else {
            return false;
        };
        track.keys.remove(at);
        if track.keys.is_empty() {
            self.tracks.remove(index);
        }
        true
    }

    /// Move every key at `from` (any bone) to `to`; keys already at `to`
    /// are replaced.
    pub fn move_keys(&mut self, from: f32, to: f32) {
        let to = to.clamp(0.0, self.duration);
        let moved: Vec<(String, Transform)> = self
            .tracks
            .iter()
            .filter_map(|t| t.key_at(from).map(|at| (t.bone.clone(), t.keys[at].pose)))
            .collect();
        for (bone, pose) in moved {
            self.remove_key(&bone, from);
            self.set_key(&bone, to, pose);
        }
    }

    /// Every time that has a key, sorted, without duplicates.
    pub fn key_times(&self) -> Vec<f32> {
        let mut times: Vec<f32> = self
            .tracks
            .iter()
            .flat_map(|t| t.keys.iter().map(|k| k.time))
            .collect();
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < KEY_EPSILON);
        times
    }

    /// Change the length; keys after the new end are dropped.
    pub fn set_duration(&mut self, duration: f32) {
        self.duration = duration.max(MIN_DURATION);
        let end = self.duration + KEY_EPSILON;
        for track in &mut self.tracks {
            track.keys.retain(|key| key.time <= end);
        }
        self.tracks.retain(|t| !t.keys.is_empty());
    }

    /// The pose of every bone the clip moves at `time`: keyed bones
    /// interpolated, the others at their rest pose.
    pub fn sample(&self, time: f32) -> HashMap<String, Transform> {
        let mut pose: HashMap<String, Transform> = self
            .rest
            .iter()
            .map(|rest| (rest.bone.clone(), rest.pose))
            .collect();
        for track in &self.tracks {
            if let Some(transform) = track.sample(time) {
                pose.insert(track.bone.clone(), transform);
            }
        }
        pose
    }

    /// The clip as an animation the engine plays (one tick per second).
    pub fn to_animation(&self, name: &str) -> Animation {
        let mut channels = HashMap::new();
        let mut add = |bone: &str, keys: Vec<(f32, Transform)>| {
            let vector = |v: [f32; 3]| Vector3::new(v[0], v[1], v[2]);
            channels.insert(
                bone.to_string(),
                Channel {
                    bone_id: bone.to_string(),
                    position_keys: keys
                        .iter()
                        .map(|(t, p)| (*t, vector(p.translation)))
                        .collect(),
                    rotation_keys: keys.iter().map(|(t, p)| (*t, p.quaternion())).collect(),
                    scaling_keys: keys.iter().map(|(t, p)| (*t, vector(p.scale))).collect(),
                },
            );
        };
        for rest in &self.rest {
            add(&rest.bone, vec![(0.0, rest.pose)]);
        }
        for track in self.tracks.iter().filter(|t| !t.keys.is_empty()) {
            add(
                &track.bone,
                track.keys.iter().map(|k| (k.time, k.pose)).collect(),
            );
        }
        Animation {
            name: name.to_string(),
            duration: self.duration.max(MIN_DURATION),
            ticks_per_second: 1.0,
            channels,
            root_channel: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, Rotation3};

    fn turned(deg: f32) -> Transform {
        let q = Quaternion::from_angle_y(Deg(deg));
        Transform {
            translation: [0.0, 1.0, 0.0],
            rotation: [q.v.x, q.v.y, q.v.z, q.s],
            scale: [1.0; 3],
        }
    }

    fn yaw(t: &Transform) -> f32 {
        let q = t.quaternion();
        (2.0 * q.v.y.atan2(q.s)).to_degrees()
    }

    #[test]
    fn keys_are_kept_sorted_and_replaced_at_the_same_time() {
        let mut clip = ClipFile::new(None, 2.0);
        clip.set_key("arm", 1.0, turned(90.0));
        clip.set_key("arm", 0.0, turned(0.0));
        clip.set_key("arm", 1.0005, turned(45.0));
        let keys = &clip.track("arm").unwrap().keys;
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].time, 0.0);
        assert!((yaw(&keys[1].pose) - 45.0).abs() < 0.01);
        assert_eq!(clip.key_times(), vec![0.0, 1.0]);

        assert!(clip.remove_key("arm", 0.0));
        assert!(clip.remove_key("arm", 1.0));
        assert!(clip.tracks.is_empty());
        assert!(!clip.remove_key("arm", 1.0));
    }

    #[test]
    fn rotations_are_slerped_and_held_outside_the_keys() {
        let mut clip = ClipFile::new(None, 2.0);
        clip.set_key("arm", 0.5, turned(0.0));
        clip.set_key("arm", 1.5, turned(90.0));
        let track = clip.track("arm").unwrap();
        assert!((yaw(&track.sample(1.0).unwrap()) - 45.0).abs() < 0.01);
        assert!(yaw(&track.sample(0.0).unwrap()).abs() < 0.01);
        assert!((yaw(&track.sample(2.0).unwrap()) - 90.0).abs() < 0.01);
    }

    #[test]
    fn the_animation_plays_like_the_clip() {
        let mut clip = ClipFile::new(None, 2.0);
        clip.set_key("arm", 0.5, turned(0.0));
        clip.set_key("arm", 1.5, turned(90.0));
        clip.rest.push(BonePose {
            bone: "leg".into(),
            pose: turned(10.0),
        });
        let animation = clip.to_animation("swing");
        assert_eq!(animation.ticks_per_second, 1.0);
        // Before the first key, the first key holds.
        let start = animation.sample(0.2);
        let arm = &start.transforms["arm"];
        assert!(arm.rotation.v.y.abs() < 1e-4);
        let middle = animation.sample(1.0);
        let q = middle.transforms["arm"].rotation;
        assert!(((2.0 * q.v.y.atan2(q.s)).to_degrees() - 45.0).abs() < 0.01);
        assert!(middle.transforms.contains_key("leg"));
    }

    #[test]
    fn matrices_round_trip_through_a_transform() {
        let original = turned(30.0).to_matrix() * Matrix4::from_scale(2.0);
        let back = Transform::from_matrix(&original).to_matrix();
        for c in 0..4 {
            for r in 0..4 {
                assert!((original[c][r] - back[c][r]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn the_file_round_trips_as_ron() {
        let mut clip = ClipFile::new(Some(AssetId::from(7)), 1.0);
        clip.set_key("mixamorig:RightArm", 0.25, turned(20.0));
        let text = clip.to_ron().unwrap();
        assert_eq!(ClipFile::from_ron(&text).unwrap(), clip);
        assert!(is_clip_file(Path::new(
            "assets/animations/Axe Swing.clip.ron"
        )));
        assert!(!is_clip_file(Path::new("Axe Swing.clip.ron.asset.ron")));
        assert_eq!(clip_name(Path::new("a/Axe Swing.clip.ron")), "Axe Swing");
    }

    #[test]
    fn moving_and_shortening_keeps_keys_in_range() {
        let mut clip = ClipFile::new(None, 2.0);
        clip.set_key("arm", 1.0, turned(0.0));
        clip.set_key("leg", 1.0, turned(5.0));
        clip.set_key("leg", 1.8, turned(9.0));
        clip.move_keys(1.0, 0.4);
        assert_eq!(clip.key_times(), vec![0.4, 1.8]);
        clip.set_duration(1.0);
        assert_eq!(clip.key_times(), vec![0.4]);
    }

    #[test]
    fn loaded_clips_are_sorted_finite_and_of_unit_rotations() {
        let mut clip = ClipFile::new(None, 2.0);
        let mut long = turned(0.0);
        long.rotation = [0.0, 0.7, 0.0, 0.7];
        let mut broken = turned(0.0);
        broken.translation[0] = f32::NAN;
        clip.tracks.push(BoneTrack {
            bone: "arm".into(),
            keys: vec![
                BoneKey { time: 1.5, pose: turned(90.0) },
                BoneKey { time: 0.5, pose: long },
                BoneKey { time: 1.0, pose: broken },
                BoneKey { time: f32::NAN, pose: turned(0.0) },
            ],
        });
        clip.tracks.push(BoneTrack {
            bone: "leg".into(),
            keys: vec![BoneKey { time: 0.0, pose: broken }],
        });
        clip.rest.push(BonePose { bone: "head".into(), pose: long });
        clip.sanitize();

        assert_eq!(clip.tracks.len(), 1);
        let keys = &clip.tracks[0].keys;
        assert_eq!(keys.iter().map(|k| k.time).collect::<Vec<_>>(), vec![0.5, 1.5]);
        for pose in [keys[0].pose, clip.rest[0].pose] {
            assert!((pose.quaternion().magnitude() - 1.0).abs() < 1e-5);
        }
        assert!((yaw(&keys[0].pose) - 90.0).abs() < 0.01);
    }
}
