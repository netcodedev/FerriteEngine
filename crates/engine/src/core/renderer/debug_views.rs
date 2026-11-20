//! Views into what the renderer makes, switched on by the editor (its
//! Performance panel): the ambient occlusion over the whole view, the sun's
//! shadow map cascades along the bottom of it. Culling has its own (see
//! `ViewFrustum::set_frozen`).

use std::{
    cell::OnceCell,
    sync::atomic::{AtomicBool, Ordering},
};

use super::shader::Shader;
use crate::core::gfx;
static AMBIENT_OCCLUSION: AtomicBool = AtomicBool::new(false);
static SUN_SHADOWS: AtomicBool = AtomicBool::new(false);

/// Show the ambient occlusion instead of the scene.
pub fn set_ambient_occlusion(on: bool) {
    AMBIENT_OCCLUSION.store(on, Ordering::Relaxed);
}

pub fn ambient_occlusion() -> bool {
    AMBIENT_OCCLUSION.load(Ordering::Relaxed)
}

/// Show the sun's shadow map cascades over the scene.
pub fn set_sun_shadows(on: bool) {
    SUN_SHADOWS.store(on, Ordering::Relaxed);
}

pub fn sun_shadows() -> bool {
    SUN_SHADOWS.load(Ordering::Relaxed)
}

const VERTEX: &str = "#version 460 core
// A rectangle over the viewport set, from three vertices.
out vec2 uv;
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

const FRAGMENT: &str = "#version 460 core
in vec2 uv;
out vec4 color;
// 0: `image`'s red channel as grey; 1: layer `layer` of the depth array
// `layers`, near dark, far light.
uniform int mode;
uniform sampler2D image;
uniform sampler2DArray layers;
uniform float layer;
void main() {
    if (mode == 0) {
        color = vec4(vec3(texture(image, uv).r), 1.0);
    } else {
        float depth = texture(layers, vec3(uv, layer)).r;
        // An edge, so the cascades stand apart.
        float edge = step(min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y)), 0.006);
        color = vec4(mix(vec3(depth), vec3(1.0, 0.8, 0.2), edge), 1.0);
    }
}
";

thread_local! {
    static OVERLAY: OnceCell<Shader> = const { OnceCell::new() };
}

fn with_overlay(draw: impl FnOnce(&Shader)) {
    OVERLAY.with(|cell| {
        let shader = cell.get_or_init(|| Shader::new(VERTEX, FRAGMENT));
        let desc = gfx::PipelineDesc { depth: gfx::DepthState::OFF, ..Default::default() };
        gfx::set_pipeline(shader.pipeline(&desc));
        // Samplers of different types on one unit make every draw fail
        // (GL 4.6 §7.10), even the one not sampled: each its own unit.
        shader.set_uniform_1i("image", 0);
        shader.set_uniform_1i("layers", 1);
        draw(shader);
    });
}

/// Draw `texture`'s red channel (the ambient occlusion: 1 open, 0 shut
/// in) as grey over the whole viewport set.
pub fn draw_ambient_occlusion(texture: gfx::RawId) {
    with_overlay(|shader| {
        shader.set_uniform_1i("mode", 0);
        gfx::bind_raw_texture(0, texture, gfx::TextureKind::D2, &gfx::SamplerDesc::LINEAR_CLAMP);
        gfx::draw(0, 3);
        gfx::unbind_texture(0, gfx::TextureKind::D2);
    });
}

/// Draw the `cascades` layers of the depth array `texture` (the sun's
/// shadow map, sampled with comparison elsewhere) side by side along the
/// bottom of the viewport set, nearest first.
pub fn draw_shadow_cascades(texture: &super::texture::Texture, cascades: usize) {
    if cascades == 0 {
        return;
    }
    let viewport = gfx::viewport();
    let (x, y, w, h) = (viewport.x, viewport.y, viewport.width as i32, viewport.height as i32);
    let gap = 8;
    let side = ((w - gap * (cascades as i32 + 1)) / cascades as i32).min(h / 3).max(16);
    with_overlay(|shader| {
        shader.set_uniform_1i("mode", 1);
        // The depths themselves, not compared.
        let mut sampler = texture.sampler();
        sampler.compare = None;
        texture.bind_to_with(1, &sampler);
        for cascade in 0..cascades {
            gfx::set_viewport(gfx::Rect::new(x + gap + cascade as i32 * (side + gap), y + gap, side as u32, side as u32));
            shader.set_uniform_1f("layer", cascade as f32);
            gfx::draw(0, 3);
        }
        gfx::unbind_texture(1, gfx::TextureKind::D2Array);
        gfx::set_viewport(viewport);
    });
}
