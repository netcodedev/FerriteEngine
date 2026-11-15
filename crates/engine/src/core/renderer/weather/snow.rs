//! Trails pressed into the snow around the camera: a map of how far the
//! snow is pressed down, where feet pushed it up around their prints, kept
//! from frame to frame (moved along with the camera) and slowly filled in
//! again while it snows.

use std::time::Instant;

use cgmath::{InnerSpace, Point3};
use gl::types::*;

use super::{Footprint, MAX_FOOTPRINTS, SNOW_TRAIL_UNIT};
use crate::core::renderer::shader::Shader;

/// Texels per side.
const SIZE: i32 = 1024;
/// Metres the map spans per side (about 5 cm per texel).
const EXTENT: f32 = 48.0;
/// The map moves in steps of this many texels: a power of two, so the
/// texels of its mipmaps (which the snow layer reads, `snowDentBlurred`)
/// stay where they were in the world as it moves, up to that level, and
/// what is read from them does not jump.
const STEP: i32 = 32;
/// Seconds of full snowfall that fill a footprint again.
const REFILL_SECONDS: f32 = 90.0;

const VERTEX: &str = "#version 460 core
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}";

struct Target {
    framebuffer: GLuint,
    texture: GLuint,
}

pub struct SnowTrails {
    shader: Shader,
    vao: GLuint,
    targets: [Target; 2],
    /// Which target holds the current trails.
    current: usize,
    /// Texel the map's corner is at, in whole texels from the origin.
    origin: Option<[i32; 2]>,
    last_update: Instant,
}

impl SnowTrails {
    pub fn new() -> Self {
        let shader = Shader::new(
            VERTEX,
            &format!("#version 460 core\n{}", include_str!("snow_update.glsl")),
        );
        let mut vao = 0;
        unsafe { gl::GenVertexArrays(1, &mut vao) };
        let targets = [Self::target(), Self::target()];
        Self {
            shader,
            vao,
            targets,
            current: 0,
            origin: None,
            last_update: Instant::now(),
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
                gl::RG16F as GLint,
                SIZE,
                SIZE,
                0,
                gl::RG,
                gl::FLOAT,
                std::ptr::null(),
            );
            // Mipmapped: the snow layer's coarser rings read the trails
            // blurred to their spacing (`snowDentLod`).
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_MIN_FILTER,
                gl::LINEAR_MIPMAP_LINEAR as GLint,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as GLint);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as GLint);
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
            gl::ClearColor(0.0, 0.0, 0.0, 0.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::GenerateMipmap(gl::TEXTURE_2D);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
        Target { framebuffer, texture }
    }

    /// Move the map along with the camera at `eye`, fill the trails in by
    /// what fell since the last update (`snowfall` 0..1) and press in
    /// `footprints`. Leaves the default framebuffer bound.
    pub fn update(&mut self, eye: Point3<f32>, snowfall: f32, footprints: &[Footprint]) {
        let texel = EXTENT / SIZE as f32;
        let corner = |v: f32| {
            (((v - EXTENT * 0.5) / texel).floor() as i32).div_euclid(STEP) * STEP
        };
        let origin = [corner(eye.x), corner(eye.z)];
        // Nothing carries over from a map far away (or none at all).
        let shift = match self.origin {
            Some(previous) => [origin[0] - previous[0], origin[1] - previous[1]],
            None => [SIZE * 4, SIZE * 4],
        };
        self.origin = Some(origin);
        let dt = self.last_update.elapsed().as_secs_f32().min(1.0);
        self.last_update = Instant::now();

        let (from, to) = (self.current, 1 - self.current);
        let shader = &self.shader;
        shader.bind();
        shader.set_uniform_1i("previous", 0);
        shader.set_uniform_2i("shift", shift[0], shift[1]);
        shader.set_uniform_1f("refill", snowfall * dt / REFILL_SECONDS);
        shader.set_uniform_2f("origin", origin[0] as f32 * texel, origin[1] as f32 * texel);
        shader.set_uniform_1f("texel", texel);
        let footprints = &footprints[..footprints.len().min(MAX_FOOTPRINTS)];
        shader.set_uniform_1i("stampCount", footprints.len() as i32);
        for (i, footprint) in footprints.iter().enumerate() {
            let forward = if footprint.forward.magnitude2() > 1e-8 {
                footprint.forward.normalize()
            } else {
                cgmath::Vector2::new(0.0, 1.0)
            };
            shader.set_uniform_4f(
                &format!("stamps[{i}]"),
                footprint.position.x,
                footprint.position.z,
                forward.x,
                forward.y,
            );
            shader.set_uniform_1f(&format!("stampLength[{i}]"), footprint.length);
        }
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.targets[to].framebuffer);
            gl::Viewport(0, 0, SIZE, SIZE);
            gl::Disable(gl::DEPTH_TEST);
            gl::Disable(gl::BLEND);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, self.targets[from].texture);
            gl::BindVertexArray(self.vao);
            gl::DrawArrays(gl::TRIANGLES, 0, 3);
            gl::BindVertexArray(0);
            gl::BindTexture(gl::TEXTURE_2D, self.targets[to].texture);
            gl::GenerateMipmap(gl::TEXTURE_2D);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::Enable(gl::DEPTH_TEST);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        }
        self.current = to;
    }

    /// World x, z of the map's corner and the metres it spans.
    pub fn area(&self) -> Option<[f32; 3]> {
        let texel = EXTENT / SIZE as f32;
        self.origin
            .map(|[x, z]| [x as f32 * texel, z as f32 * texel, EXTENT])
    }

    pub fn bind(&self) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + SNOW_TRAIL_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, self.targets[self.current].texture);
            gl::ActiveTexture(gl::TEXTURE0);
        }
    }
}

impl Drop for SnowTrails {
    fn drop(&mut self) {
        unsafe {
            for target in &self.targets {
                gl::DeleteFramebuffers(1, &target.framebuffer);
                gl::DeleteTextures(1, &target.texture);
            }
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}
