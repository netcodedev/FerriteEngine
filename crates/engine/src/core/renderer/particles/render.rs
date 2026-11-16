//! Drawing particles: every emitter's particles packed into one stream of
//! instances and drawn as quads (`particles.glsl`), in the transparent
//! pass over the opaque scene.

use std::cell::RefCell;

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Vector3, Vector4};
use gl::types::*;

use super::{ParticleBlend, ParticleSystem};
use crate::core::{
    renderer::{
        hdr,
        light::light::srgb_to_linear,
        pbr::{Lighting, LIGHTING_GLSL},
        shader::Shader,
    },
    scene::Scene,
};

const SOURCE: &str = include_str!("particles.glsl");
/// Texture unit the opaque scene's depth is bound to (soft particles).
const DEPTH_UNIT: u32 = 11;
/// Floats per instance: four vec4s.
const INSTANCE_FLOATS: usize = 16;

/// The shader and the instance buffer, shared by every emitter.
pub struct ParticleRenderer {
    shader: Shader,
    vao: GLuint,
    vbo: GLuint,
    /// Instances the buffer has room for.
    capacity: usize,
}

thread_local! {
    static RENDERER: RefCell<Option<ParticleRenderer>> = const { RefCell::new(None) };
}

/// Free the particle renderer's GL objects. Must run on the GL thread while
/// the context is still alive: dropped with the thread-local at process
/// exit, it would call into a GL driver that is already unloaded (a crash
/// on closing the window).
pub fn release_gl_resources() {
    let _ = RENDERER.try_with(|renderer| renderer.borrow_mut().take());
}

impl ParticleRenderer {
    fn new() -> Self {
        let shader = Shader::new(
            &format!("#version 460 core\n#define VERTEX\n{SOURCE}"),
            &format!("#version 460 core\n#define FRAGMENT\n{LIGHTING_GLSL}\n{SOURCE}"),
        );
        let (mut vao, mut vbo) = (0, 0);
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            let stride = (INSTANCE_FLOATS * std::mem::size_of::<f32>()) as GLsizei;
            for i in 0..4u32 {
                gl::EnableVertexAttribArray(i);
                gl::VertexAttribPointer(
                    i,
                    4,
                    gl::FLOAT,
                    gl::FALSE,
                    stride,
                    (i as usize * 4 * std::mem::size_of::<f32>()) as *const _,
                );
                gl::VertexAttribDivisor(i, 1);
            }
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        }
        Self {
            shader,
            vao,
            vbo,
            capacity: 0,
        }
    }

    fn upload(&mut self, instances: &[[f32; INSTANCE_FLOATS]]) {
        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            let bytes = std::mem::size_of_val(instances) as isize;
            if instances.len() > self.capacity {
                self.capacity = instances.len().next_power_of_two();
                let size = (self.capacity * INSTANCE_FLOATS * std::mem::size_of::<f32>()) as isize;
                gl::BufferData(gl::ARRAY_BUFFER, size, std::ptr::null(), gl::STREAM_DRAW);
            }
            gl::BufferSubData(gl::ARRAY_BUFFER, 0, bytes, instances.as_ptr() as *const _);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        }
    }
}

impl Drop for ParticleRenderer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteVertexArrays(1, &self.vao);
            gl::DeleteBuffers(1, &self.vbo);
        }
    }
}

/// The particles of `system` as instances, for a camera at `eye`:
/// `transform` takes them to the world (identity for world-space ones).
/// Alpha-blended ones come back to front.
pub fn pack(
    system: &ParticleSystem,
    transform: &Matrix4<f32>,
    eye: cgmath::Point3<f32>,
    out: &mut Vec<[f32; INSTANCE_FLOATS]>,
) {
    out.clear();
    let e = &system.effect;
    for p in &system.particles {
        let t = p.age / p.lifetime;
        let world = transform * p.position.extend(1.0);
        let world = world.truncate() / world.w;
        let velocity = (transform * Vector4::new(p.velocity.x, p.velocity.y, p.velocity.z, 0.0)).truncate();
        let [r, g, b, a] = e.color_at(t);
        out.push([
            world.x,
            world.y,
            world.z,
            e.size_at(t) * p.scale,
            srgb_to_linear(r),
            srgb_to_linear(g),
            srgb_to_linear(b),
            a,
            velocity.x,
            velocity.y,
            velocity.z,
            p.rotation,
            p.seed,
            t,
            0.0,
            0.0,
        ]);
    }
    if e.blend == ParticleBlend::Alpha {
        let distance = |i: &[f32; INSTANCE_FLOATS]| {
            (Vector3::new(i[0], i[1], i[2]) - eye.to_vec()).magnitude2()
        };
        out.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
    }
}

/// Draw `system` over the camera's view.
pub(super) fn draw(
    scene: &Scene,
    system: &ParticleSystem,
    transform: &Matrix4<f32>,
    view_projection: &Matrix4<f32>,
    scratch: &mut Vec<[f32; INSTANCE_FLOATS]>,
) {
    // Only into the camera's view: not into debug captures.
    if !hdr::draws_to_view() {
        return;
    }
    let Some(camera) = scene.get_active_camera() else {
        return;
    };
    let eye = camera.get_camera().get_eye_position();
    let view = camera.get_camera().get_matrix();
    let right = Vector3::new(view.x.x, view.y.x, view.z.x).normalize();
    let up = Vector3::new(view.x.y, view.y.y, view.z.y).normalize();
    let projection = camera.get_projection().get_matrix();
    let lighting = Lighting::from_scene(scene)
        .unwrap_or_else(|| Lighting::preview(Vector3::new(0.3, 0.8, 0.2), eye));
    pack(system, transform, eye, scratch);
    if scratch.is_empty() {
        return;
    }
    let effect = &system.effect;
    let additive = effect.blend == ParticleBlend::Additive;
    let opaque = hdr::opaque_scene();

    RENDERER.with(|renderer| {
        let mut renderer = renderer.borrow_mut();
        let renderer = renderer.get_or_insert_with(ParticleRenderer::new);
        renderer.upload(scratch);
        let shader = &renderer.shader;
        shader.bind();
        lighting.apply(shader);
        shader.set_uniform_mat4("viewProjection", view_projection);
        shader.set_uniform_3fv("camRight", &right);
        shader.set_uniform_3fv("camUp", &up);
        shader.set_uniform_3f("eye", eye.x, eye.y, eye.z);
        shader.set_uniform_1i("look", effect.look.shader_value());
        shader.set_uniform_1f("stretch", effect.stretch);
        shader.set_uniform_1i("lit", effect.lit as i32);
        shader.set_uniform_1i("additive", additive as i32);
        shader.set_uniform_1f("emissive", effect.emissive);
        shader.set_uniform_1i("sceneDepth", DEPTH_UNIT as i32);
        shader.set_uniform_1f("projZ", projection.z.z);
        shader.set_uniform_1f("projW", projection.w.z);
        let soft = effect.soft > 0.0 && opaque.is_some();
        shader.set_uniform_1i("soft", soft as i32);
        shader.set_uniform_1f("softness", effect.soft.max(1e-3));
        unsafe {
            if let Some(opaque) = opaque {
                gl::ActiveTexture(gl::TEXTURE0 + DEPTH_UNIT);
                gl::BindTexture(gl::TEXTURE_2D, opaque.depth);
                gl::ActiveTexture(gl::TEXTURE0);
            }
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::FALSE);
            gl::Enable(gl::BLEND);
            if additive {
                gl::BlendFunc(gl::ONE, gl::ONE);
            } else {
                gl::BlendFunc(gl::ONE, gl::ONE_MINUS_SRC_ALPHA);
            }
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(renderer.vao);
            gl::DrawArraysInstanced(gl::TRIANGLES, 0, 6, scratch.len() as GLsizei);
            gl::BindVertexArray(0);
            gl::Disable(gl::BLEND);
            gl::DepthMask(gl::TRUE);
            if opaque.is_some() {
                gl::ActiveTexture(gl::TEXTURE0 + DEPTH_UNIT);
                gl::BindTexture(gl::TEXTURE_2D, 0);
                gl::ActiveTexture(gl::TEXTURE0);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use cgmath::Point3;

    use super::*;
    use crate::core::renderer::particles::{ParticleEffect, ParticlePreset};

    #[test]
    fn smoke_is_packed_back_to_front_and_in_linear_colour() {
        let mut system = ParticleSystem::new(
            ParticleEffect {
                rate: 0.0,
                burst: 30,
                ..ParticlePreset::Smoke.effect()
            },
            7,
        );
        system.step(0.05, &Matrix4::from_translation(Vector3::new(0.0, 0.0, 5.0)));
        let mut out = Vec::new();
        pack(&system, &Matrix4::from_scale(1.0), Point3::new(0.0, 0.0, 0.0), &mut out);
        assert_eq!(out.len(), 30);
        let d = |i: &[f32; 16]| i[0] * i[0] + i[1] * i[1] + i[2] * i[2];
        assert!(out.windows(2).all(|w| d(&w[0]) >= d(&w[1])));
        // The start colour's grey 0.35 is about 0.1 linear.
        let first = &out[0];
        assert!(first[4] < 0.2);
    }

    #[test]
    fn local_space_particles_move_with_the_entity() {
        let mut system = ParticleSystem::new(
            ParticleEffect {
                rate: 0.0,
                burst: 1,
                speed: [0.0, 0.0],
                world_space: false,
                shape: crate::core::renderer::particles::EmitterShape::Point,
                ..ParticlePreset::Embers.effect()
            },
            3,
        );
        system.step(0.0, &Matrix4::from_scale(1.0));
        let mut out = Vec::new();
        let moved = Matrix4::from_translation(Vector3::new(4.0, 5.0, 6.0));
        pack(&system, &moved, Point3::new(0.0, 0.0, 0.0), &mut out);
        assert_eq!([out[0][0], out[0][1], out[0][2]], [4.0, 5.0, 6.0]);
    }
}
