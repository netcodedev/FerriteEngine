//! Selection outline: the selected entity is rendered into an offscreen
//! depth mask, then a screen-space pass draws a band around the mask's edge.

use cgmath::Vector3;
use ferrite::core::{
    entity::EntityHandle,
    renderer::{
        framebuffer::{FrameBuffer, ShadowFrameBuffer},
        shader::Shader,
    },
    scene::Scene,
};

pub const OUTLINE_COLOR: Vector3<f32> = Vector3::new(1.0, 0.6, 0.1);
/// Outline thickness in pixels.
const THICKNESS: f32 = 3.0;

const VERTEX: &str = r#"#version 330 core
out vec2 uv;
void main() {
    // Fullscreen triangle from gl_VertexID; no vertex buffer needed.
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"#version 330 core
uniform sampler2D mask;
uniform vec2 texel;
uniform vec3 color;
uniform float thickness;
in vec2 uv;
out vec4 FragColor;

bool covered(vec2 p) {
    return texture(mask, p).r < 1.0;
}

void main() {
    if (covered(uv)) {
        discard;
    }
    for (int i = 0; i < 16; i++) {
        float a = 6.2831853 * float(i) / 16.0;
        vec2 d = vec2(cos(a), sin(a)) * texel;
        if (covered(uv + d * thickness) || covered(uv + d * thickness * 0.5)) {
            FragColor = vec4(color, 1.0);
            return;
        }
    }
    discard;
}
"#;

pub struct Outline {
    shader: Shader,
    vao: u32,
    target: Option<(ShadowFrameBuffer, i32, i32)>,
}

impl Outline {
    pub fn new() -> Self {
        let mut vao = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
        }
        Self {
            shader: Shader::new(VERTEX, FRAGMENT),
            vao,
            target: None,
        }
    }

    /// Draw the outline of `selected` into the viewport `[x, y, w, h]` (GL
    /// coordinates). Entities without geometry produce an empty mask and no
    /// outline.
    pub fn render(&mut self, scene: &Scene, selected: EntityHandle, viewport: [i32; 4]) {
        let [x, y, w, h] = viewport;
        if w <= 0 || h <= 0 {
            return;
        }
        let Some(entity) = scene.get_entity(&selected) else {
            return;
        };
        let Some(camera) = scene.get_active_camera() else {
            return;
        };
        let Some(parent) = scene.get_parent_world_transform(&selected) else {
            return;
        };
        if self.target.as_ref().map_or(true, |(_, tw, th)| (*tw, *th) != (w, h)) {
            self.target = Some((ShadowFrameBuffer::new(w as u32, h as u32), w, h));
        }
        let (target, _, _) = self.target.as_ref().expect("target just created");

        // 1. Depth mask of just the selected entity (and its children).
        // `bind` also binds the mask texture to the active unit; use unit 0
        // and release it so it is never sampled while being written (unit
        // 15 holds the shadow map).
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
        }
        target.bind();
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::Viewport(0, 0, w, h);
            gl::Enable(gl::DEPTH_TEST);
            gl::DepthMask(gl::TRUE);
            gl::ClearDepth(1.0);
            gl::Clear(gl::DEPTH_BUFFER_BIT | gl::COLOR_BUFFER_BIT);
        }
        entity.render(scene, &camera.get_view_projection(), parent);
        FrameBuffer::unbind();

        // 2. Edge pass over the viewport, always on top of the scene.
        let Some(mask) = target.get_depth_texture() else {
            return;
        };
        unsafe {
            gl::Viewport(x, y, w, h);
            gl::Disable(gl::DEPTH_TEST);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::ActiveTexture(gl::TEXTURE0);
        }
        mask.bind();
        self.shader.bind();
        self.shader.set_uniform_1i("mask", 0);
        self.shader.set_uniform_3fv("color", &OUTLINE_COLOR);
        self.shader.set_uniform_1f("thickness", THICKNESS);
        self.shader
            .set_uniform_2f("texel", 1.0 / w as f32, 1.0 / h as f32);
        unsafe {
            gl::BindVertexArray(self.vao);
            gl::DrawArrays(gl::TRIANGLES, 0, 3);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
            gl::Enable(gl::DEPTH_TEST);
        }
    }
}
