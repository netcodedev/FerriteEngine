//! Screen space ambient occlusion: how much of the sky each pixel sees past
//! the geometry right around it.
//!
//! The renderer is forward, so the occlusion has to be ready before the
//! scene is shaded: the opaque geometry is drawn once more, depth only, from
//! the camera; the occlusion is worked out from that depth and blurred; and
//! the lit shaders darken their ambient light by it (`screenOcclusion` in
//! `pbr/lighting.glsl`), so the sun keeps its full strength.

use std::sync::atomic::{AtomicBool, Ordering};

use cgmath::{Matrix4, SquareMatrix};
use serde::{Deserialize, Serialize};

use super::{light::light::clamp_finite, shader::Shader};
use crate::core::gfx::{self, Format, Rect, SamplerDesc, TargetRef, TextureDesc};

/// Texture unit the occlusion is bound to while the scene is shaded.
pub const SSAO_UNIT: u32 = 14;
/// Samples per pixel; must match `SAMPLES` in `ssao.glsl`.
const SAMPLES: usize = 16;
/// How the depth and the occlusion are read: texel for texel, clamped at
/// the edges.
const SAMPLER: SamplerDesc = SamplerDesc::NEAREST_CLAMP;

/// Tunables of the occlusion, kept with the sky light.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct AmbientOcclusionSettings {
    pub enabled: bool,
    /// How far around a point geometry occludes it, in world units.
    pub radius: f32,
    /// Exponent on the occlusion: above 1 darkens corners further.
    pub intensity: f32,
}

impl Default for AmbientOcclusionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 1.0,
            intensity: 1.5,
        }
    }
}

impl AmbientOcclusionSettings {
    pub const RADIUS_RANGE: (f32, f32) = (0.1, 4.0);
    pub const INTENSITY_RANGE: (f32, f32) = (0.25, 4.0);

    pub fn set_radius(&mut self, radius: f32) {
        self.radius = clamp_finite(radius, Self::RADIUS_RANGE, 1.0);
    }

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = clamp_finite(intensity, Self::INTENSITY_RANGE, 1.5);
    }
}

/// Set while the camera's depth is drawn for the occlusion. Shaders see a
/// framebuffer of their own then, as in the shadow pass, but what is drawn
/// has to match what the camera shows (no hidden first person bones).
static DEPTH_PASS: AtomicBool = AtomicBool::new(false);

/// Whether the camera's depth is being drawn for the occlusion right now.
pub fn in_depth_pass() -> bool {
    DEPTH_PASS.load(Ordering::Relaxed)
}

pub struct Ssao {
    occlusion_shader: Shader,
    blur_shader: Shader,
    /// Fullscreen triangle strip.
    quad: gfx::Buffer,
    targets: Option<Targets>,
}

/// A texture with a render target drawing into it.
struct Image {
    texture: gfx::Texture,
    target: gfx::RenderTarget,
}

impl Image {
    fn new(width: i32, height: i32, format: Format) -> Self {
        let texture = gfx::Texture::new(TextureDesc::d2(format, width as u32, height as u32));
        let attachment = texture.attachment(0, None);
        let target = if format.is_depth() {
            gfx::RenderTarget::new(&[], Some(attachment))
        } else {
            gfx::RenderTarget::new(&[attachment], None)
        };
        Self { texture, target }
    }
}

/// The offscreen images, as big as the viewport.
struct Targets {
    width: i32,
    height: i32,
    /// As the HDR view's depth, so it can be copied from there.
    depth: Image,
    occlusion: Image,
    blurred: Image,
}

impl Targets {
    fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            depth: Image::new(width, height, Format::Depth24Stencil8),
            occlusion: Image::new(width, height, Format::R8),
            blurred: Image::new(width, height, Format::R8),
        }
    }

    /// A pass drawing into `image`, contents kept.
    fn begin(&self, image: &Image) {
        gfx::begin_pass(
            image.target.reference(),
            (self.width as u32, self.height as u32),
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
    }
}

/// The state the occlusion passes draw with: the fullscreen strip, no
/// depth, no blending.
fn pass_pipeline() -> gfx::PipelineDesc {
    gfx::PipelineDesc {
        vertex_layouts: vec![gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x2])],
        topology: gfx::Topology::TriangleStrip,
        depth: gfx::DepthState::OFF,
        ..Default::default()
    }
}

impl Ssao {
    pub fn new() -> Self {
        let vertex = include_str!("vertex.glsl");
        let occlusion_shader = Shader::new(vertex, include_str!("ssao.glsl"));
        let blur_shader = Shader::new(vertex, include_str!("blur.glsl"));

        // Uniforms that never change, set on the program once.
        occlusion_shader.bind();
        for (i, sample) in sample_kernel().iter().enumerate() {
            occlusion_shader.set_uniform_3f(&format!("kernel[{i}]"), sample[0], sample[1], sample[2]);
        }

        let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let quad = gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &vertices);
        Self {
            occlusion_shader,
            blur_shader,
            quad,
            targets: None,
        }
    }

    /// Draw depth only into the current target (the HDR view, cleared):
    /// the opaque scene from the camera goes in after this, cheaply (the
    /// lit shaders take their shadow pass path), and stays there as the
    /// depth the shading pass tests against, so each pixel is shaded once.
    /// Then call `take_depth`.
    pub fn begin_depth_pass(&mut self) {
        // No colour, depth tested `LESS` and written: the state code that
        // does not draw through pipelines yet finds (pipelines drawn in
        // the pass get it from the `DepthPass` overrides).
        let desc = gfx::PipelineDesc {
            color_write: false,
            depth: gfx::DepthState::LESS,
            ..pass_pipeline()
        };
        gfx::set_pipeline(self.occlusion_shader.pipeline(&desc));
        DEPTH_PASS.store(true, Ordering::Relaxed);
    }

    /// End the depth pass and copy the depth of `target` (`width` ×
    /// `height`, depth and stencil as `Depth24Stencil8`, possibly
    /// multisampled) for `resolve`. The pass on `target` goes on.
    pub fn take_depth(&mut self, target: TargetRef, width: i32, height: i32) {
        DEPTH_PASS.store(false, Ordering::Relaxed);
        let (width, height) = (width.max(1), height.max(1));
        if self
            .targets
            .as_ref()
            .map_or(true, |t| (t.width, t.height) != (width, height))
        {
            self.targets = None;
            self.targets = Some(Targets::new(width, height));
        }
        let targets = self.targets.as_ref().expect("just created");
        let rect = Rect::sized(width as u32, height as u32);
        gfx::blit(target, rect, targets.depth.target.reference(), rect, gfx::Planes::DEPTH, false);
    }

    /// Work out the occlusion from the depth taken by `take_depth` and
    /// blur it, in passes of their own: the pass running before goes on
    /// after, with its viewport over all of its target.
    pub fn resolve(&self, projection: &Matrix4<f32>, settings: &AmbientOcclusionSettings) {
        DEPTH_PASS.store(false, Ordering::Relaxed);
        let Some(targets) = &self.targets else {
            return;
        };
        let inverse = projection.invert().unwrap_or_else(Matrix4::identity);

        targets.begin(&targets.occlusion);
        let shader = &self.occlusion_shader;
        gfx::set_pipeline(shader.pipeline(&pass_pipeline()));
        gfx::bind_texture(0, &targets.depth.texture, &SAMPLER);
        shader.set_uniform_1i("depthMap", 0);
        shader.set_uniform_mat4("projection", projection);
        shader.set_uniform_mat4("inverseProjection", &inverse);
        shader.set_uniform_1f("radius", settings.radius);
        shader.set_uniform_1f("intensity", settings.intensity);
        self.draw_quad();
        gfx::end_pass();

        targets.begin(&targets.blurred);
        let shader = &self.blur_shader;
        gfx::set_pipeline(shader.pipeline(&pass_pipeline()));
        gfx::bind_texture(1, &targets.occlusion.texture, &SAMPLER);
        shader.set_uniform_1i("depthMap", 0);
        shader.set_uniform_1i("occlusionMap", 1);
        shader.set_uniform_mat4("inverseProjection", &inverse);
        self.draw_quad();
        gfx::end_pass();

        gfx::unbind_texture(1, gfx::TextureKind::D2);
        gfx::unbind_texture(0, gfx::TextureKind::D2);
    }

    /// Draw the fullscreen strip with the pipeline set.
    fn draw_quad(&self) {
        gfx::set_vertex_buffer(0, &self.quad, 0, 8);
        gfx::draw(0, 4);
    }

    /// Bind the blurred occlusion to `SSAO_UNIT` for the shading pass.
    pub fn bind_result(&self) {
        if let Some(targets) = &self.targets {
            gfx::bind_texture(SSAO_UNIT, &targets.blurred.texture, &SAMPLER);
        }
    }

    /// Raw id of the depth taken by `take_depth` (the opaque scene from
    /// the camera), read texel for texel.
    pub fn depth_texture(&self) -> Option<gfx::RawId> {
        self.targets.as_ref().map(|t| t.depth.texture.raw())
    }

    /// Raw id (GL name) of the blurred occlusion, for debug views.
    pub fn result_texture(&self) -> Option<u32> {
        self.targets.as_ref().map(|t| t.blurred.texture.raw() as u32)
    }
}

/// Sample offsets in the hemisphere above +Z, within the unit sphere and
/// more of them close to the centre, where occluders matter most. Always
/// the same, so the occlusion does not shimmer between runs.
fn sample_kernel() -> [[f32; 3]; SAMPLES] {
    let mut state: u32 = 0x9E37_79B9;
    let mut random = move || {
        // xorshift32
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };
    std::array::from_fn(|i| {
        let v = [random() * 2.0 - 1.0, random() * 2.0 - 1.0, random().max(0.05)];
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-4);
        let t = i as f32 / SAMPLES as f32;
        let scale = (0.1 + 0.9 * t * t) * random().max(0.2) / length;
        [v[0] * scale, v[1] * scale, v[2] * scale]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_stays_in_the_upper_unit_hemisphere() {
        for [x, y, z] in sample_kernel() {
            assert!(z > 0.0);
            assert!((x * x + y * y + z * z).sqrt() <= 1.0 + 1e-5);
        }
    }

    #[test]
    fn settings_are_clamped_to_their_ranges() {
        let mut settings = AmbientOcclusionSettings::default();
        settings.set_radius(100.0);
        settings.set_intensity(0.0);
        assert_eq!(settings.radius, AmbientOcclusionSettings::RADIUS_RANGE.1);
        assert_eq!(settings.intensity, AmbientOcclusionSettings::INTENSITY_RANGE.0);
    }
}
