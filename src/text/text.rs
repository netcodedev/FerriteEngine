use rusttype::gpu_cache::Cache;
use rusttype::{point, Font, Rect, PositionedGlyph, Scale};
use crate::shader::Shader;
use gl::types::GLvoid;

use super::{TextRenderer, Texture};

use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref RENDERER: Mutex<TextRenderer> = Mutex::new(TextRenderer::new(1280, 720));
}

impl TextRenderer {
    fn new(width: u32, height: u32) -> TextRenderer {
        let font_data = include_bytes!("../../assets/font/RobotoMono.ttf");
        let font = Font::try_from_bytes(font_data as &[u8]).unwrap();

        let cache: Cache<'static> = Cache::builder().dimensions(1024, 1024).build();

        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        TextRenderer {
            font,
            cache,
            shader,
            texture_buffer: Texture::new(1024, 1024),
            width,
            height,
        }
    }

    /// Renders text to the screen
    /// 
    /// Returns the width and height of the text
    pub fn render(x: i32, y: i32, size: f32, text: &str) -> (i32, i32) {
        let mut renderer = RENDERER.lock().unwrap();
        let glyphs = renderer.layout(Scale::uniform(size), renderer.width, text);
        for glyph in &glyphs {
            renderer.cache.queue_glyph(0, glyph.clone());
        }
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
            renderer.texture_buffer.bind();
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
        }
        let _ = renderer.cache.cache_queued(|rect, data| unsafe {
            gl::TexSubImage2D(
                gl::TEXTURE_2D,
                0,
                rect.min.x as i32,
                rect.min.y as i32,
                rect.width() as i32,
                rect.height() as i32,
                gl::RED, gl::UNSIGNED_BYTE, data.as_ptr() as *const std::ffi::c_void
            );
        });
        
        let mut max_x = 0;
        let mut max_y = 0;
        let vertices: Vec<f32> = glyphs.iter().filter_map(|g| renderer.cache.rect_for(0, g).ok().flatten()).flat_map(|(uv_rect, screen_rect)| {
            if screen_rect.max.x as i32 > max_x {
                max_x = screen_rect.max.x as i32;
            }
            if screen_rect.max.y as i32 > max_y {
                max_y = screen_rect.max.y as i32;
            }
            let gl_rect = Rect {
                min: point(screen_rect.min.x as f32 + x as f32, screen_rect.min.y as f32 + y as f32),
                max: point(screen_rect.max.x as f32 + x as f32, screen_rect.max.y as f32 + y as f32),
            };
            vec![
                gl_rect.min.x, gl_rect.max.y, uv_rect.min.x, uv_rect.max.y,
                gl_rect.min.x, gl_rect.min.y, uv_rect.min.x, uv_rect.min.y,
                gl_rect.max.x, gl_rect.min.y, uv_rect.max.x, uv_rect.min.y,
                gl_rect.max.x, gl_rect.min.y, uv_rect.max.x, uv_rect.min.y,
                gl_rect.max.x, gl_rect.max.y, uv_rect.max.x, uv_rect.max.y,
                gl_rect.min.x, gl_rect.max.y, uv_rect.min.x, uv_rect.max.y,
            ]
        }).collect();
        
        // create vao and upload vertex data to gpu
        let mut vao = 0;
        let mut vbo = 0;
        unsafe {
            let mut polygon_mode = 0;
            gl::GetIntegerv(gl::POLYGON_MODE, &mut polygon_mode);
            if polygon_mode != gl::FILL as i32 {
                gl::PolygonMode(gl::FRONT_AND_BACK, gl::FILL);
            }

            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(gl::ARRAY_BUFFER, (vertices.len() * std::mem::size_of::<f32>()) as isize, vertices.as_ptr() as *const std::ffi::c_void, gl::STATIC_DRAW);
            let stride = 4 * std::mem::size_of::<f32>() as i32;
            gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, stride, std::ptr::null());
            gl::EnableVertexAttribArray(0);
            let dummy = [0.0, 0.0];
            gl::VertexAttribPointer(1, 2, gl::FLOAT, gl::FALSE, stride, (dummy.len() * std::mem::size_of::<f32>()) as *const GLvoid);
            gl::EnableVertexAttribArray(1);

            // set shader uniforms
            renderer.shader.bind();
            let projection = cgmath::ortho(0.0, renderer.width as f32, renderer.height as f32, 0.0, -1.0, 100.0);
            renderer.shader.set_uniform_mat4("projection", &projection);
            renderer.shader.set_uniform_3f("color", 1.0, 1.0, 1.0);

            // draw text
            gl::Disable(gl::DEPTH_TEST);
            gl::Disable(gl::CULL_FACE);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            renderer.shader.set_uniform_1i("texture0", 0);
            gl::DrawArrays(gl::TRIANGLES, 0, vertices.len() as i32 / 4);

            // cleanup
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::DeleteVertexArrays(1, &vao);
            gl::DeleteBuffers(1, &vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
            gl::Disable(gl::BLEND);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 4);

            if polygon_mode != gl::FILL as i32 {
                gl::PolygonMode(gl::FRONT_AND_BACK, polygon_mode as u32);
            }
        }
        (max_x, max_y)
    }

    pub fn resize(width: u32, height: u32) {
        let mut renderer = RENDERER.lock().unwrap();
        renderer.width = width;
        renderer.height = height;
    }

    pub fn resize_from_event(event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::FramebufferSize(width, height) => {
                TextRenderer::resize(*width as u32, *height as u32);
            }
            _ => {}
        }
    }

    fn layout<'a>(&self, scale: Scale, width: u32, text: &str) -> Vec<PositionedGlyph<'a>> {
        let mut result = Vec::new();
        let v_metrics = self.font.v_metrics(scale);
        let advance_height = v_metrics.ascent - v_metrics.descent + v_metrics.line_gap;
        let mut caret = point(0.0, v_metrics.ascent);
        let mut last_glyph_id = None;
        for c in text.chars() {
            if c.is_control() {
                match c {
                    '\r' => {
                        caret = point(0.0, caret.y + advance_height);
                    }
                    '\n' => {}
                    _ => {}
                }
                continue;
            }
            let base_glyph = self.font.glyph(c);
            if let Some(id) = last_glyph_id.take() {
                caret.x += self.font.pair_kerning(scale, id, base_glyph.id());
            }
            last_glyph_id = Some(base_glyph.id());
            let mut glyph = base_glyph.scaled(scale).positioned(caret);
            if let Some(bb) = glyph.pixel_bounding_box() {
                if bb.max.x > width as i32 {
                    caret = point(0.0, caret.y + advance_height);
                    glyph.set_position(caret);
                    last_glyph_id = None;
                }
            }
            caret.x += glyph.unpositioned().h_metrics().advance_width;
            result.push(glyph);
        }
        result
    }
}

impl Texture {
    pub fn new(width: i32, height: i32) -> Texture {
        let mut texture_buffer = 0;
        let data = vec![0u8; width as usize * height as usize];
        unsafe {
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
            gl::GenTextures(1, &mut texture_buffer);
            gl::BindTexture(gl::TEXTURE_2D, texture_buffer);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::R8 as i32, width, height, 0, gl::RED, gl::UNSIGNED_BYTE, data.as_ptr() as *const std::ffi::c_void);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 4);
        }

        Texture { id: texture_buffer }
    }

    pub fn bind(&self) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, self.id);
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