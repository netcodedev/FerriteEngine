//! Ray traced shadows of point and spot lights (graphics setting
//! `shadows.raytraced_lights`), with ReSTIR DI (Bitterli et al. 2020)
//! choosing the lamp each pixel's one ray goes to.
//!
//! The lit shaders loop over the lights per fragment, so a fullscreen pass
//! cannot hand them one visibility per lamp. It hands them a share per
//! pixel instead (Heitz et al. 2018): the light of the shadow casting
//! lamps that gets past what is in the way, divided by what would arrive
//! with nothing in the way, per colour. `localLights` (`pbr/lighting.glsl`)
//! sums those lamps without shadows and multiplies the sum by it.
//!
//! Each frame, after the depth prepass, fullscreen passes:
//!
//! 1. `restir_initial.glsl`: a few lamps drawn from the pixel's cluster,
//!    one kept in proportion to its light there; last frame's reservoir
//!    (moved with the camera, same surface only) merged in.
//! 2. `restir_spatial.glsl`: neighbours' reservoirs merged in; also the
//!    next frame's history.
//! 3. `light_trace.glsl`: a ray toward a random point on the chosen lamp
//!    (its `radius`), the share worked out from it.
//! 4. `light_accumulate.glsl`, `light_filter.glsl`: blended over frames,
//!    blurred a little within a surface.

use cgmath::Matrix4;

use super::{bind_surface, fullscreen, RayTracedShadows, ShadowView};
use crate::core::{
    gfx::{self, Format, SamplerDesc, TextureDesc},
    renderer::{shader::Shader, taa},
};

/// Texture unit the lit shaders read the lamps' shadows from.
pub const RT_LIGHT_SHADOW_UNIT: u32 = 26;
/// Lamps drawn per pixel and frame before resampling.
const CANDIDATES: i32 = 4;
/// Last frame's reservoir stands for at most this many times this frame's
/// candidates (so a lamp that moved away is not kept for long).
const HISTORY_CAP: f32 = 20.0;
/// Neighbours merged in the spatial pass, from within this many pixels.
const NEIGHBOURS: i32 = 4;
const SPATIAL_RADIUS: f32 = 16.0;
/// A neighbour stands for at most this many candidates.
const NEIGHBOUR_CAP: f32 = 4.0 * CANDIDATES as f32;
/// Share of this frame in the blended result where the history is fine
/// (lower than the sun's: one ray among several lamps is noisier).
const CURRENT_WEIGHT: f32 = 0.12;

/// A fullscreen image and the target to draw into it.
struct Image {
    texture: gfx::Texture,
    target: gfx::RenderTarget,
}

impl Image {
    fn new(width: u32, height: u32) -> Self {
        let texture = gfx::Texture::new(TextureDesc::d2(Format::Rgba16F, width, height));
        let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
        Self { texture, target }
    }
}

struct Targets {
    width: i32,
    height: i32,
    /// After the first pass.
    initial: Image,
    /// After the spatial pass: this frame's and last frame's.
    reservoirs: [Image; 2],
    /// The share from this frame's rays.
    raw: Image,
    /// Blended over the frames: this frame's and last frame's.
    history: [Image; 2],
    filtered: Image,
}

impl Targets {
    fn new(width: i32, height: i32) -> Self {
        let (w, h) = (width.max(1) as u32, height.max(1) as u32);
        Self {
            width,
            height,
            initial: Image::new(w, h),
            reservoirs: [Image::new(w, h), Image::new(w, h)],
            raw: Image::new(w, h),
            history: [Image::new(w, h), Image::new(w, h)],
            filtered: Image::new(w, h),
        }
    }
}

pub struct LampShadows {
    initial_shader: Shader,
    spatial_shader: Shader,
    trace_shader: Shader,
    accumulate_shader: Shader,
    filter_shader: Shader,
    targets: Option<Targets>,
    frame: u32,
    /// Which of the two reservoirs and histories is this frame's.
    newest: usize,
    history_valid: bool,
    previous: Option<Matrix4<f32>>,
}

impl LampShadows {
    pub fn new() -> Self {
        let vertex = include_str!("../hdr/vertex.glsl");
        let pass = |rays, body| Shader::new(vertex, &super::pass_source(rays, true, body));
        Self {
            initial_shader: pass(false, include_str!("restir_initial.glsl")),
            spatial_shader: pass(false, include_str!("restir_spatial.glsl")),
            trace_shader: pass(true, include_str!("light_trace.glsl")),
            accumulate_shader: Shader::new(vertex, include_str!("light_accumulate.glsl")),
            filter_shader: Shader::new(vertex, include_str!("light_filter.glsl")),
            targets: None,
            frame: 0,
            newest: 0,
            history_valid: false,
            previous: None,
        }
    }

    pub fn reset(&mut self) {
        self.history_valid = false;
        self.previous = None;
    }

    pub fn result_texture(&self) -> Option<gfx::RawId> {
        self.targets.as_ref().map(|t| t.filtered.texture.raw())
    }

    /// Run the passes for `view`, tracing through `scene`'s structure, and
    /// bind the result to `RT_LIGHT_SHADOW_UNIT`.
    pub fn trace(&mut self, scene: &RayTracedShadows, view: &ShadowView, light_count: usize) {
        if self
            .targets
            .as_ref()
            .is_none_or(|t| (t.width, t.height) != (view.width, view.height))
        {
            self.targets = None;
            self.targets = Some(Targets::new(view.width, view.height));
            self.history_valid = false;
        }
        self.frame = self.frame.wrapping_add(1);
        let targets = self.targets.as_ref().expect("just made");
        let size = (targets.width.max(1) as u32, targets.height.max(1) as u32);
        let previous = self.previous.unwrap_or(view.view_projection);
        let reprojection = taa::reprojection(&view.view_projection, &previous);
        let [wx, wy, wz, ww] = taa::inverse_w(&view.view_projection);
        let next = 1 - self.newest;
        let lights = light_count.min(crate::core::renderer::light::light::MAX_LIGHTS) as i32;
        let begin = |image: &Image| {
            gfx::begin_pass(image.target.reference(), size, gfx::Load::Keep, gfx::Load::Keep);
        };
        let nearest = SamplerDesc::NEAREST_CLAMP;

        // 1. Candidates, and last frame's reservoirs.
        begin(&targets.initial);
        let shader = &self.initial_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        bind_surface(shader, view, self.frame);
        shader.set_uniform_1i("lightCount", lights);
        gfx::bind_texture(1, &targets.reservoirs[self.newest].texture, &nearest);
        shader.set_uniform_1i("history", 1);
        shader.set_uniform_mat4("reprojection", &reprojection);
        shader.set_uniform_4f("inverseW", wx, wy, wz, ww);
        shader.set_uniform_1i("historyValid", self.history_valid as i32);
        shader.set_uniform_1i("candidates", CANDIDATES);
        shader.set_uniform_1f("historyCap", HISTORY_CAP);
        scene.draw_quad();
        gfx::end_pass();
        gfx::unbind_texture(1, gfx::TextureKind::D2);

        // 2. Neighbours.
        begin(&targets.reservoirs[next]);
        let shader = &self.spatial_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        bind_surface(shader, view, self.frame);
        shader.set_uniform_1i("lightCount", lights);
        gfx::bind_texture(1, &targets.initial.texture, &nearest);
        shader.set_uniform_1i("reservoirs", 1);
        shader.set_uniform_1i("neighbours", NEIGHBOURS);
        shader.set_uniform_1f("spatialRadius", SPATIAL_RADIUS);
        shader.set_uniform_1f("neighbourCap", NEIGHBOUR_CAP);
        scene.draw_quad();
        gfx::end_pass();
        gfx::unbind_texture(1, gfx::TextureKind::D2);

        // 3. One ray per pixel, toward the chosen lamp.
        begin(&targets.raw);
        let shader = &self.trace_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        bind_surface(shader, view, self.frame);
        scene.bind_rays(shader);
        shader.set_uniform_1i("lightCount", lights);
        gfx::bind_texture(2, &targets.reservoirs[next].texture, &nearest);
        shader.set_uniform_1i("reservoirs", 2);
        scene.draw_quad();
        gfx::end_pass();
        gfx::unbind_texture(2, gfx::TextureKind::D2);

        // 4. Into the history.
        begin(&targets.history[next]);
        let shader = &self.accumulate_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(0, &targets.raw.texture, &nearest);
        shader.set_uniform_1i("current", 0);
        gfx::bind_texture(1, &targets.history[self.newest].texture, &nearest);
        shader.set_uniform_1i("history", 1);
        gfx::bind_raw_texture(2, view.depth, gfx::TextureKind::D2, &nearest);
        shader.set_uniform_1i("sceneDepth", 2);
        shader.set_uniform_mat4("reprojection", &reprojection);
        shader.set_uniform_4f("inverseW", wx, wy, wz, ww);
        shader.set_uniform_1i("historyValid", self.history_valid as i32);
        shader.set_uniform_1f("currentWeight", CURRENT_WEIGHT);
        scene.draw_quad();
        gfx::end_pass();

        // 5. A little blur within a surface.
        begin(&targets.filtered);
        let shader = &self.filter_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(0, &targets.history[next].texture, &nearest);
        shader.set_uniform_1i("accumulated", 0);
        scene.draw_quad();
        gfx::end_pass();
        for unit in 0..3 {
            gfx::unbind_texture(unit, gfx::TextureKind::D2);
        }

        self.newest = next;
        self.history_valid = true;
        self.previous = Some(view.view_projection);
        gfx::bind_texture(RT_LIGHT_SHADOW_UNIT, &targets.filtered.texture, &nearest);
    }
}
