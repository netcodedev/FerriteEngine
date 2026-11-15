//! The asset workspace: settings of one asset in the left column and a
//! preview of it where the scene viewport normally is. Edits are made on a
//! working copy and reach the asset (and whatever uses it) when saved; the
//! preview always shows the working copy.

use std::path::PathBuf;

use cgmath::{InnerSpace, Vector3};
use ferrite::core::{
    assets::{self, AssetId, AssetKind, AssetMeta, ClipInfo},
    model::{clip_file::is_clip_file, grip::Grip},
};

use super::clip_editor::ClipEdit;

mod draw;
mod inspector;
mod preview;
mod rig;
#[cfg(test)]
mod tests;

use draw::*;
pub use inspector::*;
pub use preview::*;
use rig::*;

/// A texture a material can be given.
pub use ferrite::core::assets::MaterialMap as TextureSlot;

/// The asset open in the asset workspace.
pub struct AssetEdit {
    /// Working copy of the asset's sidecar.
    pub meta: AssetMeta,
    /// Folder of the asset's source file.
    pub dir: PathBuf,
    /// Whether the working copy differs from what is saved.
    pub dirty: bool,
    /// Variant shown in the preview (index into the model's variants).
    pub variant: usize,
    /// LOD shown in the preview; `None` picks it by camera distance.
    pub lod: Option<usize>,
    /// Goes up when the settings panel has to be rebuilt.
    pub structure: u64,
    /// Goes up when the preview has to load the model again.
    pub preview: u64,
    /// The clip, when the asset is a clip made in the editor.
    pub clip: Option<ClipEdit>,
    /// The preview shows the model held in the player model's hand (it
    /// has a grip).
    pub holding: bool,
    /// What the player model does while holding it: the rest pose, or one
    /// of these clips.
    pub hold_poses: Vec<HoldPose>,
    /// Index into `hold_poses` + 1; 0 is the rest pose.
    pub hold_pose: usize,
}

/// An animation clip to show a held model in.
#[derive(Clone, Debug, PartialEq)]
pub struct HoldPose {
    pub label: String,
    pub source: PathBuf,
    /// The clip's name in a file of several.
    pub clip: Option<String>,
}

/// Every animation clip of the project, to hold a model in.
fn hold_poses() -> Vec<HoldPose> {
    let database = assets::database();
    let mut poses: Vec<HoldPose> = database
        .iter()
        .filter(|entry| matches!(entry.meta.kind, AssetKind::Animation(_)))
        .flat_map(|entry| {
            let clips = entry.meta.clips();
            let several = clips.len() > 1;
            clips
                .iter()
                .map(|clip| HoldPose {
                    label: if several {
                        format!("{} / {}", entry.meta.name, clip.name)
                    } else {
                        entry.meta.name.clone()
                    },
                    source: entry.source_path(),
                    clip: several.then(|| clip.name.clone()),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    poses.sort_by(|a, b| a.label.cmp(&b.label));
    poses
}

/// The six axis directions a grip's directions are picked from.
const GRIP_AXES: [([f32; 3], &str); 6] = [
    ([1.0, 0.0, 0.0], "+X"),
    ([-1.0, 0.0, 0.0], "-X"),
    ([0.0, 1.0, 0.0], "+Y"),
    ([0.0, -1.0, 0.0], "-Y"),
    ([0.0, 0.0, 1.0], "+Z"),
    ([0.0, 0.0, -1.0], "-Z"),
];

/// The axis direction closest to `v`.
fn grip_axis(v: [f32; 3]) -> usize {
    let v = Vector3::from(v);
    (0..GRIP_AXES.len())
        .max_by(|a, b| {
            let da = Vector3::from(GRIP_AXES[*a].0).dot(v);
            let db = Vector3::from(GRIP_AXES[*b].0).dot(v);
            da.total_cmp(&db)
        })
        .unwrap_or(0)
}

impl AssetEdit {
    /// Start editing an asset, showing the named variant if given.
    pub fn open(id: AssetId, variant: Option<&str>) -> Result<Self, String> {
        let database = assets::database();
        let entry = database
            .get(id)
            .ok_or_else(|| format!("asset {id} does not exist"))?;
        let variant = variant
            .and_then(|name| {
                entry
                    .meta
                    .as_model()?
                    .variants
                    .iter()
                    .position(|v| v.name == name)
            })
            .unwrap_or(0);
        let source = entry.source_path();
        let clip = if is_clip_file(&source) {
            Some(ClipEdit::load(&source)?)
        } else {
            None
        };
        let is_model = entry.meta.as_model().is_some();
        let meta = entry.meta.clone();
        let dir = entry.dir.clone();
        drop(database);
        Ok(Self {
            meta,
            dir,
            dirty: false,
            variant,
            lod: None,
            structure: 0,
            preview: 0,
            clip,
            holding: false,
            hold_poses: if is_model { hold_poses() } else { Vec::new() },
            hold_pose: 0,
        })
    }

    pub fn source_path(&self) -> PathBuf {
        self.dir.join(&self.meta.source)
    }

    /// The grip of the working copy, if it is a model that has one.
    pub fn grip(&self) -> Option<&Grip> {
        self.meta.as_model()?.grip.as_ref()
    }

    /// Change the grip of the working copy.
    pub fn set_grip(&mut self, grip: Option<Grip>) {
        let Some(model) = self.meta.as_model_mut() else {
            return;
        };
        if model.grip != grip {
            model.grip = grip;
            self.dirty = true;
        }
    }

    /// Give a material a texture (or none, for the automatic one).
    pub fn set_texture(&mut self, material: usize, slot: TextureSlot, texture: Option<AssetId>) {
        let Some(material) = self
            .meta
            .as_model_mut()
            .and_then(|model| model.materials.get_mut(material))
        else {
            return;
        };
        if material.map(slot) != texture {
            material.set_map(slot, texture);
            self.dirty = true;
            self.structure += 1;
            self.preview += 1;
        }
    }

    /// Write the working copy to the asset.
    pub fn save(&mut self) -> Result<(), String> {
        if let Some(clip) = &self.clip {
            clip.to_save()
                .save(&self.source_path())
                .map_err(|e| e.to_string())?;
            // The clip's length is listed with the asset; saving its
            // settings also makes graphs using it load it again.
            if let AssetKind::Animation(settings) = &mut self.meta.kind {
                let name = settings
                    .clips
                    .first()
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| self.meta.name.clone());
                settings.clips = vec![ClipInfo {
                    name,
                    duration: clip.duration(),
                }];
            }
        }
        assets::database()
            .update(self.meta.clone())
            .map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    /// Throw the edits away and go back to what is saved.
    pub fn revert(&mut self) -> Result<(), String> {
        let saved = assets::database()
            .get(self.meta.id)
            .map(|entry| entry.meta.clone())
            .ok_or_else(|| format!("asset {} does not exist any more", self.meta.id))?;
        self.meta = saved;
        if self.clip.is_some() {
            let mut clip = ClipEdit::load(&self.source_path())?;
            if let Some(old) = &self.clip {
                clip.selected = old.selected.clone();
                clip.rest = old.rest.clone();
                clip.bones = old.bones.clone();
                clip.keys = old.keys + 1;
                clip.set_time(old.time);
            }
            self.clip = Some(clip);
        }
        self.dirty = false;
        self.structure += 1;
        self.preview += 1;
        Ok(())
    }
}
