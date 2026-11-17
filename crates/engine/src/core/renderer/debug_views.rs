//! Views into what the renderer makes, switched on by the editor (its
//! Performance panel): the ambient occlusion over the whole view, the sun's
//! shadow map cascades along the bottom of it. Culling has its own (see
//! `ViewFrustum::set_frozen`).

use std::{
    cell::OnceCell,
    sync::atomic::{AtomicBool, Ordering},
};

use gl::types::{GLint, GLuint};

use super::shader::Shader;

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
    static OVERLAY: OnceCell<(Shader, GLuint)> = const { OnceCell::new() };
}

fn with_overlay(draw: impl FnOnce(&Shader)) {
    OVERLAY.with(|cell| {
        let (shader, vao) = cell.get_or_init(|| {
            let mut vao = 0;
            unsafe { gl::GenVertexArrays(1, &mut vao) };
            (Shader::new(VERTEX, FRAGMENT), vao)
        });
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
            gl::Disable(gl::BLEND);
            gl::Disable(gl::CULL_FACE);
            gl::BindVertexArray(*vao);
        }
        shader.bind();
        draw(shader);
        unsafe {
            gl::BindVertexArray(0);
            gl::ActiveTexture(gl::TEXTURE0);
        }
    });
}

/// Draw `texture`'s red channel (the ambient occlusion: 1 open, 0 shut
/// in) as grey over the whole viewport set.
pub fn draw_ambient_occlusion(texture: GLuint) {
    with_overlay(|shader| unsafe {
        shader.set_uniform_1i("mode", 0);
        shader.set_uniform_1i("image", 0);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D, texture);
        gl::DrawArrays(gl::TRIANGLES, 0, 3);
        gl::BindTexture(gl::TEXTURE_2D, 0);
    });
}

/// Draw the `cascades` layers of the depth array `texture` (the sun's
/// shadow map, sampled with comparison elsewhere) side by side along the
/// bottom of the viewport set, nearest first.
pub fn draw_shadow_cascades(texture: GLuint, cascades: usize) {
    if cascades == 0 {
        return;
    }
    let mut viewport = [0 as GLint; 4];
    unsafe { gl::GetIntegerv(gl::VIEWPORT, viewport.as_mut_ptr()) };
    let [x, y, w, h] = viewport;
    let gap = 8;
    let side = ((w - gap * (cascades as GLint + 1)) / cascades as GLint).min(h / 3).max(16);
    with_overlay(|shader| unsafe {
        shader.set_uniform_1i("mode", 1);
        shader.set_uniform_1i("layers", 0);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D_ARRAY, texture);
        // The depths themselves, not compared.
        gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_COMPARE_MODE, gl::NONE as GLint);
        for cascade in 0..cascades {
            gl::Viewport(x + gap + cascade as GLint * (side + gap), y + gap, side, side);
            shader.set_uniform_1f("layer", cascade as f32);
            gl::DrawArrays(gl::TRIANGLES, 0, 3);
        }
        gl::TexParameteri(
            gl::TEXTURE_2D_ARRAY,
            gl::TEXTURE_COMPARE_MODE,
            gl::COMPARE_REF_TO_TEXTURE as GLint,
        );
        gl::BindTexture(gl::TEXTURE_2D_ARRAY, 0);
        gl::Viewport(x, y, w, h);
    });
}

