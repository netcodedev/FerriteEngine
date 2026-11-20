//! Drawing particles: every emitter's particles packed into one stream of
//! instances and drawn as quads (`particles.glsl`), in the transparent
//! pass over the opaque scene.

use std::cell::RefCell;

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Vector3, Vector4};

use super::{ParticleBlend, ParticleSystem};
use crate::core::{
    gfx,
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
    /// The instances, rewritten for every emitter.
    instances: gfx::Buffer,
}

thread_local! {
    static RENDERER: RefCell<Option<ParticleRenderer>> = const { RefCell::new(None) };
}

/// Free the particle renderer's GPU objects (they go with the next frame)
/// while the render layer is still there to free them, not at process
/// exit with the thread-local.
pub fn release_gl_resources() {
    let _ = RENDERER.try_with(|renderer| renderer.borrow_mut().take());
}

/// An instance: four vec4s at locations 0 to 3, one per quad.
fn instance_layout() -> gfx::VertexLayout {
    gfx::VertexLayout::packed(0, gfx::StepMode::Instance, &[gfx::VertexFormat::F32x4; 4])
}

impl ParticleRenderer {
    fn new() -> Self {
        let shader = Shader::new(
            &format!("#version 460 core\n#define VERTEX\n{SOURCE}"),
            &format!("#version 460 core\n#define FRAGMENT\n{LIGHTING_GLSL}\n{SOURCE}"),
        );
        Self {
            shader,
            instances: gfx::Buffer::new(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Dynamic, 0),
        }
    }

    fn upload(&mut self, instances: &[[f32; INSTANCE_FLOATS]]) {
        // A fresh store every time: the emitter drawn before may still be
        // reading the old one, which writing over it would have to wait for.
        self.instances.replace(gfx::as_bytes(instances));
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
        let desc = gfx::PipelineDesc {
            vertex_layouts: crate::core::gfx::Layouts::of(&[instance_layout()]),
            depth: gfx::DepthState::READ,
            blend: Some(if additive { gfx::BlendState::ADDITIVE } else { gfx::BlendState::PREMULTIPLIED }),
            ..Default::default()
        };
        gfx::set_pipeline(shader.pipeline(&desc));
        gfx::set_vertex_buffer(0, &renderer.instances, 0, instance_layout().stride);
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
        if let Some(opaque) = &opaque {
            // Sampled as the HDR target made it: texel by texel.
            let sampler = gfx::SamplerDesc::new(gfx::Filter::Nearest, gfx::Wrap::Repeat);
            gfx::bind_raw_texture(DEPTH_UNIT, opaque.depth as gfx::RawId, gfx::TextureKind::D2, &sampler);
        }
        gfx::draw_instanced(0, 6, 0, scratch.len() as u32);
        if opaque.is_some() {
            gfx::unbind_texture(DEPTH_UNIT, gfx::TextureKind::D2);
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
