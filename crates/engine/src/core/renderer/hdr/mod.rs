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
    sync::atomic::{AtomicU32, Ordering},
    time::Instant,
};

use gl::types::*;
use serde::{Deserialize, Serialize};

use super::shader::Shader;

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
const COLOR_FORMAT: GLenum = gl::RGBA16F;
const BLOOM_FORMAT: GLenum = gl::R11F_G11F_B10F;
/// Most bloom levels below the scene's resolution, each half the size of
/// the one above.
const BLOOM_LEVELS: usize = 6;
/// Size of the log luminance target the average brightness is taken from.
const LUMINANCE_SIZE: i32 = 256;
/// Average scene luminance (log2) the eye adaptation stays within, so a
/// night stays dark and a sunlit field does not go grey.
const ADAPTATION_RANGE: (f32, f32) = (-5.0, 3.0);
/// Where the auto exposure puts the average luminance. Chosen so a daylit
/// scene comes out about as bright as with a fixed exposure of 0 EV.
const EXPOSURE_KEY: f32 = 0.4;
/// Bloom threshold softness, as a share of the threshold.
const BLOOM_KNEE: f32 = 0.5;

/// GL name of the HDR framebuffer while the camera's view is drawn into
/// it, 0 otherwise.
static VIEW_FRAMEBUFFER: AtomicU32 = AtomicU32::new(0);

fn bound_draw_framebuffer() -> u32 {
    let mut bound = 0;
    unsafe { gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut bound) };
    bound as u32
}

/// Whether what is drawn now ends up in the camera's view: the HDR target
/// while the scene is drawn into it, or the default framebuffer (previews
/// that draw straight to the window). Shadow maps, depth captures and the
/// editor's selection mask are their own framebuffers and return false.
pub fn draws_to_view() -> bool {
    let bound = bound_draw_framebuffer();
    bound == 0 || bound == VIEW_FRAMEBUFFER.load(Ordering::Relaxed)
}

/// Whether the shader drawing now should write linear, unclamped light
/// (into the HDR target), instead of tone mapped display colour.
pub fn writes_linear() -> bool {
    let view = VIEW_FRAMEBUFFER.load(Ordering::Relaxed);
    view != 0 && bound_draw_framebuffer() == view
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
            auto_exposure: false,
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
        self.exposure = exposure.clamp(Self::EXPOSURE_RANGE.0, Self::EXPOSURE_RANGE.1);
    }

    pub fn set_adaptation_speed(&mut self, speed: f32) {
        self.adaptation_speed =
            speed.clamp(Self::ADAPTATION_SPEED_RANGE.0, Self::ADAPTATION_SPEED_RANGE.1);
    }

    pub fn set_bloom_intensity(&mut self, intensity: f32) {
        self.bloom_intensity =
            intensity.clamp(Self::BLOOM_INTENSITY_RANGE.0, Self::BLOOM_INTENSITY_RANGE.1);
    }

    pub fn set_bloom_threshold(&mut self, threshold: f32) {
        self.bloom_threshold =
            threshold.clamp(Self::BLOOM_THRESHOLD_RANGE.0, Self::BLOOM_THRESHOLD_RANGE.1);
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

/// A texture with a framebuffer drawing into it.
struct Target {
    framebuffer: GLuint,
    texture: GLuint,
    width: i32,
    height: i32,
}

impl Target {
    fn new(width: i32, height: i32, format: GLenum, mipmaps: bool) -> Self {
        let (mut framebuffer, mut texture) = (0, 0);
        unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            let levels = if mipmaps { mip_levels(width, height) } else { 1 };
            gl::TexStorage2D(gl::TEXTURE_2D, levels, format, width, height);
            let min_filter = if mipmaps { gl::LINEAR_MIPMAP_LINEAR } else { gl::LINEAR };
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, min_filter as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::BindTexture(gl::TEXTURE_2D, 0);

            gl::GenFramebuffers(1, &mut framebuffer);
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture,
                0,
            );
            gl::DrawBuffer(gl::COLOR_ATTACHMENT0);
        }
        Self {
            framebuffer,
            texture,
            width,
            height,
        }
    }

    unsafe fn bind(&self) {
        gl::BindFramebuffer(gl::FRAMEBUFFER, self.framebuffer);
        gl::Viewport(0, 0, self.width, self.height);
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteFramebuffers(1, &self.framebuffer);
            gl::DeleteTextures(1, &self.texture);
        }
    }
}

fn mip_levels(width: i32, height: i32) -> i32 {
    32 - (width.max(height).max(1) as u32).leading_zeros() as i32
}

/// The view's targets, as big as the viewport.
struct ViewTargets {
    width: i32,
    height: i32,
    samples: i32,
    /// Multisampled colour and depth the scene is drawn into.
    framebuffer: GLuint,
    color_buffer: GLuint,
    depth_buffer: GLuint,
    /// The resolved colour and depth.
    resolve_framebuffer: GLuint,
    color: GLuint,
    depth: GLuint,
    /// Bloom chain, half the view's size and down from there.
    bloom: Vec<Target>,
}

impl ViewTargets {
    fn new(width: i32, height: i32, samples: i32) -> Self {
        let (mut framebuffer, mut color_buffer, mut depth_buffer) = (0, 0, 0);
        let (mut resolve_framebuffer, mut color, mut depth) = (0, 0, 0);
        unsafe {
            gl::GenRenderbuffers(1, &mut color_buffer);
            gl::BindRenderbuffer(gl::RENDERBUFFER, color_buffer);
            gl::RenderbufferStorageMultisample(gl::RENDERBUFFER, samples, COLOR_FORMAT, width, height);
            gl::GenRenderbuffers(1, &mut depth_buffer);
            gl::BindRenderbuffer(gl::RENDERBUFFER, depth_buffer);
            gl::RenderbufferStorageMultisample(
                gl::RENDERBUFFER,
                samples,
                gl::DEPTH24_STENCIL8,
                width,
                height,
            );
            gl::BindRenderbuffer(gl::RENDERBUFFER, 0);

            gl::GenFramebuffers(1, &mut framebuffer);
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferRenderbuffer(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::RENDERBUFFER,
                color_buffer,
            );
            gl::FramebufferRenderbuffer(
                gl::FRAMEBUFFER,
                gl::DEPTH_STENCIL_ATTACHMENT,
                gl::RENDERBUFFER,
                depth_buffer,
            );
            gl::DrawBuffer(gl::COLOR_ATTACHMENT0);
            check_framebuffer("HDR view");

            gl::GenTextures(1, &mut color);
            gl::BindTexture(gl::TEXTURE_2D, color);
            gl::TexStorage2D(gl::TEXTURE_2D, 1, COLOR_FORMAT, width, height);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::GenTextures(1, &mut depth);
            gl::BindTexture(gl::TEXTURE_2D, depth);
            gl::TexStorage2D(gl::TEXTURE_2D, 1, gl::DEPTH24_STENCIL8, width, height);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::BindTexture(gl::TEXTURE_2D, 0);

            gl::GenFramebuffers(1, &mut resolve_framebuffer);
            gl::BindFramebuffer(gl::FRAMEBUFFER, resolve_framebuffer);
            gl::FramebufferTexture2D(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, color, 0);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::DEPTH_STENCIL_ATTACHMENT,
                gl::TEXTURE_2D,
                depth,
                0,
            );
            gl::DrawBuffer(gl::COLOR_ATTACHMENT0);
            check_framebuffer("HDR resolve");
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        }

        let mut bloom = Vec::new();
        let (mut w, mut h) = (width, height);
        while bloom.len() < BLOOM_LEVELS && w > 2 && h > 2 {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            bloom.push(Target::new(w, h, BLOOM_FORMAT, false));
        }

        Self {
            width,
            height,
            samples,
            framebuffer,
            color_buffer,
            depth_buffer,
            resolve_framebuffer,
            color,
            depth,
            bloom,
        }
    }
}

impl Drop for ViewTargets {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteFramebuffers(1, &self.framebuffer);
            gl::DeleteFramebuffers(1, &self.resolve_framebuffer);
            gl::DeleteRenderbuffers(1, &self.color_buffer);
            gl::DeleteRenderbuffers(1, &self.depth_buffer);
            gl::DeleteTextures(1, &self.color);
            gl::DeleteTextures(1, &self.depth);
        }
    }
}

unsafe fn check_framebuffer(name: &str) {
    let status = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
    if status != gl::FRAMEBUFFER_COMPLETE {
        eprintln!("{name} framebuffer incomplete: 0x{status:x}");
    }
}

/// Where the view goes once it is tone mapped.
#[derive(Clone, Copy)]
struct Destination {
    framebuffer: GLuint,
    viewport: [i32; 4],
}

pub struct Hdr {
    downsample_shader: Shader,
    upsample_shader: Shader,
    luminance_shader: Shader,
    adapt_shader: Shader,
    composite_shader: Shader,
    vao: GLuint,
    vbo: GLuint,
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
        let composite_shader = Shader::new(
            vertex,
            &format!(
                "#version 330 core\n{TONEMAP_GLSL}\n{EXPOSURE_GLSL}\n{}",
                include_str!("composite.glsl")
            ),
        );

        // Fullscreen triangle strip.
        let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let (mut vao, mut vbo) = (0, 0);
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                std::mem::size_of_val(&vertices) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                2 * std::mem::size_of::<f32>() as GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }

        let luminance = Target::new(LUMINANCE_SIZE, LUMINANCE_SIZE, gl::R16F, true);
        let adapted = [
            Target::new(1, 1, gl::R32F, false),
            Target::new(1, 1, gl::R32F, false),
        ];
        unsafe {
            for target in &adapted {
                gl::BindTexture(gl::TEXTURE_2D, target.texture);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            }
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            gl::UseProgram(0);
        }

        Self {
            downsample_shader,
            upsample_shader,
            luminance_shader,
            adapt_shader,
            composite_shader,
            vao,
            vbo,
            view: None,
            luminance,
            adapted,
            current_adapted: 0,
            adaptation_fresh: true,
            last_frame: None,
            destination: None,
        }
    }

    /// Draw the camera's view into the HDR target from now on, until
    /// `finish`. The bound framebuffer and viewport are where the view
    /// ends up; the HDR target is as big as the viewport, so window
    /// positions in it are relative to the viewport's corner.
    pub fn begin(&mut self) {
        let mut viewport = [0; 4];
        let mut framebuffer = 0;
        let mut clear = [0.0f32; 4];
        unsafe {
            gl::GetIntegerv(gl::VIEWPORT, viewport.as_mut_ptr());
            gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut framebuffer);
            gl::GetFloatv(gl::COLOR_CLEAR_VALUE, clear.as_mut_ptr());
        }
        let (width, height) = (viewport[2].max(1), viewport[3].max(1));
        let samples = destination_samples();
        if self
            .view
            .as_ref()
            .map_or(true, |v| (v.width, v.height, v.samples) != (width, height, samples))
        {
            self.view = None;
            self.view = Some(ViewTargets::new(width, height, samples));
        }
        let view = self.view.as_ref().expect("just created");
        self.destination = Some(Destination {
            framebuffer: framebuffer as GLuint,
            viewport,
        });
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, view.framebuffer);
            gl::Viewport(0, 0, width, height);
            // The window's clear colour is display colour; the target
            // holds linear light.
            let linear = |c: f32| c.max(0.0).powf(2.2);
            gl::ClearColor(linear(clear[0]), linear(clear[1]), linear(clear[2]), 1.0);
            gl::DepthMask(gl::TRUE);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT | gl::STENCIL_BUFFER_BIT);
            gl::ClearColor(clear[0], clear[1], clear[2], clear[3]);
        }
        VIEW_FRAMEBUFFER.store(view.framebuffer, Ordering::Relaxed);
    }

    /// Resolve what was drawn since `begin`, add bloom, expose and tone map
    /// it into the framebuffer and viewport that were bound at `begin`
    /// (left bound), along with the scene's depth.
    pub fn finish(&mut self, settings: &HdrSettings) {
        VIEW_FRAMEBUFFER.store(0, Ordering::Relaxed);
        let (Some(destination), Some(view)) = (self.destination.take(), self.view.take()) else {
            return;
        };
        self.process(&view, destination, settings);
        self.view = Some(view);
    }

    fn process(&mut self, view: &ViewTargets, destination: Destination, settings: &HdrSettings) {
        let now = Instant::now();
        let delta = self
            .last_frame
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.5));
        self.last_frame = Some(now);

        unsafe {
            // Resolve the samples. Depth has to be resolved with NEAREST.
            gl::BindFramebuffer(gl::READ_FRAMEBUFFER, view.framebuffer);
            gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, view.resolve_framebuffer);
            gl::BlitFramebuffer(
                0,
                0,
                view.width,
                view.height,
                0,
                0,
                view.width,
                view.height,
                gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT,
                gl::NEAREST,
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);

            gl::Disable(gl::DEPTH_TEST);
            gl::DepthMask(gl::FALSE);
            gl::Disable(gl::BLEND);
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(self.vao);
        }

        if settings.auto_exposure {
            let scene = view.color;
            self.adapt(scene, settings, delta);
        } else {
            self.adaptation_fresh = true;
        }
        let adapted = self.adapted[self.current_adapted].texture;
        let bind_exposure = |shader: &Shader| unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + EXPOSURE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, adapted);
            shader.set_uniform_1i("adaptedLuminance", EXPOSURE_UNIT as i32);
            shader.set_uniform_1f("exposureScale", settings.exposure.exp2());
            shader.set_uniform_1i("autoExposure", settings.auto_exposure as i32);
            shader.set_uniform_1f("exposureKey", EXPOSURE_KEY);
        };

        let bloom = settings.bloom && settings.bloom_intensity > 0.0 && !view.bloom.is_empty();
        if bloom {
            self.render_bloom(view, settings, &bind_exposure);
        }

        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, destination.framebuffer);
            let [x, y, w, h] = destination.viewport;
            gl::Viewport(x, y, w, h);
            // The depth goes along, whatever is in the window already.
            let mut depth_func = 0;
            gl::GetIntegerv(gl::DEPTH_FUNC, &mut depth_func);
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::ALWAYS);
            gl::DepthMask(gl::TRUE);

            let shader = &self.composite_shader;
            shader.bind();
            gl::ActiveTexture(gl::TEXTURE0 + SCENE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, view.color);
            shader.set_uniform_1i("hdrColor", SCENE_UNIT as i32);
            gl::ActiveTexture(gl::TEXTURE0 + DEPTH_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, view.depth);
            shader.set_uniform_1i("sceneDepth", DEPTH_UNIT as i32);
            gl::ActiveTexture(gl::TEXTURE0 + BLOOM_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, view.bloom.first().map_or(0, |b| b.texture));
            shader.set_uniform_1i("bloom", BLOOM_UNIT as i32);
            shader.set_uniform_1i("useBloom", bloom as i32);
            shader.set_uniform_1f("bloomIntensity", settings.bloom_intensity);
            shader.set_uniform_1i("tonemapper", settings.tonemapper.index());
            shader.set_uniform_2i("origin", x, y);
            bind_exposure(shader);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

            for unit in [SCENE_UNIT, BLOOM_UNIT, DEPTH_UNIT, EXPOSURE_UNIT] {
                gl::ActiveTexture(gl::TEXTURE0 + unit);
                gl::BindTexture(gl::TEXTURE_2D, 0);
            }
            gl::ActiveTexture(gl::TEXTURE0);
            gl::DepthFunc(depth_func as GLenum);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
        }
    }

    /// Average the scene's log luminance and move the adapted value toward
    /// it.
    fn adapt(&mut self, scene: GLuint, settings: &HdrSettings, delta: f32) {
        let blend = if self.adaptation_fresh {
            1.0
        } else {
            1.0 - (-delta * settings.adaptation_speed).exp()
        };
        self.adaptation_fresh = false;
        let previous = self.current_adapted;
        let next = 1 - previous;
        unsafe {
            self.luminance.bind();
            let shader = &self.luminance_shader;
            shader.bind();
            gl::ActiveTexture(gl::TEXTURE0 + SCENE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, scene);
            shader.set_uniform_1i("hdrColor", SCENE_UNIT as i32);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            gl::BindTexture(gl::TEXTURE_2D, self.luminance.texture);
            gl::GenerateMipmap(gl::TEXTURE_2D);

            self.adapted[next].bind();
            let shader = &self.adapt_shader;
            shader.bind();
            shader.set_uniform_1i("luminance", SCENE_UNIT as i32);
            shader.set_uniform_1i(
                "averageLevel",
                mip_levels(LUMINANCE_SIZE, LUMINANCE_SIZE) - 1,
            );
            gl::ActiveTexture(gl::TEXTURE0 + EXPOSURE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, self.adapted[previous].texture);
            shader.set_uniform_1i("previous", EXPOSURE_UNIT as i32);
            shader.set_uniform_1f("blend", blend);
            shader.set_uniform_2f("range", ADAPTATION_RANGE.0, ADAPTATION_RANGE.1);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0 + SCENE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
        self.current_adapted = next;
    }

    /// Downsample the bright parts of the scene through the bloom chain and
    /// add them back up again; the result ends up in the first level.
    fn render_bloom(&self, view: &ViewTargets, settings: &HdrSettings, bind_exposure: &dyn Fn(&Shader)) {
        unsafe {
            let shader = &self.downsample_shader;
            shader.bind();
            bind_exposure(shader);
            shader.set_uniform_1i("source", SCENE_UNIT as i32);
            let [a, b, c, d] = settings.threshold_curve();
            shader.set_uniform_4f("threshold", a, b, c, d);
            gl::ActiveTexture(gl::TEXTURE0 + SCENE_UNIT);
            let (mut source, mut source_size) = (view.color, (view.width, view.height));
            for (i, level) in view.bloom.iter().enumerate() {
                level.bind();
                gl::BindTexture(gl::TEXTURE_2D, source);
                shader.set_uniform_2f(
                    "sourceTexel",
                    1.0 / source_size.0 as f32,
                    1.0 / source_size.1 as f32,
                );
                shader.set_uniform_1i("firstStep", (i == 0) as i32);
                gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
                source = level.texture;
                source_size = (level.width, level.height);
            }

            let shader = &self.upsample_shader;
            shader.bind();
            shader.set_uniform_1i("source", SCENE_UNIT as i32);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::ONE, gl::ONE);
            gl::BlendEquation(gl::FUNC_ADD);
            for pair in view.bloom.windows(2).rev() {
                let (target, smaller) = (&pair[0], &pair[1]);
                target.bind();
                gl::BindTexture(gl::TEXTURE_2D, smaller.texture);
                shader.set_uniform_2f("radius", 1.0 / target.width as f32, 1.0 / target.height as f32);
                gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            }
            gl::Disable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }
}

impl Default for Hdr {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Hdr {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}

/// Samples per pixel of the bound framebuffer (the window's 8× MSAA), so
/// the view keeps its anti-aliasing; within what the driver supports.
fn destination_samples() -> i32 {
    let (mut samples, mut max) = (0, 0);
    unsafe {
        gl::GetIntegerv(gl::SAMPLES, &mut samples);
        gl::GetIntegerv(gl::MAX_SAMPLES, &mut max);
    }
    samples.clamp(0, max.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

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
