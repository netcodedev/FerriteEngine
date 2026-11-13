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
use gl::types::*;
use serde::{Deserialize, Serialize};

use super::shader::Shader;

/// Texture unit the occlusion is bound to while the scene is shaded.
pub const SSAO_UNIT: u32 = 14;
/// Samples per pixel; must match `SAMPLES` in `ssao.glsl`.
const SAMPLES: usize = 16;

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
        self.radius = radius.clamp(Self::RADIUS_RANGE.0, Self::RADIUS_RANGE.1);
    }

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.clamp(Self::INTENSITY_RANGE.0, Self::INTENSITY_RANGE.1);
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
    vao: GLuint,
    vbo: GLuint,
    targets: Option<Targets>,
}

/// The offscreen images, as big as the viewport.
struct Targets {
    width: i32,
    height: i32,
    depth_framebuffer: GLuint,
    depth: GLuint,
    occlusion_framebuffer: GLuint,
    occlusion: GLuint,
    blur_framebuffer: GLuint,
    blurred: GLuint,
}

impl Targets {
    fn new(width: i32, height: i32) -> Self {
        unsafe {
            let depth = texture(width, height, gl::DEPTH_COMPONENT32F, gl::DEPTH_COMPONENT, gl::FLOAT);
            let occlusion = texture(width, height, gl::R8, gl::RED, gl::UNSIGNED_BYTE);
            let blurred = texture(width, height, gl::R8, gl::RED, gl::UNSIGNED_BYTE);
            let depth_framebuffer = framebuffer(gl::DEPTH_ATTACHMENT, depth);
            gl::DrawBuffer(gl::NONE);
            gl::ReadBuffer(gl::NONE);
            let occlusion_framebuffer = framebuffer(gl::COLOR_ATTACHMENT0, occlusion);
            let blur_framebuffer = framebuffer(gl::COLOR_ATTACHMENT0, blurred);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            Self {
                width,
                height,
                depth_framebuffer,
                depth,
                occlusion_framebuffer,
                occlusion,
                blur_framebuffer,
                blurred,
            }
        }
    }
}

impl Drop for Targets {
    fn drop(&mut self) {
        unsafe {
            let framebuffers = [
                self.depth_framebuffer,
                self.occlusion_framebuffer,
                self.blur_framebuffer,
            ];
            gl::DeleteFramebuffers(framebuffers.len() as GLsizei, framebuffers.as_ptr());
            let textures = [self.depth, self.occlusion, self.blurred];
            gl::DeleteTextures(textures.len() as GLsizei, textures.as_ptr());
        }
    }
}

/// A texture sampled texel for texel, at the edges clamped.
unsafe fn texture(width: i32, height: i32, internal: GLenum, format: GLenum, kind: GLenum) -> GLuint {
    let mut id = 0;
    gl::GenTextures(1, &mut id);
    gl::BindTexture(gl::TEXTURE_2D, id);
    gl::TexImage2D(
        gl::TEXTURE_2D,
        0,
        internal as GLint,
        width,
        height,
        0,
        format,
        kind,
        std::ptr::null(),
    );
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as GLint);
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as GLint);
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as GLint);
    gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as GLint);
    gl::BindTexture(gl::TEXTURE_2D, 0);
    id
}

/// A framebuffer drawing into `texture`; left bound.
unsafe fn framebuffer(attachment: GLenum, texture: GLuint) -> GLuint {
    let mut id = 0;
    gl::GenFramebuffers(1, &mut id);
    gl::BindFramebuffer(gl::FRAMEBUFFER, id);
    gl::FramebufferTexture2D(gl::FRAMEBUFFER, attachment, gl::TEXTURE_2D, texture, 0);
    id
}

impl Ssao {
    pub fn new() -> Self {
        let vertex = include_str!("vertex.glsl");
        let occlusion_shader = Shader::new(vertex, include_str!("ssao.glsl"));
        let blur_shader = Shader::new(vertex, include_str!("blur.glsl"));

        occlusion_shader.bind();
        for (i, sample) in sample_kernel().iter().enumerate() {
            occlusion_shader.set_uniform_3f(&format!("kernel[{i}]"), sample[0], sample[1], sample[2]);
        }

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
            gl::UseProgram(0);
        }
        Self {
            occlusion_shader,
            blur_shader,
            vao,
            vbo,
            targets: None,
        }
    }

    /// Bind and clear the depth target for a `width` × `height` view. Draw
    /// the opaque scene from the camera after this, then call `resolve`.
    pub fn begin_depth_pass(&mut self, width: i32, height: i32) {
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
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, targets.depth_framebuffer);
            gl::Viewport(0, 0, width, height);
            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
            gl::Clear(gl::DEPTH_BUFFER_BIT);
        }
        DEPTH_PASS.store(true, Ordering::Relaxed);
    }

    /// Work out the occlusion from the depth drawn since `begin_depth_pass`
    /// and blur it. Leaves the framebuffer unbound and the viewport for the
    /// caller to restore.
    pub fn resolve(&self, projection: &Matrix4<f32>, settings: &AmbientOcclusionSettings) {
        DEPTH_PASS.store(false, Ordering::Relaxed);
        let Some(targets) = &self.targets else {
            return;
        };
        let inverse = projection.invert().unwrap_or_else(Matrix4::identity);
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
            gl::DepthMask(gl::FALSE);
            gl::Disable(gl::BLEND);
            gl::BindVertexArray(self.vao);

            gl::BindFramebuffer(gl::FRAMEBUFFER, targets.occlusion_framebuffer);
            let shader = &self.occlusion_shader;
            shader.bind();
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, targets.depth);
            shader.set_uniform_1i("depthMap", 0);
            shader.set_uniform_mat4("projection", projection);
            shader.set_uniform_mat4("inverseProjection", &inverse);
            shader.set_uniform_1f("radius", settings.radius);
            shader.set_uniform_1f("intensity", settings.intensity);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

            gl::BindFramebuffer(gl::FRAMEBUFFER, targets.blur_framebuffer);
            let shader = &self.blur_shader;
            shader.bind();
            gl::ActiveTexture(gl::TEXTURE1);
            gl::BindTexture(gl::TEXTURE_2D, targets.occlusion);
            shader.set_uniform_1i("depthMap", 0);
            shader.set_uniform_1i("occlusionMap", 1);
            shader.set_uniform_mat4("inverseProjection", &inverse);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
        }
    }

    /// Bind the blurred occlusion to `SSAO_UNIT` for the shading pass.
    pub fn bind_result(&self) {
        if let Some(targets) = &self.targets {
            unsafe {
                gl::ActiveTexture(gl::TEXTURE0 + SSAO_UNIT);
                gl::BindTexture(gl::TEXTURE_2D, targets.blurred);
                gl::ActiveTexture(gl::TEXTURE0);
            }
        }
    }

    /// The blurred occlusion, for debug views.
    pub fn result_texture(&self) -> Option<GLuint> {
        self.targets.as_ref().map(|t| t.blurred)
    }
}

impl Drop for Ssao {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
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
