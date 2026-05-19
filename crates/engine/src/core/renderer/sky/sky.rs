use cgmath::{Point3, Vector3};
use gl::types::*;
use lazy_static::lazy_static;
use std::sync::Mutex;
use std::time::Instant;

use crate::core::renderer::shader::Shader;

lazy_static! {
    static ref START: Instant = Instant::now();
}

pub struct SkyRenderer {
    shader: Shader,
    vao: GLuint,
    #[allow(dead_code)]
    vbo: GLuint,
}

lazy_static! {
    static ref RENDERER: Mutex<SkyRenderer> = Mutex::new(SkyRenderer::new());
}

impl SkyRenderer {
    fn new() -> Self {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));

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

        Self { shader, vao, vbo }
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
        let r = RENDERER.lock().unwrap();
        unsafe {
            // Sky writes color only — never depth — so geometry rendered after
            // overdraws it via the normal depth test.
            gl::Disable(gl::DEPTH_TEST);
            gl::DepthMask(gl::FALSE);
            gl::Disable(gl::BLEND);

            r.shader.bind();
            r.shader.set_uniform_3f("camForward", cam_forward.x, cam_forward.y, cam_forward.z);
            r.shader.set_uniform_3f("camRight",   cam_right.x,   cam_right.y,   cam_right.z);
            r.shader.set_uniform_3f("camUp",      cam_up.x,      cam_up.y,      cam_up.z);
            r.shader.set_uniform_1f("tanHalfFovX", tan_half_fov_x);
            r.shader.set_uniform_1f("tanHalfFovY", tan_half_fov_y);
            r.shader.set_uniform_3f(
                "sunDirection",
                sun_direction.x,
                sun_direction.y,
                sun_direction.z,
            );
            r.shader.set_uniform_1f("timeOfDay", time_of_day);
            r.shader.set_uniform_1f("uCoverage", cloud_coverage);
            r.shader.set_uniform_3f(
                "camPos",
                camera_pos.x,
                camera_pos.y,
                camera_pos.z,
            );
            let t = START.elapsed().as_secs_f32();
            r.shader.set_uniform_1f("uTime", t);

            gl::BindVertexArray(r.vao);
            gl::DrawArrays(gl::TRIANGLE_STRIP, 0, 4);
            gl::BindVertexArray(0);
            gl::UseProgram(0);

            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
        }
    }
}
