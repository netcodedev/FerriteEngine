mod settings;
mod sky;

pub use settings::{CloudSettings, CloudType};
pub use sky::{seconds, SkyRenderer};

/// The low cloud layer's density field (`cloudDensity`, `cloudLayerTop`),
/// for shaders that need to know where the clouds are, e.g. for cloud
/// shadows. Declare `uTime`, `cloudCoverage` and `cloudType` before it.
pub const CLOUD_FIELD_GLSL: &str = include_str!("cloud_field.glsl");
/// `cloudShadow(worldPos)`: how much of the sun the clouds let through to
/// a point on the ground. Goes after `CLOUD_FIELD_GLSL`.
pub const CLOUD_SHADOW_GLSL: &str = include_str!("cloud_shadow.glsl");
