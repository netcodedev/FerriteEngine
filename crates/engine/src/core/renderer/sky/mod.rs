mod settings;
mod sky;

pub use settings::{CloudSettings, CloudType};
pub use sky::SkyRenderer;

/// The low cloud layer's density field (`cloudDensity`, `cloudLayerTop`),
/// for shaders that need to know where the clouds are, e.g. for cloud
/// shadows. Declare `uTime`, `cloudCoverage` and `cloudType` before it.
pub const CLOUD_FIELD_GLSL: &str = include_str!("cloud_field.glsl");
