use std::path::Path;

use gl::types::{GLint, GLsizei, GLsizeiptr, GLvoid};

use super::{Shader, Texture, TextureRenderer};

impl Texture {
    pub fn new() -> Self {
        let texture = Texture::gen_texture();
        texture
    }

    fn gen_texture() -> Self {
        let mut id = 0;
        unsafe {
            gl::GenTextures(1, &mut id);
        }
        Texture { id }
    }

    pub fn set_as_depth_texture(&self, width: u32, height: u32) {
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

    pub fn load_from_file(&self, path: &Path) {
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

    pub fn bind(&self) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, self.id);
        }
    }

    pub fn unbind() {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteTextures(1, &self.id);
        }
    }
}

#[allow(dead_code)]
impl TextureRenderer {
    pub fn new() -> Self {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        Self { shader }
    }

    pub fn render(&self, texture: &Texture) {
        #[rustfmt::skip]
        let vertices: Vec<f32> = vec![
            0.5, 0.5, 0.0, 0.0,
            1.0, 0.5, 1.0, 0.0,
            1.0, 1.0, 1.0, 1.0,
            0.5, 1.0, 0.0, 1.0,
        ];
        let indices = vec![0, 1, 2, 2, 3, 0];

        let mut vba = 0;
        let mut vbo = 0;
        let mut ebo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vba);
            gl::BindVertexArray(vba);
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::GenBuffers(1, &mut ebo);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                indices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                4 * std::mem::size_of::<f32>() as GLsizei as GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::VertexAttribPointer(
                1,
                2,
                gl::FLOAT,
                gl::FALSE,
                4 * std::mem::size_of::<f32>() as GLsizei,
                (indices.len() * std::mem::size_of::<f32>()) as *const GLvoid,
            );
            gl::EnableVertexAttribArray(1);
            gl::ActiveTexture(gl::TEXTURE0);
            texture.bind();
            self.shader.bind();
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::Enable(gl::BLEND);
            gl::Disable(gl::DEPTH_TEST);
            gl::DrawElements(gl::TRIANGLES, 6, gl::UNSIGNED_INT, std::ptr::null());
            gl::Disable(gl::BLEND);
            gl::DeleteBuffers(1, &vbo);
            gl::DeleteBuffers(1, &ebo);
            gl::DeleteVertexArrays(1, &vba);
        }
    }
}
