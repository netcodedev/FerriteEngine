use cgmath::{Point3, Vector3};
use lazy_static::lazy_static;
use std::sync::Mutex;
use std::time::Instant;

use super::CloudSettings;
use crate::core::{
    gfx::{self, Format, SamplerDesc, TextureDesc},
    renderer::shader::Shader,
};

const COMMON_GLSL: &str = include_str!("common.glsl");
const CLOUD_FIELD_GLSL: &str = super::CLOUD_FIELD_GLSL;
/// Texture unit the cloud texture is bound to while the sky is drawn.
const CLOUD_UNIT: u32 = 13;
/// Texture unit the clouds of the frame before are bound to while they are
/// reprojected.
const HISTORY_UNIT: u32 = 12;
/// With reprojection, the texel of each 2x2 block drawn in each of four
/// frames in turn.
const PHASES: [[i32; 2]; 4] = [[0, 0], [1, 1], [1, 0], [0, 1]];

lazy_static! {
    static ref START: Instant = Instant::now();
}

/// Seconds on the sky's clock (`uTime`), which moves the clouds.
pub fn seconds() -> f32 {
    START.elapsed().as_secs_f32()
}

pub struct SkyRenderer {
    shader: Shader,
    cloud_shader: Shader,
    reproject_shader: Shader,
    /// Fullscreen triangle strip (two triangles covering NDC -1..1).
    quad: gfx::Buffer,
    clouds: Option<CloudTarget>,
    reprojection: Option<Reprojection>,
}

/// Where the camera looked, for the cloud reprojection.
#[derive(Clone, Copy)]
struct CameraBasis {
    forward: Vector3<f32>,
    right: Vector3<f32>,
    up: Vector3<f32>,
    tan_half_fov_x: f32,
    tan_half_fov_y: f32,
}

/// The targets of the cloud reprojection (`CloudSettings::reprojection`):
/// the quarter of the texels drawn each frame, and the full cloud texture
/// of this frame and the one before, in turn.
struct Reprojection {
    fresh: CloudTarget,
    history: [CloudTarget; 2],
    /// Which of `history` holds the last frame's clouds.
    current: usize,
    frame: usize,
    /// The camera of the last frame; `None` before the first.
    previous: Option<CameraBasis>,
}

impl Reprojection {
    fn new(width: i32, height: i32) -> Self {
        Self {
            fresh: CloudTarget::new((width + 1) / 2, (height + 1) / 2),
            history: [CloudTarget::new(width, height), CloudTarget::new(width, height)],
            current: 0,
            frame: 0,
            previous: None,
        }
    }
}

/// How the clouds are read: filtered, clamped at the edges.
const CLOUD_SAMPLER: SamplerDesc = SamplerDesc::LINEAR_CLAMP;
/// How the scene's depth is read (texel for texel).
const DEPTH_SAMPLER: SamplerDesc = SamplerDesc::NEAREST_CLAMP;

/// Offscreen target of the cloud pass.
struct CloudTarget {
    texture: gfx::Texture,
    target: gfx::RenderTarget,
    width: i32,
    height: i32,
}

impl CloudTarget {
    fn new(width: i32, height: i32) -> Self {
        let texture = gfx::Texture::new(TextureDesc::d2(Format::Rgba16F, width as u32, height as u32));
        let target = gfx::RenderTarget::new(&[texture.attachment(0, None)], None);
        Self {
            texture,
            target,
            width,
            height,
        }
    }

    /// A pass drawing into it, contents kept.
    fn begin(&self) {
        gfx::begin_pass(
            self.target.reference(),
            (self.width as u32, self.height as u32),
            gfx::Load::Keep,
            gfx::Load::Keep,
        );
    }

    fn bind_to(&self, unit: u32) {
        gfx::bind_texture(unit, &self.texture, &CLOUD_SAMPLER);
    }
}

lazy_static! {
    static ref RENDERER: Mutex<SkyRenderer> = Mutex::new(SkyRenderer::new());
}

/// The state the sky's passes draw with: the fullscreen strip, no
/// blending, depth as given.
fn pipeline(depth: gfx::DepthState) -> gfx::PipelineDesc {
    gfx::PipelineDesc {
        vertex_layouts: crate::core::gfx::Layouts::of(&[gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x2])]),
        topology: gfx::Topology::TriangleStrip,
        depth,
        ..Default::default()
    }
}

impl SkyRenderer {
    fn new() -> Self {
        let shader = Shader::new(
            include_str!("vertex.glsl"),
            &format!("#version 460 core\n{COMMON_GLSL}\n{}", include_str!("fragment.glsl")),
        );
        let reproject_shader = Shader::new(
            include_str!("vertex.glsl"),
            &format!("#version 460 core\n{COMMON_GLSL}\n{}", include_str!("reproject.glsl")),
        );
        let cloud_shader = Shader::new(
            include_str!("vertex.glsl"),
            &format!(
                "#version 460 core\n{COMMON_GLSL}\n{CLOUD_FIELD_GLSL}\n{}",
                include_str!("clouds.glsl")
            ),
        );

        #[rustfmt::skip]
        let verts: [f32; 8] = [
            -1.0, -1.0,
             1.0, -1.0,
            -1.0,  1.0,
             1.0,  1.0,
        ];
        let quad = gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &verts);

        Self {
            shader,
            cloud_shader,
            reproject_shader,
            quad,
            clouds: None,
            reprojection: None,
        }
    }

    /// Render the sky using the camera's world-space basis vectors. Avoids
    /// any matrix inversion in the shader by reconstructing each ray as
    /// `forward + right * x_ndc * tanHalfFovX + up * y_ndc * tanHalfFovY`.
    /// The clouds are drawn in passes of their own first; the sky goes
    /// into the pass running now, at its viewport.
    pub fn render(
        camera_pos: Point3<f32>,
        cam_forward: Vector3<f32>,
        cam_right: Vector3<f32>,
        cam_up: Vector3<f32>,
        tan_half_fov_x: f32,
        tan_half_fov_y: f32,
        sun_direction: Vector3<f32>,
        moon_direction: Vector3<f32>,
        moonlight: f32,
        time_of_day: f32,
        cloud_coverage: f32,
        cloud_type: f32,
        cloud_settings: &CloudSettings,
        precipitation: f32,
        scene_depth: Option<gfx::RawId>,
    ) {
        super::noise::bind();
        let mut r = RENDERER.lock().unwrap();
        // Draw with `shader` and `desc` from now on, with the uniforms all
        // the sky's programs share.
        let set_uniforms = |shader: &Shader, desc: &gfx::PipelineDesc| {
            gfx::set_pipeline(shader.pipeline(desc));
            shader.set_uniform_3f("camForward", cam_forward.x, cam_forward.y, cam_forward.z);
            shader.set_uniform_3f("camRight", cam_right.x, cam_right.y, cam_right.z);
            shader.set_uniform_3f("camUp", cam_up.x, cam_up.y, cam_up.z);
            shader.set_uniform_1f("tanHalfFovX", tan_half_fov_x);
            shader.set_uniform_1f("tanHalfFovY", tan_half_fov_y);
            shader.set_uniform_3f("sunDirection", sun_direction.x, sun_direction.y, sun_direction.z);
            shader.set_uniform_1f("timeOfDay", time_of_day);
            shader.set_uniform_3f("moonDirection", moon_direction.x, moon_direction.y, moon_direction.z);
            shader.set_uniform_1f("moonlight", moonlight);
            shader.set_uniform_1f("cloudCoverage", cloud_coverage);
            shader.set_uniform_1f("cloudType", cloud_type);
            shader.set_uniform_1f("altocumulus", cloud_settings.altocumulus);
            shader.set_uniform_1f("cirrus", cloud_settings.cirrus);
            shader.set_uniform_1f("precipitation", precipitation);
            shader.set_uniform_3f("camPos", camera_pos.x, camera_pos.y, camera_pos.z);
            shader.set_uniform_1f("uTime", START.elapsed().as_secs_f32());
        };
        // The cloud passes draw colour only, no depth.
        let cloud_desc = pipeline(gfx::DepthState::OFF);

        // Where the sky goes, to come back to after the cloud passes.
        let viewport = gfx::viewport();

        // Cloud pass, into the reduced-size target.
        // The clouds are raymarched at a fraction of the width and height
        // (`effects.cloud_downscale`, 2 at High): they are soft, and the
        // raymarch is the most expensive part of the frame.
        let downscale = crate::core::graphics::engine().cloud_downscale.max(1);
        let width = (viewport.width as i32 / downscale).max(1);
        let height = (viewport.height as i32 / downscale).max(1);
        let r = &mut *r;
        set_uniforms(&r.cloud_shader, &cloud_desc);
        r.cloud_shader.set_uniform_1i("useSceneDepth", scene_depth.is_some() as i32);
        r.cloud_shader.set_uniform_2f("cloudSize", width as f32, height as f32);
        r.cloud_shader.set_uniform_1i("reproject", 0);
        if let Some(depth) = scene_depth {
            gfx::bind_raw_texture(CLOUD_UNIT, depth, gfx::TextureKind::D2, &DEPTH_SAMPLER);
            r.cloud_shader.set_uniform_1i("sceneDepth", CLOUD_UNIT as i32);
            r.cloud_shader.set_uniform_2f(
                "sceneDepthScale",
                viewport.width as f32 / width as f32,
                viewport.height as f32 / height as f32,
            );
        }
        let cloud_texture = if cloud_settings.reprojection {
            r.clouds = None;
            if r
                .reprojection
                .as_ref()
                .map_or(true, |p| (p.history[0].width, p.history[0].height) != (width, height))
            {
                r.reprojection = Some(Reprojection::new(width, height));
            }
            let p = r.reprojection.as_mut().expect("just created");
            let target = 1 - p.current;
            let output = &p.history[target];
            match p.previous {
                Some(previous) => {
                    // A quarter of the texels, fresh.
                    let [x, y] = PHASES[p.frame % PHASES.len()];
                    p.fresh.begin();
                    gfx::set_pipeline(r.cloud_shader.pipeline(&cloud_desc));
                    r.cloud_shader.set_uniform_1i("reproject", 1);
                    r.cloud_shader.set_uniform_2i("phase", x, y);
                    gfx::set_vertex_buffer(0, &r.quad, 0, 8);
                    gfx::draw(0, 4);
                    gfx::end_pass();

                    // The rest from the frame before.
                    output.begin();
                    let shader = &r.reproject_shader;
                    set_uniforms(shader, &cloud_desc);
                    shader.set_uniform_2i("phase", x, y);
                    shader.set_uniform_3fv("prevForward", &previous.forward);
                    shader.set_uniform_3fv("prevRight", &previous.right);
                    shader.set_uniform_3fv("prevUp", &previous.up);
                    shader.set_uniform_1f("prevTanHalfFovX", previous.tan_half_fov_x);
                    shader.set_uniform_1f("prevTanHalfFovY", previous.tan_half_fov_y);
                    p.fresh.bind_to(CLOUD_UNIT);
                    shader.set_uniform_1i("freshClouds", CLOUD_UNIT as i32);
                    p.history[p.current].bind_to(HISTORY_UNIT);
                    shader.set_uniform_1i("history", HISTORY_UNIT as i32);
                    gfx::set_vertex_buffer(0, &r.quad, 0, 8);
                    gfx::draw(0, 4);
                    gfx::end_pass();
                    gfx::unbind_texture(HISTORY_UNIT, gfx::TextureKind::D2);
                }
                None => {
                    // Nothing to build on yet: all of them.
                    output.begin();
                    gfx::set_pipeline(r.cloud_shader.pipeline(&cloud_desc));
                    gfx::set_vertex_buffer(0, &r.quad, 0, 8);
                    gfx::draw(0, 4);
                    gfx::end_pass();
                }
            }
            p.previous = Some(CameraBasis {
                forward: cam_forward,
                right: cam_right,
                up: cam_up,
                tan_half_fov_x,
                tan_half_fov_y,
            });
            p.current = target;
            p.frame = p.frame.wrapping_add(1);
            &p.history[target]
        } else {
            r.reprojection = None;
            if r.clouds.as_ref().map_or(true, |c| (c.width, c.height) != (width, height)) {
                r.clouds = Some(CloudTarget::new(width, height));
            }
            let clouds = r.clouds.as_ref().expect("just created");
            clouds.begin();
            gfx::set_pipeline(r.cloud_shader.pipeline(&cloud_desc));
            gfx::set_vertex_buffer(0, &r.quad, 0, 8);
            gfx::draw(0, 4);
            gfx::end_pass();
            clouds
        };

        // The sky itself, with the clouds blended in, back in the pass
        // running before. Sky writes color only — never depth — so geometry
        // rendered after overdraws it via the normal depth test. When the
        // scene's depth is in the target already, the sky is drawn only
        // where nothing covers it (it lies at the far plane).
        gfx::set_viewport(viewport);
        let depth = match scene_depth {
            Some(_) => gfx::DepthState::LEQUAL.write(false),
            None => gfx::DepthState::OFF,
        };
        set_uniforms(&r.shader, &pipeline(depth));
        cloud_texture.bind_to(CLOUD_UNIT);
        r.shader.set_uniform_1i("cloudTexture", CLOUD_UNIT as i32);
        gfx::set_vertex_buffer(0, &r.quad, 0, 8);
        gfx::draw(0, 4);
        gfx::unbind_texture(CLOUD_UNIT, gfx::TextureKind::D2);
    }
}
