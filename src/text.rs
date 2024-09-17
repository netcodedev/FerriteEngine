use rusttype::gpu_cache::Cache;
use rusttype::{point, Font, Rect, PositionedGlyph, Scale};
use crate::shader::create_shader_program;
use gl::types::{GLuint, GLvoid};
use cgmath::Matrix;

pub struct TextRenderer {
    font: Font<'static>,
    cache: Cache<'static>,
    shader_program: u32,
    texture_buffer: Texture,
    width: u32,
    height: u32,
}

impl TextRenderer {
    pub fn new(width: u32, height: u32) -> TextRenderer {
        let font_data = include_bytes!("font/RobotoMono.ttf");
        let font = Font::try_from_bytes(font_data as &[u8]).unwrap();

        let cache: Cache<'static> = Cache::builder().dimensions(1024, 1024).build();

        let vertex_source = include_str!("shaders/text_vertex.glsl");
        let fragment_source = include_str!("shaders/text_fragment.glsl");
        let shader_program = create_shader_program(&vertex_source, &fragment_source);

        TextRenderer {
            font,
            cache,
            shader_program,
            texture_buffer: Texture::new(1024, 1024),
            width,
            height,
        }
    }

    pub fn render(&mut self, x: u32, y: u32, size: f32, text: &str) {
        let glyphs = self.layout(Scale::uniform(size), self.width, &text);
        for glyph in &glyphs {
            self.cache.queue_glyph(0, glyph.clone());
        }
        let _ = self.cache.cache_queued(|rect, data| unsafe {
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
        
        let vertices: Vec<f32> = glyphs.iter().filter_map(|g| self.cache.rect_for(0, g).ok().flatten()).flat_map(|(uv_rect, screen_rect)| {
            let gl_rect = Rect {
                min: point(-1.0 + (screen_rect.min.x as f32 + x as f32) / self.width as f32, 1.0 - (screen_rect.min.y as f32 + y as f32) / self.height as f32),
                max: point(-1.0 + (screen_rect.max.x as f32 + x as f32) / self.width as f32, 1.0 - (screen_rect.max.y as f32 + y as f32) / self.height as f32),
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
            gl::UseProgram(self.shader_program);
            let projection = cgmath::ortho(0.0, 1000.0, 0.0, 1000.0, 0.1, 100.0);
            let color_loc = gl::GetUniformLocation(self.shader_program, "color\0".as_ptr() as *const i8);
            let projection_loc = gl::GetUniformLocation(self.shader_program, "projection\0".as_ptr() as *const i8);
            gl::Uniform3f(color_loc, 1.0, 1.0, 1.0);
            gl::UniformMatrix4fv(projection_loc, 1, gl::FALSE, projection.as_ptr());

            // draw text
            gl::Disable(gl::DEPTH_TEST);
            gl::Disable(gl::CULL_FACE);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::ActiveTexture(gl::TEXTURE0);
            self.texture_buffer.bind();
            gl::DrawArrays(gl::TRIANGLES, 0, vertices.len() as i32 / 4);

            // cleanup
            gl::DeleteVertexArrays(1, &vao);
            gl::DeleteBuffers(1, &vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
            gl::Disable(gl::BLEND);
        }
    }

    pub fn resize(&mut self, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::FramebufferSize(width, height) => {
                self.width = *width as u32;
                self.height = *height as u32;
            }
            _ => {}
        }
    }

    pub fn layout<'a>(&self, scale: Scale, width: u32, text: &str) -> Vec<PositionedGlyph<'a>> {
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

struct Texture {
    id: GLuint,
}

impl Texture {
    pub fn new(width: i32, height: i32) -> Texture {
        let mut texture_buffer = 0;
        let data = vec![0u8; width as usize * height as usize];
        unsafe {
            gl::GenTextures(1, &mut texture_buffer);
            gl::BindTexture(gl::TEXTURE_2D, texture_buffer);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(gl::TEXTURE_2D, 0, gl::R8 as i32, width, height, 0, gl::RED, gl::UNSIGNED_BYTE, data.as_ptr() as *const std::ffi::c_void);
            gl::GenerateMipmap(gl::TEXTURE_2D);
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