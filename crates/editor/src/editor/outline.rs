//! Selection outline: the selected entity is rendered into an offscreen
//! depth mask, then a screen-space pass draws a band around the mask's edge.

use cgmath::Vector3;
use ferrite::core::{
    entity::EntityHandle,
    gfx,
    renderer::{
        depth_pass::DepthPass,
        framebuffer::ShadowFrameBuffer,
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
    target: Option<(ShadowFrameBuffer, i32, i32)>,
}

impl Outline {
    pub fn new() -> Self {
        Self {
            shader: Shader::new(VERTEX, FRAGMENT),
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
        // Only the depth is read: the renderers draw with their depth-only
        // programs (the same positions and cut-outs, no shading) and no
        // colour is written.
        target.0.begin_pass(gfx::Load::Color([0.0; 4]), gfx::Load::Depth(1.0));
        {
            let _depth_only = DepthPass::begin();
            entity.render(scene, &camera.get_view_projection(), parent);
        }
        gfx::end_pass();

        // 2. Edge pass over the viewport, always on top of the scene.
        let Some(mask) = target.get_depth_texture() else {
            return;
        };
        gfx::set_viewport(gfx::Rect::new(x, y, w as u32, h as u32));
        let desc = gfx::PipelineDesc {
            depth: gfx::DepthState::OFF,
            blend: Some(gfx::BlendState::ALPHA),
            ..Default::default()
        };
        gfx::set_pipeline(self.shader.pipeline(&desc));
        mask.bind_to(0);
        self.shader.set_uniform_1i("mask", 0);
        self.shader.set_uniform_3fv("color", &OUTLINE_COLOR);
        self.shader.set_uniform_1f("thickness", THICKNESS);
        self.shader
            .set_uniform_2f("texel", 1.0 / w as f32, 1.0 / h as f32);
        gfx::draw(0, 3);
        gfx::unbind_texture(0, gfx::TextureKind::D2);
    }
}
