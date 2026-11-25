//! High dynamic range rendering.
//!
//! The camera's view is drawn into a floating point target (multisampled
//! like the window), so light is kept in linear, unbounded units instead of
//! being clamped and gamma corrected by every shader. Afterwards the target
//! is resolved, bright areas bleed into a bloom chain, an exposure (fixed,
//! or adapted to the scene's average brightness) scales the light, and a
//! tone mapping curve brings it into the display's range, with the sRGB
//! transfer applied at the very end (`composite.glsl`).
//!
//! Shaders that draw the view ask `draws_to_view` (instead of checking for
//! the default framebuffer) whether they shade the camera's view, and
//! `writes_linear` whether to leave their colour linear for this pass.

use std::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Instant,
};

use cgmath::Matrix4;
use serde::{Deserialize, Serialize};

use super::{
    light::light::clamp_finite,
    shader::Shader,
    taa::{self, Taa, TaaTargets},
};
use crate::core::gfx::{self, Filter, Format, Rect, SamplerDesc, TargetRef, TextureDesc, Wrap};

/// Shared GLSL: `tonemap`, `linearToSrgb`.
pub const TONEMAP_GLSL: &str = include_str!("tonemap.glsl");
const EXPOSURE_GLSL: &str = include_str!("exposure.glsl");

/// Texture units used by the post passes (they run after the scene, so
/// these only need to stay clear of each other).
const SCENE_UNIT: u32 = 0;
const BLOOM_UNIT: u32 = 1;
const DEPTH_UNIT: u32 = 2;
const EXPOSURE_UNIT: u32 = 3;

/// Format of the scene and bloom targets.
const COLOR_FORMAT: Format = Format::Rgba16F;
const BLOOM_FORMAT: Format = Format::Rg11B10F;
/// Most bloom levels below the scene's resolution, each half the size of
/// the one above.
const BLOOM_LEVELS: usize = 6;
/// Size of the log luminance target the average brightness is taken from.
const LUMINANCE_SIZE: i32 = 256;
/// Average scene luminance (log2) the eye adaptation stays within. The
/// bottom keeps an unlit room from being boosted into daylight (looking
/// into it from outside stays dark), the top lets a sunlit field expose
/// down far enough that the sky does not clip.
const ADAPTATION_RANGE: (f32, f32) = (-2.0, 6.0);
/// Where the auto exposure puts the average luminance, i.e. "middle grey"
/// in the linear scene.
const EXPOSURE_KEY: f32 = 0.25;
/// Bloom threshold softness, as a share of the threshold.
const BLOOM_KNEE: f32 = 0.5;

/// How the resolved scene depth is read: texel for texel.
const DEPTH_SAMPLER: SamplerDesc = SamplerDesc::new(Filter::Nearest, Wrap::Repeat);
/// How the adapted luminance is read: its one texel.
const ADAPTED_SAMPLER: SamplerDesc = SamplerDesc::NEAREST_CLAMP;

/// Raw id of the HDR target while the camera's view is drawn into it, 0
/// otherwise.
static VIEW_TARGET: AtomicU64 = AtomicU64::new(0);
/// A render target an editor picture (a thumbnail) is drawn into: a view
/// in display colour, like the window, not a shadow map. 0: none.
static PICTURE_TARGET: AtomicU64 = AtomicU64::new(0);

/// Count `target` as a view drawn in display colour (as previews in the
/// window are) until `None` is given: for pictures drawn into a target of
/// their own.
pub fn set_picture_target(target: Option<TargetRef>) {
    let raw = match target {
        Some(TargetRef::Target(raw)) => raw,
        _ => 0,
    };
    PICTURE_TARGET.store(raw, Ordering::Relaxed);
}

/// The target drawn into now. Code that does not draw through `gfx`
/// passes yet binds GL framebuffers itself, through `framebuffer::bind`
/// as the GL backend does, so under GL the engine's record of the bound
/// framebuffer is where drawing goes (a target's raw id is its GL name);
/// other backends know it from the passes alone.
fn drawing_into() -> Option<TargetRef> {
    gfx::current_target()
}

/// Raw ids of the resolved colour and depth textures holding the opaque
/// scene, while a transparent pass draws over it (see `capture_opaque`),
/// 0 otherwise.
static OPAQUE_COLOR: AtomicU32 = AtomicU32::new(0);

/// When blurred UI panels last asked for the backdrop (ms since the
/// process started; 0: never).
static BACKDROP_WANTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The blurred picture of the view drawn last, for UI panels.
static BACKDROP: std::sync::Mutex<Option<Backdrop>> = std::sync::Mutex::new(None);

/// A blurred copy of the camera's view as it is on screen: what UI panels
/// with `Plane::blur` show through them. `gl_FragCoord.xy - origin` over
/// `size` is the place in it.
#[derive(Clone, Copy, Debug)]
pub struct Backdrop {
    /// Raw id of the texture (display colour, RGBA8).
    pub texture: u32,
    pub origin: (i32, i32),
    pub size: (u32, u32),
}

fn process_millis() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// A blurred panel is being drawn: keep the backdrop coming. Asked every
/// frame it is drawn; the backdrop stops a moment after it is not.
pub fn want_backdrop() {
    BACKDROP_WANTED.store(process_millis(), Ordering::Relaxed);
}

/// The backdrop of the view drawn last, if a panel asked for it lately.
pub fn backdrop() -> Option<Backdrop> {
    *BACKDROP.lock().unwrap_or_else(|e| e.into_inner())
}

fn backdrop_wanted() -> bool {
    let at = BACKDROP_WANTED.load(Ordering::Relaxed);
    at != 0 && process_millis().saturating_sub(at) < 500
}
static OPAQUE_DEPTH: AtomicU32 = AtomicU32::new(0);

/// A copy of the camera's view as it was before the transparent pass:
/// linear colour and depth, as big as the HDR target (window position
/// `gl_FragCoord.xy` is the texel). Water reads it for what lies under its
/// surface and around it. The textures' raw ids (GL names).
#[derive(Clone, Copy, Debug)]
pub struct OpaqueScene {
    pub color: u32,
    pub depth: u32,
}

/// The opaque scene behind what is drawn now, when the camera's view is
/// being drawn into the HDR target and it was captured.
pub fn opaque_scene() -> Option<OpaqueScene> {
    let color = OPAQUE_COLOR.load(Ordering::Relaxed);
    let depth = OPAQUE_DEPTH.load(Ordering::Relaxed);
    (color != 0 && writes_linear()).then_some(OpaqueScene { color, depth })
}

/// Whether what is drawn now ends up in the camera's view: the HDR target
/// while the scene is drawn into it, or the window (previews that draw
/// straight to it). Shadow maps, depth captures and the editor's selection
/// mask are targets of their own and return false.
pub fn draws_to_view() -> bool {
    // The depth pass draws into the view's target, but only its depth.
    if super::ssao::in_depth_pass() {
        return false;
    }
    match drawing_into() {
        None | Some(TargetRef::Window) => true,
        Some(TargetRef::Target(raw)) => {
            raw == VIEW_TARGET.load(Ordering::Relaxed) || raw == PICTURE_TARGET.load(Ordering::Relaxed)
        }
    }
}

/// Whether the shader drawing now should write linear, unclamped light
/// (into the HDR target), instead of tone mapped display colour.
pub fn writes_linear() -> bool {
    let view = VIEW_TARGET.load(Ordering::Relaxed);
    view != 0 && drawing_into() == Some(TargetRef::Target(view))
}


/// Curve that brings HDR light into the display's range.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tonemapper {
    /// Filmic ACES curve: rich contrast, highlights roll off to white.
    #[default]
    Aces,
    /// John Hable's filmic curve: softer contrast than ACES.
    Filmic,
    /// Simple `c / (1 + c)`: keeps hues, a bit flat.
    Reinhard,
    /// Clamps to 0..1, like rendering without HDR.
    None,
}

impl Tonemapper {
    pub const ALL: [Tonemapper; 4] = [
        Tonemapper::Aces,
        Tonemapper::Filmic,
        Tonemapper::Reinhard,
        Tonemapper::None,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tonemapper::Aces => "ACES",
            Tonemapper::Filmic => "Filmic",
            Tonemapper::Reinhard => "Reinhard",
            Tonemapper::None => "None",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    /// Matches the `TONEMAP_*` constants in `tonemap.glsl`.
    fn index(self) -> i32 {
        match self {
            Tonemapper::Aces => 0,
            Tonemapper::Filmic => 1,
            Tonemapper::Reinhard => 2,
            Tonemapper::None => 3,
        }
    }
}

/// Exposure, tone mapping and bloom, kept with the sky light.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct HdrSettings {
    pub tonemapper: Tonemapper,
    /// Exposure compensation in stops: +1 doubles the brightness.
    pub exposure: f32,
    /// Adapt the exposure to the scene's average brightness over time.
    pub auto_exposure: bool,
    /// How quickly the auto exposure follows (higher is faster).
    pub adaptation_speed: f32,
    pub bloom: bool,
    /// How strongly bright areas glow.
    pub bloom_intensity: f32,
    /// Exposed brightness above which light starts to glow.
    pub bloom_threshold: f32,
}

impl Default for HdrSettings {
    fn default() -> Self {
        Self {
            tonemapper: Tonemapper::Aces,
            exposure: 0.0,
            auto_exposure: true,
            adaptation_speed: 1.5,
            bloom: true,
            bloom_intensity: 0.25,
            bloom_threshold: 1.0,
        }
    }
}

impl HdrSettings {
    pub const EXPOSURE_RANGE: (f32, f32) = (-6.0, 6.0);
    pub const ADAPTATION_SPEED_RANGE: (f32, f32) = (0.1, 10.0);
    pub const BLOOM_INTENSITY_RANGE: (f32, f32) = (0.0, 2.0);
    pub const BLOOM_THRESHOLD_RANGE: (f32, f32) = (0.0, 10.0);

    pub fn set_exposure(&mut self, exposure: f32) {
        self.exposure = clamp_finite(exposure, Self::EXPOSURE_RANGE, 0.0);
    }

    pub fn set_adaptation_speed(&mut self, speed: f32) {
        self.adaptation_speed = clamp_finite(speed, Self::ADAPTATION_SPEED_RANGE, 1.5);
    }

    pub fn set_bloom_intensity(&mut self, intensity: f32) {
        self.bloom_intensity = clamp_finite(intensity, Self::BLOOM_INTENSITY_RANGE, 0.25);
    }

    pub fn set_bloom_threshold(&mut self, threshold: f32) {
        self.bloom_threshold = clamp_finite(threshold, Self::BLOOM_THRESHOLD_RANGE, 1.0);
    }

    /// Copy every field through its setter, so out of range values (from a
    /// hand edited scene file) are clamped.
    pub fn sanitized(self) -> Self {
        let mut settings = self;
        settings.set_exposure(self.exposure);
        settings.set_adaptation_speed(self.adaptation_speed);
        settings.set_bloom_intensity(self.bloom_intensity);
        settings.set_bloom_threshold(self.bloom_threshold);
        settings
    }

    /// `threshold` uniform of `downsample.glsl`.
    fn threshold_curve(&self) -> [f32; 4] {
        let threshold = self.bloom_threshold;
        let knee = (threshold * BLOOM_KNEE).max(1e-4);
        [threshold, threshold - knee, 2.0 * knee, 0.25 / knee]
    }
}

/// A texture with a render target drawing into it.
struct Target {
    texture: gfx::Texture,
    target: gfx::RenderTarget,
    width: i32,
    height: i32,
    /// How it is read.
    sampler: SamplerDesc,
}

impl Target {
    fn new(width: i32, height: i32, format: Format, mipmaps: bool, sampler: SamplerDesc) -> Self {
        let desc = TextureDesc::d2(format, width as u32, height as u32).with_mips(if mipmaps { 0 } else { 1 });
        let texture = gfx::Texture::new(desc);
        let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
        Self { texture, target, width, height, sampler }
    }

    /// A pass drawing into it, contents kept.
    fn begin(&self) {
        gfx::begin_pass(
            self.target.reference(),
            (self.width as u32, self.height as u32),
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
    }

    fn bind_to(&self, unit: u32) {
        gfx::bind_texture(unit, &self.texture, &self.sampler);
    }
}

fn mip_levels(width: i32, height: i32) -> i32 {
    32 - (width.max(height).max(1) as u32).leading_zeros() as i32
}

/// The view's targets, as big as the viewport.
struct ViewTargets {
    width: i32,
    height: i32,
    samples: u32,
    /// Multisampled colour and depth the scene is drawn into.
    target: gfx::RenderTarget,
    _color_buffer: gfx::Texture,
    _depth_buffer: gfx::Texture,
    /// The resolved colour and depth.
    resolve: gfx::RenderTarget,
    color: gfx::Texture,
    depth: gfx::Texture,
    /// Bloom chain, half the view's size and down from there.
    bloom: Vec<Target>,
    /// Motion vectors and history, with TAA on.
    taa: Option<TaaTargets>,
}

impl ViewTargets {
    fn new(width: i32, height: i32, samples: u32, with_taa: bool) -> Self {
        let (w, h) = (width as u32, height as u32);
        let color_buffer = gfx::Texture::new(TextureDesc::d2(COLOR_FORMAT, w, h).with_samples(samples));
        let depth_buffer =
            gfx::Texture::new(TextureDesc::d2(Format::Depth24Stencil8, w, h).with_samples(samples));
        let target = gfx::RenderTarget::new(
            &[color_buffer.attachment(0, None)],
            Some(depth_buffer.attachment(0, None)),
        );

        let color = gfx::Texture::new(TextureDesc::d2(COLOR_FORMAT, w, h));
        let depth = gfx::Texture::new(TextureDesc::d2(Format::Depth24Stencil8, w, h));
        let resolve = gfx::RenderTarget::new(&[color.attachment(0, None)], Some(depth.attachment(0, None)));
        // Sampled as they will be from the start: the water binds them by
        // name before the post passes first do.
        gfx::bind_texture(SCENE_UNIT, &color, &SamplerDesc::LINEAR_CLAMP);
        gfx::bind_texture(SCENE_UNIT, &depth, &DEPTH_SAMPLER);
        gfx::unbind_texture(SCENE_UNIT, gfx::TextureKind::D2);

        let mut bloom = Vec::new();
        let (mut w, mut h) = (width, height);
        while bloom.len() < BLOOM_LEVELS && w > 2 && h > 2 {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            bloom.push(Target::new(w, h, BLOOM_FORMAT, false, SamplerDesc::LINEAR_CLAMP));
        }
        let taa = with_taa.then(|| TaaTargets::new(width, height, &depth));

        Self {
            width,
            height,
            samples,
            target,
            _color_buffer: color_buffer,
            _depth_buffer: depth_buffer,
            resolve,
            color,
            depth,
            bloom,
            taa,
        }
    }

    fn rect(&self) -> Rect {
        Rect::sized(self.width as u32, self.height as u32)
    }

    /// Resolve the samples drawn so far into `color` and `depth`. Depth
    /// has to be resolved with NEAREST.
    fn resolve(&self) {
        gfx::blit(
            self.target.reference(),
            self.rect(),
            self.resolve.reference(),
            self.rect(),
            gfx::Planes::COLOR_DEPTH,
            false,
        );
    }
}

/// Where the view goes once it is tone mapped: the viewport of the pass
/// the HDR pass was begun in.
#[derive(Clone, Copy)]
struct Destination {
    viewport: Rect,
}

pub struct Hdr {
    downsample_shader: Shader,
    upsample_shader: Shader,
    luminance_shader: Shader,
    adapt_shader: Shader,
    composite_shader: Shader,
    /// Fullscreen triangle strip.
    quad: gfx::Buffer,
    view: Option<ViewTargets>,
    luminance: Target,
    /// Adapted log2 luminance, 1×1, ping-ponged: one is read while the
    /// other is written.
    adapted: [Target; 2],
    current_adapted: usize,
    /// No adapted value yet (first frame, or auto exposure just turned on):
    /// jump straight to the scene's average.
    adaptation_fresh: bool,
    last_frame: Option<Instant>,
    destination: Option<Destination>,
    /// Temporal anti-aliasing's state, while it is on.
    taa: Option<Taa>,
    backdrop_shader: Shader,
    blur_shader: Shader,
    /// Quarter size ping-pong targets of the backdrop, for the view size
    /// they were made for.
    backdrop: Option<((i32, i32), [Target; 2])>,
}

/// The state the post passes draw with: the fullscreen strip, no depth,
/// no culling, blending as given.
fn post_pipeline(blend: Option<gfx::BlendState>) -> gfx::PipelineDesc {
    gfx::PipelineDesc {
        vertex_layouts: crate::core::gfx::Layouts::of(&[gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x2])]),
        topology: gfx::Topology::TriangleStrip,
        depth: gfx::DepthState::OFF,
        blend,
        cull: gfx::Cull::None,
        ..Default::default()
    }
}

impl Hdr {
    pub fn new() -> Self {
        let vertex = include_str!("vertex.glsl");
        let with_exposure =
            |body: &str| format!("#version 330 core\n{EXPOSURE_GLSL}\n{body}");
        let downsample_shader = Shader::new(vertex, &with_exposure(include_str!("downsample.glsl")));
        let upsample_shader = Shader::new(vertex, include_str!("upsample.glsl"));
        let luminance_shader = Shader::new(vertex, include_str!("luminance.glsl"));
        let adapt_shader = Shader::new(vertex, include_str!("adapt.glsl"));
        let backdrop_shader = Shader::new(
            vertex,
            &format!(
                "#version 330 core\n{TONEMAP_GLSL}\n{EXPOSURE_GLSL}\n{}",
                include_str!("backdrop.glsl")
            ),
        );
        let blur_shader = Shader::new(vertex, include_str!("blur.glsl"));
        let composite_shader = Shader::new(
            vertex,
            &format!(
                "#version 330 core\n{TONEMAP_GLSL}\n{EXPOSURE_GLSL}\n{}",
                include_str!("composite.glsl")
            ),
        );

        let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let quad = gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &vertices);

        let luminance = Target::new(
            LUMINANCE_SIZE,
            LUMINANCE_SIZE,
            Format::R16F,
            true,
            SamplerDesc::LINEAR_CLAMP.mipmapped(),
        );
        let adapted = [
            Target::new(1, 1, Format::R32F, false, ADAPTED_SAMPLER),
            Target::new(1, 1, Format::R32F, false, ADAPTED_SAMPLER),
        ];

        Self {
            downsample_shader,
            upsample_shader,
            luminance_shader,
            adapt_shader,
            composite_shader,
            backdrop_shader,
            blur_shader,
            backdrop: None,
            quad,
            view: None,
            luminance,
            adapted,
            current_adapted: 0,
            adaptation_fresh: true,
            last_frame: None,
            destination: None,
            taa: None,
        }
    }

    /// Draw the camera's view into the HDR target from now on, until
    /// `finish`: a pass on it begins inside the pass running now, whose
    /// viewport is where the view ends up. The HDR target is as big as the
    /// viewport, so window positions in it are relative to the viewport's
    /// corner.
    pub fn begin(&mut self) {
        let viewport = gfx::viewport();
        // The graphics settings may draw the view smaller (scaled up again
        // in `process`) and with fewer samples.
        let knobs = crate::core::graphics::engine();
        let scaled = |size: u32| ((size.max(1) as f32 * knobs.resolution_scale).round() as i32).max(1);
        let (width, height) = (scaled(viewport.width), scaled(viewport.height));
        // As many samples as the window has (8× MSAA) and the settings
        // allow, so the view keeps its anti-aliasing. TAA smooths the view
        // instead, which is drawn with one sample then.
        let to_window = matches!(gfx::current_target(), Some(TargetRef::Window) | None);
        let with_taa = to_window && knobs.taa;
        let samples = match to_window && !with_taa {
            true => gfx::window_samples().min(knobs.msaa.max(1)),
            false => 1,
        };
        if self.view.as_ref().map_or(true, |v| {
            (v.width, v.height, v.samples, v.taa.is_some()) != (width, height, samples, with_taa)
        }) {
            self.view = None;
            self.view = Some(ViewTargets::new(width, height, samples, with_taa));
            if let Some(taa) = &mut self.taa {
                taa.reset();
            }
        }
        if with_taa {
            self.taa.get_or_insert_with(Taa::new);
        } else {
            self.taa = None;
        }
        let view = self.view.as_ref().expect("just created");
        self.destination = Some(Destination { viewport });
        // The window's clear colour is display colour; the target holds
        // linear light.
        let clear = gfx::window_clear_color();
        let linear = |c: f32| c.max(0.0).powf(2.2);
        gfx::begin_pass(
            view.target.reference(),
            (width as u32, height as u32),
            gfx::Load::Color([linear(clear[0]), linear(clear[1]), linear(clear[2]), 1.0]),
            gfx::Load::Depth(1.0),
        );
        VIEW_TARGET.store(view.target.raw(), Ordering::Relaxed);
        OPAQUE_COLOR.store(0, Ordering::Relaxed);
        OPAQUE_DEPTH.store(0, Ordering::Relaxed);
    }

    /// What the camera's view is drawn with, given its view-projection:
    /// with TAA, shifted by this frame's jitter (and models record their
    /// motion until `finish`); otherwise as it is. Call after `begin`.
    pub fn view_projection(&mut self, view_projection: Matrix4<f32>) -> Matrix4<f32> {
        let (Some(view), Some(taa)) = (self.view.as_ref(), self.taa.as_mut()) else {
            return view_projection;
        };
        if self.destination.is_none() || view.taa.is_none() {
            return view_projection;
        }
        taa::motion::begin_frame();
        taa.begin_frame(view_projection, view.width, view.height)
    }

    /// The target the scene is drawn into, with its size, while it is.
    pub fn view_target(&self) -> Option<(TargetRef, i32, i32)> {
        self.destination.as_ref()?;
        self.view.as_ref().map(|v| (v.target.reference(), v.width, v.height))
    }

    /// Copy what was drawn since `begin` (the opaque scene) where
    /// transparent surfaces can read it (`opaque_scene`), and keep drawing
    /// into the HDR target.
    pub fn capture_opaque(&self) {
        let Some(view) = self.view.as_ref() else {
            return;
        };
        if self.destination.is_none() {
            return;
        }
        view.resolve();
        OPAQUE_COLOR.store(view.color.raw() as u32, Ordering::Relaxed);
        OPAQUE_DEPTH.store(view.depth.raw() as u32, Ordering::Relaxed);
    }

    /// End the pass `begin` began; resolve what was drawn since, add
    /// bloom, expose and tone map it into the pass it was begun in, at the
    /// viewport it had then (left set), along with the scene's depth.
    pub fn finish(&mut self, settings: &HdrSettings) {
        taa::motion::end_frame();
        VIEW_TARGET.store(0, Ordering::Relaxed);
        OPAQUE_COLOR.store(0, Ordering::Relaxed);
        OPAQUE_DEPTH.store(0, Ordering::Relaxed);
        let Some(destination) = self.destination.take() else {
            return;
        };
        gfx::end_pass();
        let Some(view) = self.view.take() else {
            return;
        };
        self.process(&view, destination, settings);
        self.view = Some(view);
    }

    /// Draw the fullscreen strip with the pipeline set.
    fn draw_quad(&self) {
        gfx::set_vertex_buffer(0, &self.quad, 0, 8);
        gfx::draw(0, 4);
    }

    fn process(&mut self, view: &ViewTargets, destination: Destination, settings: &HdrSettings) {
        let now = Instant::now();
        let delta = self
            .last_frame
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.5));
        self.last_frame = Some(now);

        view.resolve();

        let wants_backdrop = backdrop_wanted();
        if wants_backdrop {
            let size = ((view.width / 4).max(1), (view.height / 4).max(1));
            if self.backdrop.as_ref().is_none_or(|(made, _)| *made != size) {
                let make = || Target::new(size.0, size.1, Format::Rgba8, false, SamplerDesc::LINEAR_CLAMP);
                self.backdrop = Some((size, [make(), make()]));
            }
        } else if self.backdrop.take().is_some() {
            *BACKDROP.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }

        // With TAA, this frame blended into the ones before is what the
        // rest works with.
        let quad = &self.quad;
        let draw_quad = || {
            gfx::set_vertex_buffer(0, quad, 0, 8);
            gfx::draw(0, 4);
        };
        let color = match (&mut self.taa, &view.taa) {
            (Some(taa), Some(targets)) => {
                taa.draw_velocity(targets, &view.depth, &draw_quad);
                taa.resolve(targets, &view.color, &view.depth, &draw_quad)
            }
            _ => &view.color,
        };

        if settings.auto_exposure {
            self.adapt(color, settings, delta);
        } else {
            self.adaptation_fresh = true;
        }
        let adapted = &self.adapted[self.current_adapted];
        let bind_exposure = |shader: &Shader| {
            adapted.bind_to(EXPOSURE_UNIT);
            shader.set_uniform_1i("adaptedLuminance", EXPOSURE_UNIT as i32);
            shader.set_uniform_1f("exposureScale", settings.exposure.exp2());
            shader.set_uniform_1i("autoExposure", settings.auto_exposure as i32);
            shader.set_uniform_1f("exposureKey", EXPOSURE_KEY);
        };

        let bloom = settings.bloom && settings.bloom_intensity > 0.0 && !view.bloom.is_empty();
        if bloom {
            self.render_bloom(view, color, settings, &bind_exposure);
        }

        let viewport = destination.viewport;
        if wants_backdrop {
            self.render_backdrop(view, color, settings, &bind_exposure, viewport);
        }

        // Back in the pass the view was begun in, at its viewport. The
        // depth goes along, whatever is in the target already.
        gfx::set_viewport(viewport);
        let desc = gfx::PipelineDesc {
            depth: gfx::DepthState::LESS.func(gfx::CompareFunc::Always),
            ..post_pipeline(None)
        };
        let shader = &self.composite_shader;
        gfx::set_pipeline(shader.pipeline(&desc));
        gfx::bind_texture(SCENE_UNIT, color, &SamplerDesc::LINEAR_CLAMP);
        shader.set_uniform_1i("hdrColor", SCENE_UNIT as i32);
        gfx::bind_texture(DEPTH_UNIT, &view.depth, &DEPTH_SAMPLER);
        shader.set_uniform_1i("sceneDepth", DEPTH_UNIT as i32);
        match view.bloom.first() {
            Some(first) => first.bind_to(BLOOM_UNIT),
            None => gfx::unbind_texture(BLOOM_UNIT, gfx::TextureKind::D2),
        }
        shader.set_uniform_1i("bloom", BLOOM_UNIT as i32);
        shader.set_uniform_1i("useBloom", bloom as i32);
        shader.set_uniform_1f("bloomIntensity", settings.bloom_intensity);
        shader.set_uniform_1i("tonemapper", settings.tonemapper.index());
        shader.set_uniform_2i("origin", viewport.x, viewport.y);
        // Drawn smaller: scaled up, colour filtered, depth the nearest.
        let scaled = (view.width, view.height) != (viewport.width as i32, viewport.height as i32);
        shader.set_uniform_1i("scaled", scaled as i32);
        shader.set_uniform_2f(
            "viewScale",
            view.width as f32 / viewport.width.max(1) as f32,
            view.height as f32 / viewport.height.max(1) as f32,
        );
        bind_exposure(shader);
        self.draw_quad();

        for unit in [SCENE_UNIT, BLOOM_UNIT, DEPTH_UNIT, EXPOSURE_UNIT] {
            gfx::unbind_texture(unit, gfx::TextureKind::D2);
        }
    }

    /// Average the scene's log luminance and move the adapted value toward
    /// it.
    fn adapt(&mut self, color: &gfx::Texture, settings: &HdrSettings, delta: f32) {
        let blend = if self.adaptation_fresh {
            1.0
        } else {
            1.0 - (-delta * settings.adaptation_speed).exp()
        };
        self.adaptation_fresh = false;
        let previous = self.current_adapted;
        let next = 1 - previous;

        self.luminance.begin();
        let shader = &self.luminance_shader;
        gfx::set_pipeline(shader.pipeline(&post_pipeline(None)));
        gfx::bind_texture(SCENE_UNIT, color, &SamplerDesc::LINEAR_CLAMP);
        shader.set_uniform_1i("hdrColor", SCENE_UNIT as i32);
        self.draw_quad();
        gfx::end_pass();
        self.luminance.bind_to(SCENE_UNIT);
        self.luminance.texture.generate_mips();

        self.adapted[next].begin();
        let shader = &self.adapt_shader;
        gfx::set_pipeline(shader.pipeline(&post_pipeline(None)));
        shader.set_uniform_1i("luminance", SCENE_UNIT as i32);
        shader.set_uniform_1i("averageLevel", mip_levels(LUMINANCE_SIZE, LUMINANCE_SIZE) - 1);
        self.adapted[previous].bind_to(EXPOSURE_UNIT);
        shader.set_uniform_1i("previous", EXPOSURE_UNIT as i32);
        shader.set_uniform_1f("blend", blend);
        shader.set_uniform_2f("range", ADAPTATION_RANGE.0, ADAPTATION_RANGE.1);
        self.draw_quad();
        gfx::end_pass();
        gfx::unbind_texture(EXPOSURE_UNIT, gfx::TextureKind::D2);
        gfx::unbind_texture(SCENE_UNIT, gfx::TextureKind::D2);
        self.current_adapted = next;
    }

    /// Downsample the bright parts of the scene through the bloom chain and
    /// add them back up again; the result ends up in the first level.
    /// The view blurred at a quarter of its size, for UI panels (see
    /// `want_backdrop`).
    fn render_backdrop(
        &self,
        view: &ViewTargets,
        color: &gfx::Texture,
        settings: &HdrSettings,
        bind_exposure: &dyn Fn(&Shader),
        viewport: Rect,
    ) {
        let Some((size, targets)) = self.backdrop.as_ref() else {
            return;
        };
        let size = *size;
        let shader = &self.backdrop_shader;
        targets[0].begin();
        gfx::set_pipeline(shader.pipeline(&post_pipeline(None)));
        gfx::bind_texture(SCENE_UNIT, color, &SamplerDesc::LINEAR_CLAMP);
        shader.set_uniform_1i("source", SCENE_UNIT as i32);
        shader.set_uniform_2f("sourceTexel", 1.0 / view.width.max(1) as f32, 1.0 / view.height.max(1) as f32);
        shader.set_uniform_1i("tonemapper", settings.tonemapper.index());
        bind_exposure(shader);
        self.draw_quad();
        gfx::end_pass();
        // Twice across and down, the second time spread wider.
        let shader = &self.blur_shader;
        let texel = (1.0 / size.0 as f32, 1.0 / size.1 as f32);
        for spread in [1.0f32, 2.0] {
            for (from, to, direction) in [(0, 1, (texel.0 * spread, 0.0)), (1, 0, (0.0, texel.1 * spread))] {
                targets[to].begin();
                gfx::set_pipeline(shader.pipeline(&post_pipeline(None)));
                targets[from].bind_to(SCENE_UNIT);
                shader.set_uniform_1i("source", SCENE_UNIT as i32);
                shader.set_uniform_2f("direction", direction.0, direction.1);
                self.draw_quad();
                gfx::end_pass();
            }
        }
        gfx::unbind_texture(SCENE_UNIT, gfx::TextureKind::D2);
        *BACKDROP.lock().unwrap_or_else(|e| e.into_inner()) = Some(Backdrop {
            texture: targets[0].texture.raw() as u32,
            origin: (viewport.x, viewport.y),
            size: (viewport.width, viewport.height),
        });
    }

    fn render_bloom(
        &self,
        view: &ViewTargets,
        color: &gfx::Texture,
        settings: &HdrSettings,
        bind_exposure: &dyn Fn(&Shader),
    ) {
        let shader = &self.downsample_shader;
        let pipeline = shader.pipeline(&post_pipeline(None));
        gfx::set_pipeline(pipeline);
        bind_exposure(shader);
        shader.set_uniform_1i("source", SCENE_UNIT as i32);
        let [a, b, c, d] = settings.threshold_curve();
        shader.set_uniform_4f("threshold", a, b, c, d);
        let mut source_size = (view.width, view.height);
        for (i, level) in view.bloom.iter().enumerate() {
            level.begin();
            gfx::set_pipeline(pipeline);
            match i {
                0 => gfx::bind_texture(SCENE_UNIT, color, &SamplerDesc::LINEAR_CLAMP),
                _ => view.bloom[i - 1].bind_to(SCENE_UNIT),
            }
            shader.set_uniform_2f(
                "sourceTexel",
                1.0 / source_size.0 as f32,
                1.0 / source_size.1 as f32,
            );
            shader.set_uniform_1i("firstStep", (i == 0) as i32);
            self.draw_quad();
            gfx::end_pass();
            source_size = (level.width, level.height);
        }

        let shader = &self.upsample_shader;
        let pipeline = shader.pipeline(&post_pipeline(Some(gfx::BlendState::ADDITIVE)));
        gfx::set_pipeline(pipeline);
        shader.set_uniform_1i("source", SCENE_UNIT as i32);
        for pair in view.bloom.windows(2).rev() {
            let (target, smaller) = (&pair[0], &pair[1]);
            target.begin();
            gfx::set_pipeline(pipeline);
            smaller.bind_to(SCENE_UNIT);
            shader.set_uniform_2f("radius", 1.0 / target.width as f32, 1.0 / target.height as f32);
            self.draw_quad();
            gfx::end_pass();
        }
        gfx::unbind_texture(SCENE_UNIT, gfx::TextureKind::D2);
    }
}

impl Default for Hdr {
    fn default() -> Self {
        Self::new()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_settings_fall_back_to_the_defaults() {
        let settings = HdrSettings {
            exposure: f32::NAN,
            adaptation_speed: f32::NAN,
            bloom_intensity: f32::NAN,
            bloom_threshold: f32::NAN,
            ..HdrSettings::default()
        }
        .sanitized();
        assert_eq!(settings, HdrSettings::default());
    }

    #[test]
    fn tonemappers_round_trip_through_their_names() {
        for tonemapper in Tonemapper::ALL {
            assert_eq!(Tonemapper::from_name(tonemapper.name()), Some(tonemapper));
        }
        assert_eq!(Tonemapper::from_name("nope"), None);
    }

    #[test]
    fn settings_are_clamped_to_their_ranges() {
        let settings = HdrSettings {
            exposure: 100.0,
            adaptation_speed: 0.0,
            bloom_intensity: -1.0,
            bloom_threshold: 50.0,
            ..HdrSettings::default()
        }
        .sanitized();
        assert_eq!(settings.exposure, HdrSettings::EXPOSURE_RANGE.1);
        assert_eq!(settings.adaptation_speed, HdrSettings::ADAPTATION_SPEED_RANGE.0);
        assert_eq!(settings.bloom_intensity, HdrSettings::BLOOM_INTENSITY_RANGE.0);
        assert_eq!(settings.bloom_threshold, HdrSettings::BLOOM_THRESHOLD_RANGE.1);
    }

    #[test]
    fn bloom_threshold_knee_is_continuous() {
        // The soft knee ends where the hard threshold takes over, at the
        // threshold plus the knee, and starts at zero below it.
        let [threshold, low, double_knee, quarter] = HdrSettings::default().threshold_curve();
        let knee = double_knee / 2.0;
        let soft = |brightness: f32| {
            let s = (brightness - low).clamp(0.0, double_knee);
            s * s * quarter
        };
        assert!((soft(threshold + knee) - knee).abs() < 1e-5);
        assert_eq!(soft(low), 0.0);
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let settings: HdrSettings = ron::from_str("(exposure: 1.5)").unwrap();
        assert_eq!(settings.exposure, 1.5);
        assert_eq!(settings.tonemapper, Tonemapper::Aces);
        assert!(settings.bloom);
    }

    #[test]
    fn mip_levels_reach_one_texel() {
        assert_eq!(mip_levels(256, 256), 9);
        assert_eq!(mip_levels(1, 1), 1);
        assert_eq!(mip_levels(300, 2), 9);
    }
}
