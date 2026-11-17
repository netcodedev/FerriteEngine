//! Pictures of a scatter model's variants from around it, for drawing them
//! far away as flat cards that turn toward the camera (see
//! `terrain::far_scatter`): each variant's most detailed tier, seen level
//! from `FRAMES` directions, its colour and its normals (model space), cut
//! out where it is not.

use std::{cell::OnceCell, rc::Rc};

use cgmath::{Matrix4, Point3, Vector3};
use gl::types::{GLint, GLsizei, GLuint, GLvoid};

use crate::core::renderer::{pbr, shader::Shader};

use super::{ModelVariant, NORMAL_OFFSET, POS_OFFSET, UV_OFFSET, VERTEX_STRIDE};

/// Directions a variant is pictured from, evenly around it: frame `k` from
/// model-space direction `(cos a, 0, sin a)`, `a = k * TAU / FRAMES`.
pub const FRAMES: usize = 8;
/// Pixels of a picture.
const WIDTH: i32 = 128;
const HEIGHT: i32 = 256;

/// One variant's pictures: two arrays of `FRAMES` layers each, the colour
/// (sRGB, alpha the cut-out, premultiplied) and the normal (`n * 0.5 + 0.5`,
/// premultiplied by the cut-out too).
pub struct Impostor {
    pub albedo: GLuint,
    pub normal: GLuint,
    /// How far the pictures reach sideways from the model's upright axis,
    /// and from where to where they reach up (model space, metres).
    pub half_width: f32,
    pub bottom: f32,
    pub top: f32,
}

impl Drop for Impostor {
    fn drop(&mut self) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            gl::DeleteTextures(2, [self.albedo, self.normal].as_ptr());
        }
    }
}

thread_local! {
    static BAKE_SHADER: OnceCell<Rc<Shader>> = const { OnceCell::new() };
}

fn bake_shader() -> Rc<Shader> {
    BAKE_SHADER.with(|cell| {
        cell.get_or_init(|| {
            Rc::new(Shader::new(
                include_str!("impostor_bake_vertex.glsl"),
                include_str!("impostor_bake_fragment.glsl"),
            ))
        })
        .clone()
    })
}

/// Picture `variant`. Must run on the GL thread; leaves the GL state as it
/// found it.
pub(super) fn bake(variant: &ModelVariant) -> Impostor {
    let (min, max) = variant.bounds;
    // However the model is turned, it stays within this of its axis.
    let half_width = (min[0].abs().max(max[0].abs())).hypot(min[2].abs().max(max[2].abs())).max(0.1);
    let (bottom, top) = (min[1], max[1].max(min[1] + 0.1));
    let mut impostor = Impostor {
        albedo: 0,
        normal: 0,
        half_width,
        bottom,
        top,
    };
    if crate::core::headless::is_headless() {
        return impostor;
    }
    let Some(lod) = variant.lods.first() else {
        return impostor;
    };
    let levels = 1 + (HEIGHT.max(WIDTH) as f32).log2() as GLsizei;
    unsafe {
        // What is changed here, to be put back.
        let mut framebuffer: GLint = 0;
        gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut framebuffer);
        let mut read_framebuffer: GLint = 0;
        gl::GetIntegerv(gl::READ_FRAMEBUFFER_BINDING, &mut read_framebuffer);
        let mut viewport = [0 as GLint; 4];
        gl::GetIntegerv(gl::VIEWPORT, viewport.as_mut_ptr());
        let mut clear = [0.0f32; 4];
        gl::GetFloatv(gl::COLOR_CLEAR_VALUE, clear.as_mut_ptr());
        let mut depth_func: GLint = 0;
        gl::GetIntegerv(gl::DEPTH_FUNC, &mut depth_func);
        let mut depth_mask: u8 = 0;
        gl::GetBooleanv(gl::DEPTH_WRITEMASK, &mut depth_mask);
        let enabled = [gl::DEPTH_TEST, gl::BLEND, gl::CULL_FACE, gl::FRAMEBUFFER_SRGB]
            .map(|cap| (cap, gl::IsEnabled(cap) == gl::TRUE));

        let mut textures = [0 as GLuint; 2];
        gl::GenTextures(2, textures.as_mut_ptr());
        for (texture, format) in textures.iter().zip([gl::SRGB8_ALPHA8, gl::RGBA8]) {
            gl::BindTexture(gl::TEXTURE_2D_ARRAY, *texture);
            gl::TexStorage3D(gl::TEXTURE_2D_ARRAY, levels, format, WIDTH, HEIGHT, FRAMES as GLsizei);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_MIN_FILTER, gl::LINEAR_MIPMAP_LINEAR as GLint);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_MAG_FILTER, gl::LINEAR as GLint);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as GLint);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as GLint);
        }
        impostor.albedo = textures[0];
        impostor.normal = textures[1];

        let mut depth: GLuint = 0;
        gl::GenRenderbuffers(1, &mut depth);
        gl::BindRenderbuffer(gl::RENDERBUFFER, depth);
        gl::RenderbufferStorage(gl::RENDERBUFFER, gl::DEPTH_COMPONENT24, WIDTH, HEIGHT);
        gl::BindRenderbuffer(gl::RENDERBUFFER, 0);
        let mut fbo: GLuint = 0;
        gl::GenFramebuffers(1, &mut fbo);
        gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
        gl::FramebufferRenderbuffer(gl::FRAMEBUFFER, gl::DEPTH_ATTACHMENT, gl::RENDERBUFFER, depth);
        gl::DrawBuffers(2, [gl::COLOR_ATTACHMENT0, gl::COLOR_ATTACHMENT1].as_ptr());

        // The tier's submeshes, as the instances draw them (without the
        // instance attributes).
        let mut vaos = vec![0 as GLuint; lod.submeshes.len()];
        gl::GenVertexArrays(vaos.len() as GLsizei, vaos.as_mut_ptr());
        for (vao, sub) in vaos.iter().zip(&lod.submeshes) {
            gl::BindVertexArray(*vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, sub.vbo);
            for (location, size, offset) in [(0, 3, POS_OFFSET), (1, 3, NORMAL_OFFSET), (2, 2, UV_OFFSET)] {
                gl::EnableVertexAttribArray(location);
                gl::VertexAttribPointer(location, size, gl::FLOAT, gl::FALSE, VERTEX_STRIDE, offset as *const GLvoid);
            }
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, sub.ibo);
        }

        gl::Viewport(0, 0, WIDTH, HEIGHT);
        gl::ClearColor(0.0, 0.0, 0.0, 0.0);
        gl::Enable(gl::DEPTH_TEST);
        gl::DepthFunc(gl::LESS);
        gl::DepthMask(gl::TRUE);
        gl::Disable(gl::BLEND);
        gl::Disable(gl::CULL_FACE);
        // The colour is written as linear light into the sRGB pictures.
        gl::Enable(gl::FRAMEBUFFER_SRGB);
        let shader = bake_shader();
        shader.bind();
        let middle = 0.5 * (bottom + top);
        let distance = 2.0 * half_width + 1.0;
        let projection = cgmath::ortho(
            -half_width,
            half_width,
            bottom - middle,
            top - middle,
            0.01,
            2.0 * distance,
        );
        for frame in 0..FRAMES {
            for (attachment, texture) in [(gl::COLOR_ATTACHMENT0, textures[0]), (gl::COLOR_ATTACHMENT1, textures[1])] {
                gl::FramebufferTextureLayer(gl::FRAMEBUFFER, attachment, texture, 0, frame as GLint);
            }
            if frame == 0 && gl::CheckFramebufferStatus(gl::FRAMEBUFFER) != gl::FRAMEBUFFER_COMPLETE {
                log::warn!("terrain scatter: cannot picture a model for far away");
                break;
            }
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
            let angle = frame as f32 * std::f32::consts::TAU / FRAMES as f32;
            let eye = Point3::new(angle.cos() * distance, middle, angle.sin() * distance);
            let view = Matrix4::look_at_rh(eye, Point3::new(0.0, middle, 0.0), Vector3::unit_y());
            shader.set_uniform_mat4("viewProjection", &(projection * view));
            for (vao, sub) in vaos.iter().zip(&lod.submeshes) {
                pbr::bind_material(&shader, variant.model.material_textures(sub.material_index));
                gl::BindVertexArray(*vao);
                gl::DrawElements(gl::TRIANGLES, sub.index_count, gl::UNSIGNED_INT, std::ptr::null());
            }
        }
        gl::BindVertexArray(0);
        gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        gl::DeleteVertexArrays(vaos.len() as GLsizei, vaos.as_ptr());
        gl::DeleteFramebuffers(1, &fbo);
        gl::DeleteRenderbuffers(1, &depth);
        for texture in textures {
            gl::BindTexture(gl::TEXTURE_2D_ARRAY, texture);
            gl::GenerateMipmap(gl::TEXTURE_2D_ARRAY);
        }
        gl::BindTexture(gl::TEXTURE_2D_ARRAY, 0);

        gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, framebuffer as GLuint);
        gl::BindFramebuffer(gl::READ_FRAMEBUFFER, read_framebuffer as GLuint);
        gl::Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        gl::ClearColor(clear[0], clear[1], clear[2], clear[3]);
        gl::DepthFunc(depth_func as GLuint);
        gl::DepthMask(depth_mask);
        for (cap, on) in enabled {
            if on {
                gl::Enable(cap);
            } else {
                gl::Disable(cap);
            }
        }
    }
    impostor
}
