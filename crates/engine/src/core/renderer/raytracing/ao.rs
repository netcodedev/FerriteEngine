//! Ray traced ambient occlusion (graphics setting `effects.raytraced_ao`):
//! instead of SSAO, a few rays per pixel into the hemisphere above the
//! visible surface, as long as the sky light's occlusion radius
//! (`ao_trace.glsl`), denoised over frames and within a surface
//! (`denoise`). The result goes where SSAO's would (`ssao::SSAO_UNIT`), so
//! the lit shaders darken their ambient light by it unchanged.

use super::{bind_surface, denoise::{Denoiser, Image}, fullscreen, RayTracedShadows, ShadowView};
use crate::core::renderer::{shader::Shader, ssao::AmbientOcclusionSettings};

/// Rays per pixel and frame.
const RAYS: i32 = 2;
/// Share of this frame in the blended result where the history is fine.
const CURRENT_WEIGHT: f32 = 0.1;

pub struct RayTracedAo {
    trace_shader: Shader,
    raw: Option<(u32, u32, Image)>,
    frame: u32,
    denoiser: Denoiser,
}

impl RayTracedAo {
    pub fn new() -> Self {
        let vertex = include_str!("../hdr/vertex.glsl");
        Self {
            trace_shader: Shader::new(vertex, &super::pass_source(true, false, include_str!("ao_trace.glsl"))),
            raw: None,
            frame: 0,
            denoiser: Denoiser::new(CURRENT_WEIGHT),
        }
    }

    pub fn reset(&mut self) {
        self.denoiser.reset();
    }

    pub fn result_texture(&self) -> Option<crate::core::gfx::RawId> {
        self.denoiser.result()
    }

    /// Trace and denoise the occlusion of `view` through `scene`'s
    /// structure, and bind it to `unit` for the shading pass.
    pub fn trace(&mut self, scene: &RayTracedShadows, view: &ShadowView, settings: &AmbientOcclusionSettings, unit: u32) {
        let size = (view.width.max(1) as u32, view.height.max(1) as u32);
        if self.raw.as_ref().is_none_or(|(w, h, _)| (*w, *h) != size) {
            self.raw = Some((size.0, size.1, Image::new(size.0, size.1)));
            self.denoiser.reset();
        }
        self.frame = self.frame.wrapping_add(1);
        let (_, _, raw) = self.raw.as_ref().expect("just made");

        raw.begin(size);
        let shader = &self.trace_shader;
        crate::core::gfx::set_pipeline(shader.pipeline(&fullscreen()));
        bind_surface(shader, view, self.frame);
        scene.bind_rays(shader);
        shader.set_uniform_1f("radius", settings.radius);
        shader.set_uniform_1f("intensity", settings.intensity);
        shader.set_uniform_1i("rays", RAYS);
        scene.draw_quad();
        crate::core::gfx::end_pass();

        let quad = || scene.draw_quad();
        self.denoiser.run(&raw.texture, view, &quad, Some(unit));
    }
}
