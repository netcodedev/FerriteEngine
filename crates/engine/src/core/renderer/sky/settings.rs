//! Which clouds the sky shows, kept with the sky light (coverage of the low
//! layer is `SkyLight::cloud_coverage`).

use serde::{Deserialize, Serialize};

/// Form of the low cloud layer. The sky varies it a little from region to
/// region, so a cumulus sky still has some flatter and taller clouds.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CloudType {
    /// A flat grey sheet, low and wide.
    Stratus,
    /// Lumpy, flat-topped clouds in a shallow layer.
    Stratocumulus,
    /// Fair-weather heaps with flat bases and round tops.
    #[default]
    Cumulus,
    /// Towering storm clouds that spread into an anvil at the top.
    Cumulonimbus,
}

impl CloudType {
    pub const ALL: [CloudType; 4] = [
        CloudType::Stratus,
        CloudType::Stratocumulus,
        CloudType::Cumulus,
        CloudType::Cumulonimbus,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CloudType::Stratus => "Stratus",
            CloudType::Stratocumulus => "Stratocumulus",
            CloudType::Cumulus => "Cumulus",
            CloudType::Cumulonimbus => "Cumulonimbus",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The `cloudType` uniform of `cloud_field.glsl` (0 flat .. 1 towering).
    pub fn shader_value(self) -> f32 {
        match self {
            CloudType::Stratus => 0.0,
            CloudType::Stratocumulus => 0.36,
            CloudType::Cumulus => 0.68,
            CloudType::Cumulonimbus => 1.0,
        }
    }
}

/// The cloud layers above the low one, and the low one's type.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct CloudSettings {
    pub cloud_type: CloudType,
    /// Fields of small puffs at mid height, 0 (none) .. 1 (most of the sky).
    pub altocumulus: f32,
    /// Streaks of ice high up, 0 (none) .. 1 (most of the sky).
    pub cirrus: f32,
    /// Draw a quarter of the clouds each frame and take the rest from the
    /// frames before: about 4x cheaper, but fast turns of the camera and
    /// fast clouds can smear a little.
    pub reprojection: bool,
}

impl Default for CloudSettings {
    fn default() -> Self {
        Self {
            cloud_type: CloudType::Cumulus,
            altocumulus: 0.15,
            cirrus: 0.35,
            reprojection: false,
        }
    }
}

impl CloudSettings {
    pub fn set_altocumulus(&mut self, amount: f32) {
        self.altocumulus = amount.clamp(0.0, 1.0);
    }

    pub fn set_cirrus(&mut self, amount: f32) {
        self.cirrus = amount.clamp(0.0, 1.0);
    }

    /// Copy every field through its setter, so out of range values (from a
    /// hand edited scene file) are clamped.
    pub fn sanitized(self) -> Self {
        let mut settings = self;
        settings.set_altocumulus(self.altocumulus);
        settings.set_cirrus(self.cirrus);
        settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_types_round_trip_through_their_names() {
        for t in CloudType::ALL {
            assert_eq!(CloudType::from_name(t.name()), Some(t));
        }
        assert_eq!(CloudType::from_name("Nimbostratus"), None);
    }

    #[test]
    fn cloud_types_get_taller_in_order() {
        let values: Vec<f32> = CloudType::ALL.iter().map(|t| t.shader_value()).collect();
        assert!(values.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(values[0], 0.0);
        assert_eq!(values[3], 1.0);
    }

    #[test]
    fn scenes_without_cloud_settings_get_the_defaults() {
        let settings: CloudSettings = ron::from_str("()").unwrap();
        assert_eq!(settings, CloudSettings::default());
        let clamped = CloudSettings { cloud_type: CloudType::Stratus, altocumulus: 3.0, cirrus: -1.0 }.sanitized();
        assert_eq!((clamped.altocumulus, clamped.cirrus), (1.0, 0.0));
    }
}
