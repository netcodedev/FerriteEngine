//! Making animation clips (`*.clip.ron`) in the asset workspace: the clip
//! being edited, the timeline below the preview and the rings that turn the
//! selected bone. The preview (`asset_editor::AssetPreview`) poses the model
//! from the clip and writes keys back into it.

use std::path::Path;

use cgmath::{InnerSpace, Point3, Vector3};
use ferrite::core::{
    assets::{self, AssetId},
    model::clip_file::{BonePose, ClipFile, ClipReference, Transform, KEY_EPSILON},
};

mod timeline;
#[cfg(test)]
mod tests;

pub use timeline::*;

/// Something asked of the clip by a button or a shortcut; the preview
/// carries it out, as it holds the posed skeleton.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipRequest {
    TogglePlay,
    ToggleLoop,
    ToggleAutoKey,
    /// Key the selected bone and the bones posed since the last key.
    Key,
    /// Remove the selected bone's key at the playhead, or every key there
    /// without a selected bone.
    DeleteKey,
    ToStart,
}

/// The clip open in the asset workspace.
pub struct ClipEdit {
    /// Working copy.
    pub file: ClipFile,
    /// Playhead in seconds.
    pub time: f32,
    pub playing: bool,
    /// Playback starts over at the end.
    pub looping: bool,
    /// Posing a bone keys it at the playhead right away.
    pub auto_key: bool,
    /// Bone selected for the rings, the inspector and the timeline.
    pub selected: Option<String>,
    /// Goes up when keys change; the preview poses the model again.
    pub keys: u64,
    /// Goes up when the selection changes from outside the preview.
    pub selection: u64,
    /// Rest pose of the model's bones (filled by the preview); saved with
    /// the clip for the bones without keys.
    pub rest: Vec<BonePose>,
    /// Bones in skeleton order (filled by the preview), for the inspector.
    pub bones: Vec<String>,
    pub requests: Vec<ClipRequest>,
    /// Goes up when references are added, removed or given another model
    /// (the preview loads their models again).
    pub references: u64,
}

impl ClipEdit {
    pub fn new(file: ClipFile) -> Self {
        Self {
            file,
            time: 0.0,
            playing: false,
            looping: true,
            auto_key: true,
            selected: None,
            keys: 0,
            selection: 0,
            rest: Vec::new(),
            bones: Vec::new(),
            requests: Vec::new(),
            references: 0,
        }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        ClipFile::load(path)
            .map(Self::new)
            .map_err(|e| e.to_string())
    }

    pub fn duration(&self) -> f32 {
        self.file.duration
    }

    pub fn set_time(&mut self, time: f32) {
        self.time = time.clamp(0.0, self.file.duration);
    }

    /// Move the playhead on while playing.
    pub fn advance(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        let duration = self.file.duration;
        self.time += dt;
        if self.time >= duration {
            if self.looping {
                self.time %= duration;
            } else {
                self.time = duration;
                self.playing = false;
            }
        }
    }

    pub fn toggle_play(&mut self) {
        if !self.playing && self.time >= self.file.duration - KEY_EPSILON {
            self.time = 0.0;
        }
        self.playing = !self.playing;
    }

    /// Key these bones at the playhead.
    pub fn key(&mut self, poses: &[(String, Transform)]) {
        for (bone, pose) in poses {
            self.file.set_key(bone, self.time, *pose);
        }
        if !poses.is_empty() {
            self.keys += 1;
        }
    }

    /// See `ClipRequest::DeleteKey`. Returns whether a key went.
    pub fn delete_key(&mut self) -> bool {
        let removed = match &self.selected {
            Some(bone) => self.file.remove_key(bone, self.time),
            None => {
                let bones: Vec<String> = self.file.tracks.iter().map(|t| t.bone.clone()).collect();
                let mut any = false;
                for bone in bones {
                    any |= self.file.remove_key(&bone, self.time);
                }
                any
            }
        };
        if removed {
            self.keys += 1;
        }
        removed
    }

    /// Move a key (of one bone, or of all bones at that time).
    pub fn move_key(&mut self, bone: Option<&str>, from: f32, to: f32) {
        if (from - to).abs() < KEY_EPSILON {
            return;
        }
        match bone {
            Some(bone) => {
                let pose = self
                    .file
                    .track(bone)
                    .and_then(|t| t.key_at(from).map(|at| t.keys[at].pose));
                if let Some(pose) = pose {
                    self.file.remove_key(bone, from);
                    self.file.set_key(bone, to, pose);
                }
            }
            None => self.file.move_keys(from, to),
        }
        self.keys += 1;
    }

    pub fn set_duration(&mut self, duration: f32) {
        self.file.set_duration(duration);
        self.set_time(self.time);
        self.keys += 1;
    }

    pub fn select(&mut self, bone: Option<String>) {
        if self.selected != bone {
            self.selected = bone;
            self.selection += 1;
        }
    }

    /// Show another model with the clip.
    pub fn add_reference(&mut self, reference: ClipReference) {
        self.file.references.push(reference);
        self.references += 1;
    }

    pub fn remove_reference(&mut self, index: usize) {
        if index < self.file.references.len() {
            self.file.references.remove(index);
            self.references += 1;
        }
    }

    /// Change a reference; whether it changed.
    pub fn set_reference(&mut self, index: usize, reference: ClipReference) -> bool {
        let Some(old) = self.file.references.get_mut(index) else { return false };
        if *old == reference {
            return false;
        }
        if old.model != reference.model {
            self.references += 1;
        }
        *old = reference;
        true
    }

    /// An anchor of `asset` was renamed: references naming it follow.
    /// `clip_model` is the asset the clip is posed on.
    pub fn rename_anchor(&mut self, asset: AssetId, clip_model: Option<AssetId>, from: &str, to: &str) {
        for reference in &mut self.file.references {
            if reference.model == asset && reference.anchor.as_deref() == Some(from) {
                reference.anchor = Some(to.to_string());
            }
            if clip_model == Some(asset) && reference.model_anchor.as_deref() == Some(from) {
                reference.model_anchor = Some(to.to_string());
            }
        }
    }

    /// The working copy as it is written to disk.
    pub fn to_save(&self) -> ClipFile {
        let mut file = self.file.clone();
        file.rest = self.rest.clone();
        file
    }
}

/// Default length of a new clip (seconds).
pub const NEW_CLIP_DURATION: f32 = 1.0;
/// Folder below the asset root new clips go to.
const CLIP_FOLDER: &str = "animations";

/// Create `assets/animations/<name>.clip.ron` posed on `model` and register
/// it. Returns the new asset.
pub fn create_clip(name: &str, model: AssetId) -> Result<AssetId, String> {
    let name: String = name
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let name = name.trim();
    if name.is_empty() {
        return Err("the clip needs a name".into());
    }
    let mut database = assets::database();
    let folder = database.root().join(CLIP_FOLDER);
    let path = folder.join(format!(
        "{name}{}",
        ferrite::core::model::clip_file::CLIP_SUFFIX
    ));
    if path.exists() {
        return Err(format!("{} exists already", path.display()));
    }
    ClipFile::new(Some(model), NEW_CLIP_DURATION)
        .save(&path)
        .map_err(|e| e.to_string())?;
    database.import(&path).map_err(|e| e.to_string())
}

pub const RING_SEGMENTS: usize = 48;
pub const RING_COLORS: [(f32, f32, f32); 3] =
    [(0.95, 0.3, 0.3), (0.35, 0.85, 0.35), (0.35, 0.55, 1.0)];

/// Points of a ring of `radius` around `center`, perpendicular to `axis`.
pub fn ring_points(center: Point3<f32>, axis: Vector3<f32>, radius: f32) -> Vec<Point3<f32>> {
    let helper = if axis.y.abs() < 0.9 {
        Vector3::unit_y()
    } else {
        Vector3::unit_x()
    };
    let u = axis.cross(helper).normalize();
    let v = axis.cross(u).normalize();
    (0..=RING_SEGMENTS)
        .map(|i| {
            let a = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
            center + (u * a.cos() + v * a.sin()) * radius
        })
        .collect()
}

pub const AXES: [Vector3<f32>; 3] = [
    Vector3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    },
    Vector3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    },
    Vector3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    },
];
