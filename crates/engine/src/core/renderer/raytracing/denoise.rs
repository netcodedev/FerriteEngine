//! Denoising of a per-pixel RGB result traced once per pixel (the lamps'
//! shadow share, the ambient occlusion): blended into the frames before,
//! moved with the camera (`light_accumulate.glsl`), then blurred a little
//! within a surface and limited to 0..1 (`light_filter.glsl`).

use cgmath::Matrix4;

use super::{fullscreen, ShadowView};
use crate::core::{
    gfx::{self, Format, SamplerDesc, TextureDesc},
    renderer::{shader::Shader, taa},
};

/// A fullscreen image and the target to draw into it.
pub struct Image {
    pub texture: gfx::Texture,
    pub target: gfx::RenderTarget,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        let texture = gfx::Texture::new(TextureDesc::d2(Format::Rgba16F, width, height));
        let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
        Self { texture, target }
    }

    /// Begin a pass drawing into the whole image (`size`).
    pub fn begin(&self, size: (u32, u32)) {
        gfx::begin_pass(self.target.reference(), size, gfx::Load::Keep, gfx::Load::Keep);
    }
}

struct Targets {
    size: (u32, u32),
    /// Blended over the frames: this frame's and last frame's.
    history: [Image; 2],
    filtered: Image,
}

pub struct Denoiser {
    accumulate_shader: Shader,
    filter_shader: Shader,
    targets: Option<Targets>,
    newest: usize,
    history_valid: bool,
    previous: Option<Matrix4<f32>>,
    /// Share of this frame where the history is fine.
    current_weight: f32,
}

impl Denoiser {
    pub fn new(current_weight: f32) -> Self {
        let vertex = include_str!("../hdr/vertex.glsl");
        Self {
            accumulate_shader: Shader::new(vertex, include_str!("light_accumulate.glsl")),
            filter_shader: Shader::new(vertex, include_str!("light_filter.glsl")),
            targets: None,
            newest: 0,
            history_valid: false,
            previous: None,
            current_weight,
        }
    }

    pub fn reset(&mut self) {
        self.history_valid = false;
        self.previous = None;
    }

    /// Raw id of the denoised result.
    pub fn result(&self) -> Option<gfx::RawId> {
        self.targets.as_ref().map(|t| t.filtered.texture.raw())
    }

    /// Denoise `raw` (rgb the value, a 0 where nothing was traced) for
    /// `view`, with the fullscreen strip `quad`; the result stays in
    /// `result` (and is bound to `unit`, if given).
    pub fn run(&mut self, raw: &gfx::Texture, view: &ShadowView, quad: &dyn Fn(), unit: Option<u32>) {
        let size = (view.width.max(1) as u32, view.height.max(1) as u32);
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = None;
            self.targets = Some(Targets {
                size,
                history: [Image::new(size.0, size.1), Image::new(size.0, size.1)],
                filtered: Image::new(size.0, size.1),
            });
            self.history_valid = false;
        }
        let targets = self.targets.as_ref().expect("just made");
        let nearest = SamplerDesc::NEAREST_CLAMP;
        let previous = self.previous.unwrap_or(view.view_projection);
        let [wx, wy, wz, ww] = taa::inverse_w(&view.view_projection);
        let next = 1 - self.newest;

        targets.history[next].begin(size);
        let shader = &self.accumulate_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(0, raw, &nearest);
        shader.set_uniform_1i("current", 0);
        gfx::bind_texture(1, &targets.history[self.newest].texture, &nearest);
        shader.set_uniform_1i("history", 1);
        gfx::bind_raw_texture(2, view.depth, gfx::TextureKind::D2, &nearest);
        shader.set_uniform_1i("sceneDepth", 2);
        shader.set_uniform_mat4("reprojection", &taa::reprojection(&view.view_projection, &previous));
        shader.set_uniform_4f("inverseW", wx, wy, wz, ww);
        shader.set_uniform_1i("historyValid", self.history_valid as i32);
        shader.set_uniform_1f("currentWeight", self.current_weight);
        quad();
        gfx::end_pass();

        targets.filtered.begin(size);
        let shader = &self.filter_shader;
        gfx::set_pipeline(shader.pipeline(&fullscreen()));
        gfx::bind_texture(0, &targets.history[next].texture, &nearest);
        shader.set_uniform_1i("accumulated", 0);
        quad();
        gfx::end_pass();
        for unit in 0..3 {
            gfx::unbind_texture(unit, gfx::TextureKind::D2);
        }

        self.newest = next;
        self.history_valid = true;
        self.previous = Some(view.view_projection);
        if let Some(unit) = unit {
            gfx::bind_texture(unit, &targets.filtered.texture, &nearest);
        }
    }
}
