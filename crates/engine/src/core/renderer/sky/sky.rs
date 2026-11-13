use cgmath::{Point3, Vector3};
use gl::types::*;
use lazy_static::lazy_static;
use std::sync::Mutex;
use std::time::Instant;

use crate::core::renderer::shader::Shader;

const COMMON_GLSL: &str = include_str!("common.glsl");
/// Texture unit the cloud texture is bound to while the sky is drawn.
const CLOUD_UNIT: u32 = 13;
/// The clouds are raymarched at 1 / CLOUD_DOWNSCALE of the width and
/// height: they are soft, and the raymarch is the most expensive part of
/// the frame.
const CLOUD_DOWNSCALE: i32 = 2;

lazy_static! {
    static ref START: Instant = Instant::now();
}

pub struct SkyRenderer {
    shader: Shader,
    cloud_shader: Shader,
    vao: GLuint,
    #[allow(dead_code)]
    vbo: GLuint,
    clouds: Option<CloudTarget>,
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
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
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
            gl::DeleteFramebuffers(1, &self.framebuffer);
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
        let cloud_shader = Shader::new(
            include_str!("vertex.glsl"),
            &format!("#version 460 core\n{COMMON_GLSL}\n{}", include_str!("clouds.glsl")),
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
            vao,
            vbo,
            clouds: None,
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
        time_of_day: f32,
        cloud_coverage: f32,
    ) {
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
            shader.set_uniform_1f("uCoverage", cloud_coverage);
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
            let mut framebuffer = 0;
            gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut framebuffer);

            // Cloud pass, into the reduced-size target.
            let width = (viewport[2] / CLOUD_DOWNSCALE).max(1);
            let height = (viewport[3] / CLOUD_DOWNSCALE).max(1);
            if r.clouds.as_ref().map_or(true, |c| (c.width, c.height) != (width, height)) {
                r.clouds = Some(CloudTarget::new(width, height));
            }
            let clouds = r.clouds.as_ref().expect("just created");
            gl::BindFramebuffer(gl::FRAMEBUFFER, clouds.framebuffer);
            gl::Viewport(0, 0, width, height);
            set_uniforms(&r.cloud_shader);
            gl::BindVertexArray(r.vao);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);

            // The sky itself, with the clouds blended in.
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer as GLuint);
            gl::Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            set_uniforms(&r.shader);
            gl::ActiveTexture(gl::TEXTURE0 + CLOUD_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, clouds.texture);
            r.shader.set_uniform_1i("cloudTexture", CLOUD_UNIT as i32);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0);

            gl::BindVertexArray(0);
            gl::UseProgram(0);

            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
        }
    }
}
