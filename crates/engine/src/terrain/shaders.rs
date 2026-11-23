//! The terrain's programs: the standard ones a source may keep, the pieces
//! a source's own fragment program builds on, and the water's.
//!
//! A source's fragment program (see `TerrainSource::shaders`) starts with
//! `fragment_prelude()`: the GLSL version, the scene's lighting and weather
//! (`shadeSurface`, `applyRain`, `skyExposure`, `cloudShadow`, `toDisplay`,
//! `cameraPosition`, `isShadowPass`...) and `fragment_common.glsl` (the
//! standard vertex program's outputs `Normal`, `WorldPos`, `GroundColor`,
//! `Data0..Data3`, `FragColor`, `hiddenHorizon()`, `terrainHaze()` and
//! `waterLine`) and `HAZE_GLSL`.

use crate::core::renderer::{pbr, sky};

use super::columns::COLUMNS_GLSL;

/// The standard vertex program: positions, normals, the ground's colour and
/// the source's data at locations 0 to 5, sunk under the chunks where it
/// draws the horizon.
pub const STANDARD_VERTEX: &str = include_str!("shaders/vertex.glsl");

/// The standard depth-only vertex program, matching `STANDARD_VERTEX`.
pub const STANDARD_DEPTH_VERTEX: &str = include_str!("shaders/depth_vertex.glsl");

/// The body of the engine's own fragment program (after
/// `fragment_prelude()`): the vertices' colours, lit.
pub const DEFAULT_FRAGMENT: &str = include_str!("shaders/default_fragment.glsl");

/// `terrainHaze(color, p)`: the haze the terrain's far ground gets, for
/// what stands on it to get the same. Goes after `pbr::LIGHTING_GLSL`.
pub const HAZE_GLSL: &str = include_str!("shaders/haze.glsl");

/// Uniforms `sky::CLOUD_FIELD_GLSL` needs that `pbr::LIGHTING_GLSL` does
/// not declare.
const CLOUD_UNIFORMS: &str = "uniform float uTime;\nuniform float cloudType;";

/// What goes before a terrain fragment program's own code: see the module.
pub fn fragment_prelude() -> String {
    format!(
        "#version 460 core\n{}\n{CLOUD_UNIFORMS}\n{}\n{}\n{COLUMNS_GLSL}\n{}\n{HAZE_GLSL}",
        pbr::LIGHTING_GLSL,
        sky::CLOUD_FIELD_GLSL,
        sky::CLOUD_SHADOW_GLSL,
        include_str!("shaders/fragment_common.glsl"),
    )
}

/// The fragment program the horizon's depth is drawn with (after a depth
/// vertex program that hands on `WorldPos`): left out where the chunks are
/// loaded, as `hiddenHorizon()` leaves it out.
pub fn horizon_depth_fragment() -> String {
    format!("#version 460 core\n{COLUMNS_GLSL}\n{}", include_str!("shaders/horizon_depth_fragment.glsl"))
}

/// The water's programs (see `water`).
pub(crate) fn water_sources() -> (String, String) {
    (
        format!(
            "#version 460 core\n{}\n{}",
            include_str!("shaders/water_waves.glsl"),
            include_str!("shaders/water_vertex.glsl")
        ),
        format!(
            "#version 460 core\n{}\n{COLUMNS_GLSL}\n{}\n{}",
            pbr::LIGHTING_GLSL,
            include_str!("shaders/water_waves.glsl"),
            include_str!("shaders/water_fragment.glsl")
        ),
    )
}
