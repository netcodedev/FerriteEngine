//! Contents of an asset's sidecar file (`<source file>.asset.ron`).

use serde::{Deserialize, Serialize};

use super::AssetId;

pub const ASSET_META_VERSION: u32 = 1;

/// Appended to the source file's name to get the sidecar's name.
pub const META_SUFFIX: &str = ".asset.ron";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AssetMeta {
    pub version: u32,
    pub id: AssetId,
    /// Display name; free to change.
    pub name: String,
    /// File name of the source file, which sits next to the sidecar.
    pub source: String,
    pub kind: AssetKind,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AssetKind {
    Model(ModelSettings),
    Texture(TextureSettings),
    /// A file that holds animation clips but no meshes.
    Animation(AnimationSettings),
}

impl AssetKind {
    /// Short name for lists and messages.
    pub fn label(&self) -> &'static str {
        match self {
            AssetKind::Model(_) => "Model",
            AssetKind::Texture(_) => "Texture",
            AssetKind::Animation(_) => "Animation",
        }
    }
}

/// Settings of a model asset, plus the structure found in the file at
/// import (cached here so tools can show it without loading the model).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ModelSettings {
    /// Applied on top of the units of the file.
    #[serde(default = "one")]
    pub scale: f32,
    /// The separately usable objects in the file, e.g. the trees of a pack.
    #[serde(default)]
    pub variants: Vec<VariantInfo>,
    /// In the file's material order.
    #[serde(default)]
    pub materials: Vec<MaterialSettings>,
    /// Animation clips the file brings along with its meshes.
    #[serde(default)]
    pub clips: Vec<ClipInfo>,
}

fn one() -> f32 {
    1.0
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            scale: 1.0,
            variants: Vec::new(),
            materials: Vec::new(),
            clips: Vec::new(),
        }
    }
}

impl ModelSettings {
    pub fn variant(&self, name: &str) -> Option<&VariantInfo> {
        self.variants.iter().find(|v| v.name == name)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VariantInfo {
    pub name: String,
    /// Names of the detail tiers, as found in the file.
    #[serde(default)]
    pub lods: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MaterialSettings {
    /// Name in the file; settings are matched to materials by it on
    /// reimport.
    pub name: String,
    /// Texture asset used as colour map. `None`: whatever the model loader
    /// finds for this material on its own.
    #[serde(default)]
    pub base_color: Option<AssetId>,
    /// Texture asset used as normal map. `None`: as for `base_color`.
    #[serde(default)]
    pub normal: Option<AssetId>,
    /// Texture asset used as roughness map. `None`: as for `base_color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roughness: Option<AssetId>,
    /// Texture asset used as metallic map. `None`: as for `base_color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metallic: Option<AssetId>,
    /// Texture asset used as ambient occlusion map. `None`: as for
    /// `base_color`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occlusion: Option<AssetId>,
    /// Leaves and the like: drawn, but left out of collision shapes.
    #[serde(default)]
    pub foliage: bool,
}

/// A texture slot of a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialMap {
    BaseColor,
    Normal,
    Roughness,
    Metallic,
    Occlusion,
}

impl MaterialMap {
    pub const ALL: [MaterialMap; 5] = [
        MaterialMap::BaseColor,
        MaterialMap::Normal,
        MaterialMap::Roughness,
        MaterialMap::Metallic,
        MaterialMap::Occlusion,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MaterialMap::BaseColor => "base color",
            MaterialMap::Normal => "normal",
            MaterialMap::Roughness => "roughness",
            MaterialMap::Metallic => "metallic",
            MaterialMap::Occlusion => "occlusion",
        }
    }
}

impl MaterialSettings {
    pub fn map(&self, map: MaterialMap) -> Option<AssetId> {
        match map {
            MaterialMap::BaseColor => self.base_color,
            MaterialMap::Normal => self.normal,
            MaterialMap::Roughness => self.roughness,
            MaterialMap::Metallic => self.metallic,
            MaterialMap::Occlusion => self.occlusion,
        }
    }

    pub fn set_map(&mut self, map: MaterialMap, texture: Option<AssetId>) {
        let slot = match map {
            MaterialMap::BaseColor => &mut self.base_color,
            MaterialMap::Normal => &mut self.normal,
            MaterialMap::Roughness => &mut self.roughness,
            MaterialMap::Metallic => &mut self.metallic,
            MaterialMap::Occlusion => &mut self.occlusion,
        };
        *slot = texture;
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TextureSettings {}

/// The clips found in an animation file at import.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AnimationSettings {
    #[serde(default)]
    pub clips: Vec<ClipInfo>,
}

/// One animation clip of a file. A reference names it as its part.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    /// Length in seconds.
    #[serde(default)]
    pub duration: f32,
}

impl AssetMeta {
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn as_model(&self) -> Option<&ModelSettings> {
        match &self.kind {
            AssetKind::Model(settings) => Some(settings),
            _ => None,
        }
    }

    pub fn as_model_mut(&mut self) -> Option<&mut ModelSettings> {
        match &mut self.kind {
            AssetKind::Model(settings) => Some(settings),
            _ => None,
        }
    }

    /// The animation clips of the asset: those of an animation file, or
    /// the ones a model file brings along.
    pub fn clips(&self) -> &[ClipInfo] {
        match &self.kind {
            AssetKind::Animation(settings) => &settings.clips,
            AssetKind::Model(settings) => &settings.clips,
            AssetKind::Texture(_) => &[],
        }
    }
}
