//! Scatter layers: a model plus a density, as stored and as loaded.

use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::core::assets::{self, AssetRef};

use super::*;

/// One scatter layer as stored in a scene file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScatterLayerData {
    /// The model asset to scatter: all its variants, or the one named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<AssetRef>,
    /// A raw model file, as a path relative to `assets/models/`. Used by
    /// scenes from before assets existed and when `asset` is not set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Placements tried per 100 m x 100 m of terrain.
    pub density: f32,
    /// What the world makes of the layer: its source may let the models of
    /// a "tree" layer grow thick in forests and thin in the plains (see
    /// `TerrainSource::scatter_chance`). Empty: untagged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
}

impl ScatterLayerData {
    pub fn asset(asset: AssetRef, density: f32) -> Self {
        Self {
            asset: Some(asset),
            model: String::new(),
            density,
            tag: String::new(),
        }
    }

    pub fn file(model: impl Into<String>, density: f32) -> Self {
        Self {
            asset: None,
            model: model.into(),
            density,
            tag: String::new(),
        }
    }
}

/// Seconds an edited layer has to stay untouched before it is applied, so
/// typing a path or dragging the density does not replant on every change.
pub(super) const APPLY_DELAY: f64 = 0.6;

pub(super) struct ScatterLayer {
    /// The asset to scatter. Takes precedence over `model`.
    pub(super) asset: Option<AssetRef>,
    /// Raw model file as typed; empty when unused.
    pub(super) model: String,
    pub(super) density: f32,
    pub(super) tag: String,
    /// The tag of what is planted right now.
    pub(super) applied_tag: String,
    /// What is planted right now.
    pub(super) loaded: Option<Rc<ScatterModel>>,
    /// Revision of the asset `loaded` was built from.
    loaded_revision: u64,
    pub(super) applied_density: f32,
    pub(super) error: Option<String>,
    /// Countdown until the edited values are applied; `None` when they are.
    pub(super) apply_in: Option<f64>,
}

impl ScatterLayer {
    pub(super) fn new(mut data: ScatterLayerData) -> Self {
        // A raw file that is registered as an asset is referred to as that
        // asset from here on; scenes from before assets existed are brought
        // up to date this way.
        if data.asset.is_none() && !data.model.trim().is_empty() {
            let database = assets::database();
            let source = database
                .root()
                .join("models")
                .join(data.model.trim().replace('\\', "/"));
            if let Some(id) = database.find_by_source(&source) {
                data.asset = Some(AssetRef::whole(id));
                data.model.clear();
            }
        }
        Self {
            asset: data.asset,
            model: data.model,
            density: data.density,
            applied_tag: data.tag.clone(),
            tag: data.tag,
            loaded: None,
            loaded_revision: 0,
            applied_density: data.density,
            error: None,
            apply_in: Some(0.0),
        }
    }

    /// Stable per-model salt, so a model keeps its spots when layers are
    /// reordered.
    pub(super) fn salt(&self) -> u64 {
        match &self.asset {
            Some(asset) => fnv1a(&format!(
                "{}/{}",
                asset.id,
                asset.part.as_deref().unwrap_or("")
            )),
            None => fnv1a(&self.path()),
        }
    }

    /// The model path as used for loading and saving: trimmed, with forward
    /// slashes so a scene saved on Windows also loads on Linux.
    pub(super) fn path(&self) -> String {
        self.model.trim().replace('\\', "/")
    }

    /// Neither an asset nor a file: the layer scatters nothing.
    pub(super) fn is_unused(&self) -> bool {
        self.asset.is_none() && self.path().is_empty()
    }

    /// What the layer scatters, for messages.
    pub(super) fn label(&self) -> String {
        match &self.asset {
            Some(asset) => assets::database()
                .display_name(asset)
                .unwrap_or_else(|| format!("missing asset {}", asset.id)),
            None => self.path(),
        }
    }

    /// Whether the asset was saved again since it was loaded.
    pub(super) fn is_outdated(&self) -> bool {
        let (Some(asset), Some(_)) = (&self.asset, &self.loaded) else {
            return false;
        };
        assets::database()
            .get(asset.id)
            .is_some_and(|entry| entry.revision != self.loaded_revision)
    }

    pub(super) fn apply(&mut self) {
        self.applied_density = self.density;
        self.applied_tag = self.tag.clone();
        self.error = None;
        self.loaded = None;
        let result = match &self.asset {
            Some(asset) => ScatterModel::load_asset(asset).map(|(model, revision)| {
                self.loaded_revision = revision;
                model
            }),
            None => {
                let path = self.path();
                if path.is_empty() {
                    return;
                }
                ScatterModel::load(&path)
            }
        };
        match result {
            Ok(model) => self.loaded = Some(model),
            Err(e) => {
                log::warn!("terrain scatter: could not load {}: {e}", self.label());
                self.error = Some(e);
            }
        }
    }
}
