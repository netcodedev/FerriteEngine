//! Rain and snow falling around the camera.

use cgmath::{Matrix4, Point3};
use gl::types::*;

use super::{Precipitation, WeatherMaps, EXPOSURE_UNIT};
use crate::core::renderer::{
    pbr::{Lighting, LIGHTING_GLSL},
    shader::Shader,
};

const SOURCE: &str = include_str!("precipitation.glsl");
/// Drops in the box around the camera in the heaviest rain.
const MAX_DROPS: i32 = 14000;
/// Flakes in the box around the camera in the heaviest snowfall.
const MAX_FLAKES: i32 = 9000;
/// Metres per side of the box rain falls in around the camera.
const RAIN_BOX: f32 = 30.0;
/// Snow falls slower and shows from further away: a smaller box with the
/// flakes closer together.
const SNOW_BOX: f32 = 24.0;
/// Metres per second the wind blows the rain sideways.
const WIND: [f32; 3] = [1.2, 0.0, 0.6];

pub struct PrecipitationRenderer {
    shader: Shader,
    vao: GLuint,
}

impl PrecipitationRenderer {
    pub fn new() -> Self {
        let shader = Shader::new(
            &format!("#version 460 core\n#define VERTEX\n{SOURCE}"),
            &format!("#version 460 core\n#define FRAGMENT\n{LIGHTING_GLSL}\n{SOURCE}"),
        );
        let mut vao = 0;
        unsafe { gl::GenVertexArrays(1, &mut vao) };
        Self { shader, vao }
    }

    /// Draw `amount` (0..1) of `kind` around `eye` into the bound target,
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
        let count = (count as f32 * amount.clamp(0.0, 1.0)) as i32;
        if count == 0 {
            return;
        }
        let shader = &self.shader;
        shader.bind();
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
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::FALSE);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(self.vao);
            gl::DrawArraysInstanced(gl::TRIANGLES, 0, 6, count);
            gl::BindVertexArray(0);
            gl::Disable(gl::BLEND);
            gl::DepthMask(gl::TRUE);
        }
    }
}

impl Drop for PrecipitationRenderer {
    fn drop(&mut self) {
        unsafe { gl::DeleteVertexArrays(1, &self.vao) };
    }
}
