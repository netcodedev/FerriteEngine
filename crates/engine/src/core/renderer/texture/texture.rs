use std::path::Path;

use gl::types::{GLint, GLsizei, GLsizeiptr};

use super::{Shader, Texture, TextureRenderer};

impl Texture {
    pub fn new() -> Self {
        let texture = Texture::gen_texture();
        texture
    }

    fn gen_texture() -> Self {
        if crate::core::headless::is_headless() {
            return Texture { id: 0, target: gl::TEXTURE_2D, owned: true };
        }
        let mut id = 0;
        unsafe {
            gl::GenTextures(1, &mut id);
        }
        Texture {
            id,
            target: gl::TEXTURE_2D,
            owned: true,
        }
    }

    /// Another handle on this texture, for many models to use one
    /// texture. It does not free the texture: this one must outlive it.
    pub fn share(&self) -> Texture {
        Texture {
            id: self.id,
            target: self.target,
            owned: false,
        }
    }

    pub fn set_as_depth_texture(&self, width: u32, height: u32) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.bind();
        unsafe {
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_WRAP_S,
                gl::CLAMP_TO_BORDER as i32,
            );
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_WRAP_T,
                gl::CLAMP_TO_BORDER as i32,
            );
            let color = [1.0, 1.0, 1.0, 1.0];
            gl::TexParameterfv(gl::TEXTURE_2D, gl::TEXTURE_BORDER_COLOR, color.as_ptr());
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::DEPTH_COMPONENT as GLint,
                width as GLsizei,
                height as GLsizei,
                0,
                gl::DEPTH_COMPONENT,
                gl::FLOAT,
                std::ptr::null(),
            );
        }
    }

    /// An array of `layers` depth maps (16 bit), one per shadow cascade.
    /// Outside the maps the depth reads as far away, i.e. lit.
    pub fn set_as_depth_array(&mut self, width: u32, height: u32, layers: u32) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.target = gl::TEXTURE_2D_ARRAY;
        self.bind();
        unsafe {
            // Sampled with depth comparison (`sampler2DArrayShadow`): each
            // lookup compares the 2x2 texels around it and filters the
            // results bilinearly. Lit where the reference is not farther.
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(
                gl::TEXTURE_2D_ARRAY,
                gl::TEXTURE_COMPARE_MODE,
                gl::COMPARE_REF_TO_TEXTURE as i32,
            );
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_COMPARE_FUNC, gl::LEQUAL as i32);
            gl::TexImage3D(
                gl::TEXTURE_2D_ARRAY,
                0,
                gl::DEPTH_COMPONENT16 as GLint,
                width as GLsizei,
                height as GLsizei,
                layers as GLsizei,
                0,
                gl::DEPTH_COMPONENT,
                gl::FLOAT,
                std::ptr::null(),
            );
            gl::BindTexture(gl::TEXTURE_2D_ARRAY, 0);
        }
    }

    pub fn set_as_color_texture(&self, width: u32, height: u32) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.bind();
        unsafe {
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA8 as GLint,
                width as GLsizei,
                height as GLsizei,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                std::ptr::null(),
            );
        }
    }

    /// Like `load_from_file`, but reports a file that cannot be read as an
    /// image instead of panicking. Returns the image's size in pixels.
    pub fn try_load_from_file(&self, path: &Path) -> Result<(u32, u32), image::ImageError> {
        let img = image::open(path)?.flipv().to_rgba8();
        let size = (img.width(), img.height());
        self.load_from_data(size.0, size.1, img.into_raw());
        Ok(size)
    }

    pub fn load_from_file(&self, path: &Path) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.bind();
        let img = image::open(path)
            .expect("Image not found")
            .flipv()
            .to_rgba8();
        unsafe {
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::REPEAT as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::REPEAT as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as GLint,
                img.width() as GLsizei,
                img.height() as GLsizei,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                img.as_ptr() as *const _,
            );
        }
        Texture::unbind();
    }

    pub fn load_from_data(&self, width: u32, height: u32, data: Vec<u8>) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.bind();
        unsafe {
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::REPEAT as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::REPEAT as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as GLint,
                width as GLsizei,
                height as GLsizei,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const _,
            );
        }
        Texture::unbind();
    }

    /// Upload a material map with mipmaps and anisotropic filtering. A colour map is `srgb`, so the
    /// shader samples it in linear space; data maps (normal, roughness, ...)
    /// are stored as they are.
    pub fn load_material_map(&self, width: u32, height: u32, data: Vec<u8>, srgb: bool) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.bind();
        let internal = if srgb { gl::SRGB8_ALPHA8 } else { gl::RGBA8 };
        unsafe {
            gl::TexParameteri(
                gl::TEXTURE_2D,
                gl::TEXTURE_MIN_FILTER,
                gl::LINEAR_MIPMAP_LINEAR as i32,
            );
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::REPEAT as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::REPEAT as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                internal as GLint,
                width as GLsizei,
                height as GLsizei,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const _,
            );
            // Sharp at a glancing angle too, as the terrain's textures
            // (GL_TEXTURE_MAX_ANISOTROPY, see `load_array`).
            const TEXTURE_MAX_ANISOTROPY: gl::types::GLenum = 0x84FE;
            gl::TexParameterf(gl::TEXTURE_2D, TEXTURE_MAX_ANISOTROPY, 8.0);
            gl::GenerateMipmap(gl::TEXTURE_2D);
        }
        Texture::unbind();
    }

    /// Load `layers` (RGBA8, all `width` x `height`) as a 2D array texture
    /// with mipmaps and anisotropic filtering. `srgb` as for
    /// `load_material_map`.
    pub fn load_array(&mut self, width: u32, height: u32, layers: &[Vec<u8>], srgb: bool) {
        if crate::core::headless::is_headless() {
            return;
        }
        self.target = gl::TEXTURE_2D_ARRAY;
        let internal = if srgb { gl::SRGB8_ALPHA8 } else { gl::RGBA8 };
        self.bind();
        unsafe {
            gl::TexImage3D(
                gl::TEXTURE_2D_ARRAY,
                0,
                internal as GLint,
                width as GLsizei,
                height as GLsizei,
                layers.len() as GLsizei,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                std::ptr::null(),
            );
            for (index, layer) in layers.iter().enumerate() {
                gl::TexSubImage3D(
                    gl::TEXTURE_2D_ARRAY,
                    0,
                    0,
                    0,
                    index as GLint,
                    width as GLsizei,
                    height as GLsizei,
                    1,
                    gl::RGBA,
                    gl::UNSIGNED_BYTE,
                    layer.as_ptr() as *const _,
                );
            }
            gl::TexParameteri(
                gl::TEXTURE_2D_ARRAY,
                gl::TEXTURE_MIN_FILTER,
                gl::LINEAR_MIPMAP_LINEAR as i32,
            );
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_WRAP_S, gl::REPEAT as i32);
            gl::TexParameteri(gl::TEXTURE_2D_ARRAY, gl::TEXTURE_WRAP_T, gl::REPEAT as i32);
            // GL_TEXTURE_MAX_ANISOTROPY (core in 4.6, EXT before); the
            // generated bindings predate it.
            const TEXTURE_MAX_ANISOTROPY: gl::types::GLenum = 0x84FE;
            gl::TexParameterf(gl::TEXTURE_2D_ARRAY, TEXTURE_MAX_ANISOTROPY, 8.0);
            gl::GenerateMipmap(gl::TEXTURE_2D_ARRAY);
            gl::BindTexture(gl::TEXTURE_2D_ARRAY, 0);
        }
    }

    pub fn bind(&self) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            gl::BindTexture(self.target, self.id);
        }
    }

    /// Unbind whatever is bound to this texture's target on the active unit.
    pub fn unbind_target(&self) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            gl::BindTexture(self.target, 0);
        }
    }

    pub fn unbind() {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        if crate::core::headless::is_headless() || !self.owned {
            return;
        }
        unsafe {
            gl::DeleteTextures(1, &self.id);
        }
    }
}

impl TextureRenderer {
    pub fn new() -> Self {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        let mut vao = 0;
        let mut vbo = 0;
        if !crate::core::headless::is_headless() {
            // The unit square every quad is drawn from (see `vertex.glsl`).
            let corners: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0];
            unsafe {
                gl::GenVertexArrays(1, &mut vao);
                gl::BindVertexArray(vao);
                gl::GenBuffers(1, &mut vbo);
                gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    std::mem::size_of_val(&corners) as GLsizeiptr,
                    corners.as_ptr() as *const _,
                    gl::STATIC_DRAW,
                );
                gl::VertexAttribPointer(
                    0,
                    2,
                    gl::FLOAT,
                    gl::FALSE,
                    2 * std::mem::size_of::<f32>() as GLsizei,
                    std::ptr::null(),
                );
                gl::EnableVertexAttribArray(0);
                gl::BindVertexArray(0);
                gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            }
        }
        Self { shader, vao, vbo }
    }

    pub fn render_depth(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.render_inner(texture, x, y, w, h, true);
    }

    pub fn render_color(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.render_inner(texture, x, y, w, h, false);
    }

    /// Kept for compatibility — defaults to colour mode.
    pub fn render(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.render_inner(texture, x, y, w, h, false);
    }

    fn render_inner(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32, is_depth: bool) {
        if self.vao == 0 {
            return;
        }
        unsafe {
            gl::BindVertexArray(self.vao);
            gl::ActiveTexture(gl::TEXTURE0);
            texture.bind();
            self.shader.bind();
            // Standard UV layout: (0,0) = bottom-left, (1,1) = top-right.
            self.shader.set_uniform_4f("rect", x, y, w, h);
            self.shader.set_uniform_1i("isDepth", if is_depth { 1 } else { 0 });
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::Enable(gl::BLEND);
            gl::Disable(gl::DEPTH_TEST);
            gl::DrawArrays(gl::TRIANGLES, 0, 6);
            gl::Disable(gl::BLEND);
            // As the quad's own (deleted) objects used to leave it.
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        }
    }
}

impl Drop for TextureRenderer {
    fn drop(&mut self) {
        if self.vao == 0 || crate::core::headless::is_headless() {
            return;
        }
        // Not once the context is gone (a renderer kept until exit).
        if unsafe { glfw::ffi::glfwGetCurrentContext() }.is_null() {
            return;
        }
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteVertexArrays(1, &self.vao);
        }
    }
}
