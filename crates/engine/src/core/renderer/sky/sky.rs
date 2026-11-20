use cgmath::{Point3, Vector3};
use gl::types::*;
use lazy_static::lazy_static;
use std::sync::Mutex;
use std::time::Instant;

use super::CloudSettings;
use crate::core::renderer::{framebuffer, shader::Shader};

const COMMON_GLSL: &str = include_str!("common.glsl");
const CLOUD_FIELD_GLSL: &str = super::CLOUD_FIELD_GLSL;
/// Texture unit the cloud texture is bound to while the sky is drawn.
const CLOUD_UNIT: u32 = 13;
/// The clouds are raymarched at 1 / CLOUD_DOWNSCALE of the width and
/// height: they are soft, and the raymarch is the most expensive part of
/// the frame.
const CLOUD_DOWNSCALE: i32 = 2;
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
    vao: GLuint,
    #[allow(dead_code)]
    vbo: GLuint,
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

/// Offscreen target of the cloud pass.
struct CloudTarget {
    framebuffer: GLuint,
    texture: GLuint,
    width: i32,
    height: i32,
}

impl CloudTarget {
    fn new(width: i32, height: i32) -> Self {
        let (mut framebuffer, mut texture) = (0, 0);
        unsafe {
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA16F as GLint,
                width,
                height,
                0,
                gl::RGBA,
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
        }
        Self {
            framebuffer,
            texture,
            width,
            height,
        }
    }
}

impl Drop for CloudTarget {
    fn drop(&mut self) {
        unsafe {
            framebuffer::delete(&[self.framebuffer]);
            gl::DeleteTextures(1, &self.texture);
        }
    }
}

lazy_static! {
    static ref RENDERER: Mutex<SkyRenderer> = Mutex::new(SkyRenderer::new());
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

        // Fullscreen triangle strip (two triangles covering NDC -1..1).
        let verts: [f32; 8] = [
            -1.0, -1.0,
             1.0, -1.0,
            -1.0,  1.0,
             1.0,  1.0,
        ];

        let mut vao = 0;
        let mut vbo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (verts.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                verts.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                2 * std::mem::size_of::<GLfloat>() as GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }

        Self {
            shader,
            cloud_shader,
            reproject_shader,
            vao,
            vbo,
            clouds: None,
            reprojection: None,
        }
    }

    /// Render the sky using the camera's world-space basis vectors. Avoids
    /// any matrix inversion in the shader by reconstructing each ray as
    /// `forward + right * x_ndc * tanHalfFovX + up * y_ndc * tanHalfFovY`.
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
        scene_depth: Option<GLuint>,
    ) {
        super::noise::bind();
        let mut r = RENDERER.lock().unwrap();
        let set_uniforms = |shader: &Shader| {
            shader.bind();
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
        unsafe {
            // Sky writes color only — never depth — so geometry rendered after
            // overdraws it via the normal depth test.
            gl::Disable(gl::DEPTH_TEST);
            gl::DepthMask(gl::FALSE);
            gl::Disable(gl::BLEND);

            // Where the sky goes, to come back to after the cloud pass.
            let mut viewport = [0; 4];
            gl::GetIntegerv(gl::VIEWPORT, viewport.as_mut_ptr());
            let framebuffer = framebuffer::bound_draw();

            // Cloud pass, into the reduced-size target.
            let width = (viewport[2] / CLOUD_DOWNSCALE).max(1);
            let height = (viewport[3] / CLOUD_DOWNSCALE).max(1);
            let r = &mut *r;
            set_uniforms(&r.cloud_shader);
            r.cloud_shader.set_uniform_1i("useSceneDepth", scene_depth.is_some() as i32);
            r.cloud_shader.set_uniform_2f("cloudSize", width as f32, height as f32);
            r.cloud_shader.set_uniform_1i("reproject", 0);
            if let Some(depth) = scene_depth {
                gl::ActiveTexture(gl::TEXTURE0 + CLOUD_UNIT);
                gl::BindTexture(gl::TEXTURE_2D, depth);
                r.cloud_shader.set_uniform_1i("sceneDepth", CLOUD_UNIT as i32);
                r.cloud_shader.set_uniform_2f(
                    "sceneDepthScale",
                    viewport[2] as f32 / width as f32,
                    viewport[3] as f32 / height as f32,
                );
            }
            gl::BindVertexArray(r.vao);
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
                        framebuffer::bind(gl::FRAMEBUFFER, p.fresh.framebuffer);
                        gl::Viewport(0, 0, p.fresh.width, p.fresh.height);
                        r.cloud_shader.bind();
                        r.cloud_shader.set_uniform_1i("reproject", 1);
                        r.cloud_shader.set_uniform_2i("phase", x, y);
                        gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

                        // The rest from the frame before.
                        framebuffer::bind(gl::FRAMEBUFFER, output.framebuffer);
                        gl::Viewport(0, 0, width, height);
                        let shader = &r.reproject_shader;
                        set_uniforms(shader);
                        shader.set_uniform_2i("phase", x, y);
                        shader.set_uniform_3fv("prevForward", &previous.forward);
                        shader.set_uniform_3fv("prevRight", &previous.right);
                        shader.set_uniform_3fv("prevUp", &previous.up);
                        shader.set_uniform_1f("prevTanHalfFovX", previous.tan_half_fov_x);
                        shader.set_uniform_1f("prevTanHalfFovY", previous.tan_half_fov_y);
                        gl::ActiveTexture(gl::TEXTURE0 + CLOUD_UNIT);
                        gl::BindTexture(gl::TEXTURE_2D, p.fresh.texture);
                        shader.set_uniform_1i("freshClouds", CLOUD_UNIT as i32);
                        gl::ActiveTexture(gl::TEXTURE0 + HISTORY_UNIT);
                        gl::BindTexture(gl::TEXTURE_2D, p.history[p.current].texture);
                        shader.set_uniform_1i("history", HISTORY_UNIT as i32);
                        gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
                        gl::BindTexture(gl::TEXTURE_2D, 0);
                    }
                    None => {
                        // Nothing to build on yet: all of them.
                        framebuffer::bind(gl::FRAMEBUFFER, output.framebuffer);
                        gl::Viewport(0, 0, width, height);
                        gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
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
                output.texture
            } else {
                r.reprojection = None;
                if r.clouds.as_ref().map_or(true, |c| (c.width, c.height) != (width, height)) {
                    r.clouds = Some(CloudTarget::new(width, height));
                }
                let clouds = r.clouds.as_ref().expect("just created");
                framebuffer::bind(gl::FRAMEBUFFER, clouds.framebuffer);
                gl::Viewport(0, 0, width, height);
                r.cloud_shader.bind();
                gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
                clouds.texture
            };

            // The sky itself, with the clouds blended in.
            framebuffer::bind(gl::FRAMEBUFFER, framebuffer);
            gl::Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            set_uniforms(&r.shader);
            if scene_depth.is_some() {
                // The scene's depth is in the target already: the sky is
                // drawn only where nothing covers it (it lies at the far
                // plane).
                gl::Enable(gl::DEPTH_TEST);
                gl::DepthFunc(gl::LEQUAL);
            }
            gl::ActiveTexture(gl::TEXTURE0 + CLOUD_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, cloud_texture);
            r.shader.set_uniform_1i("cloudTexture", CLOUD_UNIT as i32);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0);

            gl::BindVertexArray(0);
            gl::UseProgram(0);

            gl::DepthFunc(gl::LESS);
            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
        }
    }
}
