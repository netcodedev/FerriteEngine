//! How open the sky is over the ground, smoothed: from the exposure map
//! (everything seen from above) and the ground map (the terrain alone), the
//! share of each spot that nothing well above the ground shelters, averaged
//! over the spot and blurred over about a metre. Snow under trees lies
//! thinner there instead of following every gap in the leaves; the snow
//! layer (`snow_layer.rs`) takes its height from this. It covers the same
//! area as the exposure map and is made whenever that is drawn.

use gl::types::*;

use super::{exposure::SkyExposure, SHELTER_UNIT};
use crate::core::renderer::{framebuffer, shader::Shader};

/// Texels per side: four exposure map texels each (about 63 cm).
const SIZE: i32 = 256;

const VERTEX: &str = "#version 460 core
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}";

/// The open share of each texel, from the exposure and ground maps.
const OPEN: &str = "#version 460 core
uniform sampler2D exposureMap;
uniform sampler2D groundMap;
uniform float exposureRange;
out float open;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    ivec2 size = textureSize(exposureMap, 0);
    // Exposure map texels per texel of this one, on each side.
    int step = size.x / SHELTER_SIZE;
    float sheltered = 0.0;
    for (int j = 0; j < step; ++j) {
        for (int i = 0; i < step; ++i) {
            ivec2 t = at * step + ivec2(i, j);
            float ground = texelFetch(groundMap, t, 0).r;
            float top = texelFetch(exposureMap, t, 0).r;
            // How high the first thing from above is over the ground: only
            // what is well above it (a roof, a crown) shelters.
            float above = ground >= 1.0 ? 0.0 : (ground - top) * exposureRange;
            sheltered += smoothstep(0.5, 1.5, above);
        }
    }
    open = 1.0 - sheltered / float(step * step);
}";

/// One direction of a gaussian blur, `direction` (1, 0) or (0, 1).
const BLUR: &str = "#version 460 core
uniform sampler2D source;
uniform ivec2 direction;
out float result;
void main() {
    ivec2 at = ivec2(gl_FragCoord.xy);
    ivec2 last = textureSize(source, 0) - 1;
    // Sigma of about 1.6 texels (a metre).
    const float WEIGHTS[5] = float[](0.2501, 0.2047, 0.1123, 0.0412, 0.0102);
    float sum = texelFetch(source, at, 0).r * WEIGHTS[0];
    for (int i = 1; i < 5; ++i) {
        sum += texelFetch(source, clamp(at + direction * i, ivec2(0), last), 0).r * WEIGHTS[i];
        sum += texelFetch(source, clamp(at - direction * i, ivec2(0), last), 0).r * WEIGHTS[i];
    }
    result = sum;
}";

struct Target {
    framebuffer: GLuint,
    texture: GLuint,
}

pub struct ShelterMap {
    open: Shader,
    blur: Shader,
    vao: GLuint,
    /// The result is in the second; the first holds the half-blurred map.
    targets: [Target; 2],
}

impl ShelterMap {
    pub fn new() -> Self {
        let open = Shader::new(VERTEX, &OPEN.replace("SHELTER_SIZE", &SIZE.to_string()));
        let blur = Shader::new(VERTEX, BLUR);
        let mut vao = 0;
        unsafe { gl::GenVertexArrays(1, &mut vao) };
        Self {
            open,
            blur,
            vao,
            targets: [Self::target(), Self::target()],
        }
    }

    fn target() -> Target {
        let (mut framebuffer, mut texture) = (0, 0);
        unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::R16F as GLint,
                SIZE,
                SIZE,
                0,
                gl::RED,
                gl::FLOAT,
                std::ptr::null(),
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as GLint);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::GenFramebuffers(1, &mut framebuffer);
            framebuffer::bind(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture,
                0,
            );
            gl::DrawBuffer(gl::COLOR_ATTACHMENT0);
            // Open everywhere until the first update.
            gl::ClearColor(1.0, 1.0, 1.0, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            framebuffer::bind(gl::FRAMEBUFFER, 0);
        }
        Target { framebuffer, texture }
    }

    /// Make the map from `exposure`'s maps, just drawn. Leaves the default
    /// framebuffer bound; the caller restores the viewport.
    pub fn update(&self, exposure: &SkyExposure, range: f32) {
        let (sky, ground) = exposure.textures();
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
            gl::Disable(gl::BLEND);
            gl::Viewport(0, 0, SIZE, SIZE);
            gl::BindVertexArray(self.vao);

            // The open share, into the second target.
            framebuffer::bind(gl::FRAMEBUFFER, self.targets[1].framebuffer);
            self.open.bind();
            self.open.set_uniform_1i("exposureMap", 0);
            self.open.set_uniform_1i("groundMap", 1);
            self.open.set_uniform_1f("exposureRange", range);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, sky);
            gl::ActiveTexture(gl::TEXTURE1);
            gl::BindTexture(gl::TEXTURE_2D, ground);
            gl::DrawArrays(gl::TRIANGLES, 0, 3);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0);

            // Blurred across into the first, then down back into the second.
            self.blur.bind();
            self.blur.set_uniform_1i("source", 0);
            for (from, to, direction) in [(1, 0, (1, 0)), (0, 1, (0, 1))] {
                framebuffer::bind(gl::FRAMEBUFFER, self.targets[to].framebuffer);
                self.blur.set_uniform_2i("direction", direction.0, direction.1);
                gl::BindTexture(gl::TEXTURE_2D, self.targets[from].texture);
                gl::DrawArrays(gl::TRIANGLES, 0, 3);
            }
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindVertexArray(0);
            gl::Enable(gl::DEPTH_TEST);
            framebuffer::bind(gl::FRAMEBUFFER, 0);
        }
    }

    /// Bind the result (the second target) to `SHELTER_UNIT`.
    pub fn bind(&self) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + SHELTER_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, self.targets[1].texture);
            gl::ActiveTexture(gl::TEXTURE0);
        }
    }
}

impl Drop for ShelterMap {
    fn drop(&mut self) {
        unsafe {
            for target in &self.targets {
                framebuffer::delete(&[target.framebuffer]);
                gl::DeleteTextures(1, &target.texture);
            }
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}
