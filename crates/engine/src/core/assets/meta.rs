//! Contents of an asset's sidecar file (`<source file>.asset.ron`).

use serde::{Deserialize, Serialize};

use super::AssetId;
use crate::core::model::{anchor::Anchor, grip::Grip};

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
    /// A scatter graph (`*.scatter.ron`, see `terrain::scatter::graph`).
    ScatterGraph(ScatterGraphSettings),
    /// A material (`*.material.ron`, see `material::asset`).
    Material(MaterialAssetSettings),
    /// A material graph (`*.matgraph.ron`, see `material::graph`).
    MaterialGraph(MaterialGraphSettings),
    /// A sound file (.ogg, .wav, .flac, .mp3; see `audio`).
    Sound(SoundSettings),
}

impl AssetKind {
    /// Short name for lists and messages.
    pub fn label(&self) -> &'static str {
        match self {
            AssetKind::Model(_) => "Model",
            AssetKind::Texture(_) => "Texture",
            AssetKind::Animation(_) => "Animation",
            AssetKind::ScatterGraph(_) => "Scatter graph",
            AssetKind::Material(_) => "Material",
            AssetKind::MaterialGraph(_) => "Material graph",
            AssetKind::Sound(_) => "Sound",
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
    /// How the model is held in a hand, if it can be (an axe, a torch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grip: Option<Grip>,
    /// Named frames where other models go (a seat, a point on the back);
    /// see `model::anchor`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
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
            grip: None,
            anchors: Vec::new(),
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

/// One material slot of a model asset (in the file's material order).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MaterialSettings {
    /// Name in the file; settings are matched to materials by it on
    /// reimport.
    pub name: String,
    /// The material asset the slot is drawn with. `None`: what the model
    /// loader makes of the file's material (only until the project's
    /// models are given material assets, see
    /// `material::asset::ensure_all_model_materials`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<AssetId>,
    /// No mesh of the file is drawn with this material (Assimp's default
    /// material of an OBJ file): it gets no material asset of its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unused: bool,
    // From before materials (2026-11-21): texture assets per map and the
    // foliage flag, read so they move into the new material assets, never
    // written again.
    #[serde(default, skip_serializing)]
    pub base_color: Option<AssetId>,
    #[serde(default, skip_serializing)]
    pub normal: Option<AssetId>,
    #[serde(default, skip_serializing)]
    pub roughness: Option<AssetId>,
    #[serde(default, skip_serializing)]
    pub metallic: Option<AssetId>,
    #[serde(default, skip_serializing)]
    pub occlusion: Option<AssetId>,
    #[serde(default, rename = "foliage", skip_serializing, deserialize_with = "some_bool")]
    pub legacy_foliage: Option<bool>,
}

/// A plain `true`/`false` in the file, as `Some` (absent: `None`).
fn some_bool<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    bool::deserialize(deserializer).map(Some)
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
    /// A legacy map override (see the fields).
    pub fn map(&self, map: MaterialMap) -> Option<AssetId> {
        match map {
            MaterialMap::BaseColor => self.base_color,
            MaterialMap::Normal => self.normal,
            MaterialMap::Roughness => self.roughness,
            MaterialMap::Metallic => self.metallic,
            MaterialMap::Occlusion => self.occlusion,
        }
    }

    /// Whether the slot still waits for a material asset (see
    /// `material::asset::ensure_model_materials`).
    pub fn needs_material(&self) -> bool {
        self.material.is_none() && !self.unused
    }

    /// Forget the legacy fields (they moved into a material asset).
    pub fn clear_legacy(&mut self) {
        self.base_color = None;
        self.normal = None;
        self.roughness = None;
        self.metallic = None;
        self.occlusion = None;
        self.legacy_foliage = None;
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct TextureSettings {}

/// A scatter graph asset. The graph itself is the file; nothing about it
/// is kept here yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ScatterGraphSettings {}

/// A material asset. The material itself is the file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MaterialAssetSettings {}

/// A sound asset. The sound is the file, played as it is.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SoundSettings {}

/// A material graph asset. The graph itself is the file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct MaterialGraphSettings {}

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
            AssetKind::Texture(_)
            | AssetKind::ScatterGraph(_)
            | AssetKind::Material(_)
            | AssetKind::MaterialGraph(_)
            | AssetKind::Sound(_) => &[],
        }
    }
}

#[cfg(test)]
mod legacy_tests {
    use super::*;

    /// Sidecars from before material assets (2026-11-21) still load, their
    /// overrides and foliage flags kept for the move to material assets.
    #[test]
    fn a_material_slot_from_before_material_assets_loads() {
        let old = format!(r#"(name: "Leaves", base_color: Some({}), normal: None, foliage: true)"#, ron::to_string(&AssetId::from(12)).unwrap());
        let slot: MaterialSettings = ron::from_str(&old).unwrap();
        assert_eq!(slot.legacy_foliage, Some(true));
        assert_eq!(slot.base_color, Some(AssetId::from(12)));
        assert_eq!(slot.material, None);
        let plain: MaterialSettings = ron::from_str(r#"(name: "Bark", foliage: false)"#).unwrap();
        assert_eq!(plain.legacy_foliage, Some(false));
        // Written again: only the name and the material.
        let mut slot = slot;
        slot.material = Some(AssetId::from(7));
        slot.clear_legacy();
        let text = ron::to_string(&slot).unwrap();
        assert!(!text.contains("foliage") && !text.contains("base_color"), "{text}");
    }
}
