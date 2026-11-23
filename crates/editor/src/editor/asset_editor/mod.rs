//! The asset workspace: settings of one asset in the left column and a
//! preview of it where the scene viewport normally is. Edits are made on a
//! working copy and reach the asset (and whatever uses it) when saved; the
//! preview always shows the working copy.

use std::path::PathBuf;

use cgmath::{InnerSpace, Vector3};
use ferrite::core::{
    assets::{self, AssetId, AssetKind, AssetMeta, ClipInfo, ModelSettings},
    model::{anchor::Anchor, clip_file::is_clip_file, grip::Grip},
};

use super::clip_editor::ClipEdit;

mod anchors_ui;
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
    /// Working copies of other model assets whose anchors are edited here
    /// (a clip's model and its references), and whether they changed.
    /// Saved and reverted with the asset.
    pub others: Vec<(AssetMeta, bool)>,
    /// The anchor selected in the preview and the inspector.
    pub anchor: Option<AnchorPick>,
}

/// An anchor of a model asset, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorPick {
    pub asset: AssetId,
    pub name: String,
}

/// The player model's asset, which clips without a model are posed on.
pub fn player_model() -> Option<AssetId> {
    let database = assets::database();
    let source = database
        .root()
        .join("models")
        .join(ferrite::player::PLAYER_MODEL_FILE);
    database.find_by_source(&source)
}

/// Every model asset of the project, by name: what a clip can show as a
/// reference.
pub fn model_assets() -> Vec<(AssetId, String)> {
    let database = assets::database();
    let mut models: Vec<(AssetId, String)> = database
        .iter()
        .filter(|entry| entry.meta.as_model().is_some())
        .map(|entry| (entry.meta.id, entry.meta.name.clone()))
        .collect();
    models.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    models
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
            .ok_or_else(|| tr!("asset_editor.error.missing", id = id))?;
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
            others: Vec::new(),
            anchor: None,
        })
    }

    /// The model asset the open clip is posed on.
    pub fn clip_model(&self) -> Option<AssetId> {
        self.clip.as_ref()?.file.model.or_else(player_model)
    }

    /// The working copy of a model asset's settings: this asset's, an
    /// edited other one's, or as saved.
    pub fn model_settings(&self, id: AssetId) -> Option<ModelSettings> {
        if id == self.meta.id {
            return self.meta.as_model().cloned();
        }
        if let Some((meta, _)) = self.others.iter().find(|(meta, _)| meta.id == id) {
            return meta.as_model().cloned();
        }
        assets::database().get(id)?.meta.as_model().cloned()
    }

    /// The anchors of a model asset, as edited.
    pub fn anchors(&self, id: AssetId) -> Vec<Anchor> {
        self.model_settings(id).map(|m| m.anchors).unwrap_or_default()
    }

    pub fn anchor_named(&self, id: AssetId, name: &str) -> Option<Anchor> {
        self.anchors(id).into_iter().find(|a| a.name == name)
    }

    /// The grip of a model asset, as edited.
    pub fn grip_of(&self, id: AssetId) -> Option<Grip> {
        self.model_settings(id)?.grip
    }

    /// Change the anchors of a model asset (this one or another).
    pub fn set_anchors(&mut self, id: AssetId, anchors: Vec<Anchor>) {
        if id == self.meta.id {
            if let Some(model) = self.meta.as_model_mut() {
                if model.anchors != anchors {
                    model.anchors = anchors;
                    self.dirty = true;
                }
            }
            return;
        }
        let at = match self.others.iter().position(|(meta, _)| meta.id == id) {
            Some(at) => at,
            None => {
                let Some(meta) = assets::database().get(id).map(|e| e.meta.clone()) else {
                    return;
                };
                self.others.push((meta, false));
                self.others.len() - 1
            }
        };
        let (meta, changed) = &mut self.others[at];
        if let Some(model) = meta.as_model_mut() {
            if model.anchors != anchors {
                model.anchors = anchors;
                *changed = true;
                self.dirty = true;
            }
        }
    }

    /// Replace the anchor `name` of a model asset (renaming it renames it
    /// in the clip's references too).
    pub fn set_anchor(&mut self, id: AssetId, name: &str, anchor: Anchor) {
        let mut anchors = self.anchors(id);
        let Some(at) = anchors.iter().position(|a| a.name == name) else { return };
        let renamed = anchor.name != name;
        if renamed && anchors.iter().any(|a| a.name == anchor.name) {
            return;
        }
        let new_name = anchor.name.clone();
        anchors[at] = anchor;
        self.set_anchors(id, anchors);
        if renamed {
            let clip_model = self.clip_model();
            if let Some(clip) = &mut self.clip {
                clip.rename_anchor(id, clip_model, name, &new_name);
            }
            if self.anchor.as_ref() == Some(&AnchorPick { asset: id, name: name.to_string() }) {
                self.anchor = Some(AnchorPick { asset: id, name: new_name });
            }
        }
    }

    /// Add an anchor to a model asset and select it.
    pub fn add_anchor(&mut self, id: AssetId, bone: Option<String>, position: [f32; 3]) {
        let mut anchors = self.anchors(id);
        let base = match &bone {
            Some(bone) => ferrite::core::model::ik::base_name(bone).to_string(),
            None => "anchor".to_string(),
        };
        let name = ferrite::core::model::anchor::free_name(&anchors, &base);
        anchors.push(Anchor::new(&name, bone, position));
        self.set_anchors(id, anchors);
        self.anchor = Some(AnchorPick { asset: id, name });
        self.structure += 1;
    }

    /// Remove the selected anchor.
    pub fn remove_anchor(&mut self) {
        let Some(pick) = self.anchor.take() else { return };
        let mut anchors = self.anchors(pick.asset);
        anchors.retain(|a| a.name != pick.name);
        self.set_anchors(pick.asset, anchors);
        self.structure += 1;
    }

    pub fn select_anchor(&mut self, pick: Option<AnchorPick>) {
        if self.anchor != pick {
            self.anchor = pick;
            self.structure += 1;
        }
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

    /// Draw a material slot with a material asset (or what the loader
    /// makes of the file's material).
    pub fn set_material(&mut self, material: usize, asset: Option<AssetId>) {
        let Some(material) = self
            .meta
            .as_model_mut()
            .and_then(|model| model.materials.get_mut(material))
        else {
            return;
        };
        if material.material != asset {
            material.material = asset;
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
        // Anchors moved on the clip's model or a reference.
        for (meta, changed) in &mut self.others {
            if *changed {
                assets::database()
                    .update(meta.clone())
                    .map_err(|e| format!("{}: {e}", meta.name))?;
                *changed = false;
            }
        }
        self.dirty = false;
        Ok(())
    }

    /// Throw the edits away and go back to what is saved.
    pub fn revert(&mut self) -> Result<(), String> {
        let saved = assets::database()
            .get(self.meta.id)
            .map(|entry| entry.meta.clone())
            .ok_or_else(|| tr!("asset_editor.error.gone", id = self.meta.id))?;
        self.meta = saved;
        self.others.clear();
        self.anchor = None;
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
