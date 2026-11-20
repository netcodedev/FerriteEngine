//! Rain and snow falling around the camera.

use cgmath::{Matrix4, Point3};

use super::{Precipitation, WeatherMaps, EXPOSURE_UNIT};
use crate::core::{
    gfx,
    renderer::{
        pbr::{Lighting, LIGHTING_GLSL},
        shader::Shader,
    },
};

const SOURCE: &str = include_str!("precipitation.glsl");
/// Drops in the box around the camera in the heaviest rain.
const MAX_DROPS: u32 = 14000;
/// Flakes in the box around the camera in the heaviest snowfall.
const MAX_FLAKES: u32 = 9000;
/// Metres per side of the box rain falls in around the camera.
const RAIN_BOX: f32 = 30.0;
/// Snow falls slower and shows from further away: a smaller box with the
/// flakes closer together.
const SNOW_BOX: f32 = 24.0;
/// Metres per second the wind blows the rain sideways.
const WIND: [f32; 3] = [1.2, 0.0, 0.6];

pub struct PrecipitationRenderer {
    shader: Shader,
}

impl PrecipitationRenderer {
    pub fn new() -> Self {
        let shader = Shader::new(
            &format!("#version 460 core\n#define VERTEX\n{SOURCE}"),
            &format!("#version 460 core\n#define FRAGMENT\n{LIGHTING_GLSL}\n{SOURCE}"),
        );
        Self { shader }
    }

    /// Draw `amount` (0..1) of `kind` around `eye` into the current pass,
    /// behind what is already in its depth buffer.
    pub fn render(
        &self,
        view_projection: &Matrix4<f32>,
        eye: Point3<f32>,
        kind: Precipitation,
        amount: f32,
        lighting: &Lighting,
        maps: Option<&WeatherMaps>,
    ) {
        let (count, box_size, snowing) = match kind {
            Precipitation::None => return,
            Precipitation::Rain => (MAX_DROPS, RAIN_BOX, false),
            Precipitation::Snow => (MAX_FLAKES, SNOW_BOX, true),
        };
        let count = (count as f32 * amount.clamp(0.0, 1.0)) as u32;
        if count == 0 {
            return;
        }
        let shader = &self.shader;
        // Quads made from `gl_VertexID` and `gl_InstanceID`, blended over
        // the scene, behind what is in the depth buffer.
        let desc = gfx::PipelineDesc {
            depth: gfx::DepthState::READ,
            blend: Some(gfx::BlendState::ALPHA),
            ..Default::default()
        };
        gfx::set_pipeline(shader.pipeline(&desc));
        lighting.apply(shader);
        shader.set_uniform_mat4("viewProjection", view_projection);
        shader.set_uniform_3f("eye", eye.x, eye.y, eye.z);
        shader.set_uniform_1f("time", super::seconds());
        shader.set_uniform_1i("snowing", snowing as i32);
        shader.set_uniform_1f("boxSize", box_size);
        shader.set_uniform_3f("wind", WIND[0], WIND[1], WIND[2]);
        shader.set_uniform_1i("exposureMap", EXPOSURE_UNIT as i32);
        match maps.and_then(|maps| maps.exposure) {
            Some((matrix, _)) => {
                shader.set_uniform_1i("useExposure", 1);
                shader.set_uniform_mat4("exposureMatrix", &matrix);
            }
            None => shader.set_uniform_1i("useExposure", 0),
        }
        gfx::draw_instanced(0, 6, 0, count);
    }
}
