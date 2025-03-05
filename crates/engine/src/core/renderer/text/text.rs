use rusttype::gpu_cache::Cache;
use rusttype::{point, PositionedGlyph, Rect, Scale};

use crate::core::primitives::{Position, Size};
use crate::core::renderer::shader::{DynamicVertexArray, VertexAttributes};
use crate::core::renderer::text::Fonts;

use super::{Font, Shader, Text, TextMesh, TextRenderer, TextVertex, Texture};

use lazy_static::lazy_static;
use std::sync::{Mutex, OnceLock};

lazy_static! {
    static ref RENDERER: Mutex<TextRenderer> = Mutex::new(TextRenderer::new(1280, 720));
}

impl Font {
    fn new(font_data: &'static [u8]) -> Self {
        Font {
            font: rusttype::Font::try_from_bytes(font_data).unwrap(),
        }
    }
}

impl Fonts {
    fn get(&self) -> &Font {
        static ROBOTO_MONO: OnceLock<Font> = OnceLock::new();

        match self {
            Fonts::RobotoMono => {
                ROBOTO_MONO.get_or_init(|| Font::new(include_bytes!("RobotoMono.ttf")))
            }
        }
    }
}

impl Text {
    pub fn new(font: Fonts, x: i32, y: i32, z: i32, size: f32, content: String) -> Text {
        let mut text = Text {
            content,
            font,
            size,
            glyphs: Vec::new(),
            dirty: true,
            x,
            y,
            z,
            mesh: TextMesh::new(),
            max_x: x,
            max_y: y,
        };
        text.layout(TextRenderer::get_size().0);
        text
    }

    pub fn render(&self) -> (i32, i32) {
        TextRenderer::render(self)
    }

    pub fn prepare_render_at(&mut self, position: Position) {
        let (x, y, z) = (position.x as i32, position.y as i32, position.z as i32);
        if self.x == x && self.y == y && self.z == z {
            return;
        }
        self.x = x;
        self.y = y;
        self.z = z;
        self.layout(TextRenderer::get_size().0);
    }

    pub fn set_content(&mut self, content: &str) {
        if self.content == content {
            return;
        }
        self.content = content.to_owned();
        self.dirty = true;
        self.layout(TextRenderer::get_size().0);
    }

    pub fn set_z_index(&mut self, z_index: f32) {
        if self.z == z_index as i32 {
            return;
        }
        self.z = z_index as i32;
        self.dirty = true;
        self.layout(TextRenderer::get_size().0);
    }

    pub fn get_size(&self) -> Size {
        Size {
            width: self.max_x as f32,
            height: self.max_y as f32,
        }
    }

    fn layout(&mut self, width: u32) {
        if self.dirty {
            self.glyphs = self.layout_text(Scale::uniform(self.size), width, &self.content);
            self.dirty = false;
        }
        self.update_mesh();
    }

    fn update_mesh(&mut self) {
        let vertices: Vec<TextVertex> = self
            .glyphs
            .iter()
            .filter_map(|g| TextRenderer::rect_for(0, g.clone()))
            .flat_map(|(uv_rect, screen_rect)| {
                if self.max_x < screen_rect.max.x as i32 {
                    self.max_x = screen_rect.max.x as i32;
                }
                if self.max_y < screen_rect.max.y as i32 {
                    self.max_y = screen_rect.max.y as i32;
                }
                let gl_rect = Rect {
                    min: point(
                        screen_rect.min.x as f32 + self.x as f32,
                        screen_rect.min.y as f32 + self.y as f32,
                    ),
                    max: point(
                        screen_rect.max.x as f32 + self.x as f32,
                        screen_rect.max.y as f32 + self.y as f32,
                    ),
                };
                vec![
                    TextVertex {
                        position: (gl_rect.min.x, gl_rect.max.y, self.z as f32),
                        texture_coords: (uv_rect.min.x, uv_rect.max.y),
                    },
                    TextVertex {
                        position: (gl_rect.min.x, gl_rect.min.y, self.z as f32),
                        texture_coords: (uv_rect.min.x, uv_rect.min.y),
                    },
                    TextVertex {
                        position: (gl_rect.max.x, gl_rect.min.y, self.z as f32),
                        texture_coords: (uv_rect.max.x, uv_rect.min.y),
                    },
                    TextVertex {
                        position: (gl_rect.max.x, gl_rect.min.y, self.z as f32),
                        texture_coords: (uv_rect.max.x, uv_rect.min.y),
                    },
                    TextVertex {
                        position: (gl_rect.max.x, gl_rect.max.y, self.z as f32),
                        texture_coords: (uv_rect.max.x, uv_rect.max.y),
                    },
                    TextVertex {
                        position: (gl_rect.min.x, gl_rect.max.y, self.z as f32),
                        texture_coords: (uv_rect.min.x, uv_rect.max.y),
                    },
                ]
            })
            .collect();
        self.mesh.update_vertices(vertices);
    }

    fn layout_text<'a>(&self, scale: Scale, width: u32, text: &str) -> Vec<PositionedGlyph<'a>> {
        let font = &self.font.get().font;
        let mut result = Vec::new();
        let v_metrics = font.v_metrics(scale);
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
            let base_glyph = font.glyph(c);
            if let Some(id) = last_glyph_id.take() {
                caret.x += font.pair_kerning(scale, id, base_glyph.id());
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

impl TextRenderer {
    fn new(width: u32, height: u32) -> TextRenderer {
        let cache: Cache<'static> = Cache::builder().dimensions(1024, 1024).build();

        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        TextRenderer {
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
    pub fn render(text: &Text) -> (i32, i32) {
        let renderer = RENDERER.lock().unwrap();
        let mut polygon_mode = 0;
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
            renderer.texture_buffer.bind();
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);

            gl::GetIntegerv(gl::POLYGON_MODE, &mut polygon_mode);
            if polygon_mode != gl::FILL as i32 {
                gl::PolygonMode(gl::FRONT_AND_BACK, gl::FILL);
            }
        }

        text.mesh.vertex_array.bind();

        // set shader uniforms
        renderer.shader.bind();
        let projection = cgmath::ortho(
            0.0,
            renderer.width as f32,
            renderer.height as f32,
            0.0,
            -100.0,
            100.0,
        );
        renderer.shader.set_uniform_mat4("projection", &projection);
        renderer.shader.set_uniform_3f("color", 1.0, 1.0, 1.0);

        unsafe {
            // draw text
            gl::Enable(gl::DEPTH_TEST);
            gl::Disable(gl::CULL_FACE);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            renderer.shader.set_uniform_1i("texture0", 0);
            gl::DrawArrays(
                gl::TRIANGLES,
                0,
                text.mesh.vertex_array.get_element_count() as i32,
            );

            // cleanup
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
            gl::Disable(gl::BLEND);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 4);

            if polygon_mode != gl::FILL as i32 {
                gl::PolygonMode(gl::FRONT_AND_BACK, polygon_mode as u32);
            }
        }
        (text.max_x, text.max_y)
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

    pub fn get_size() -> (u32, u32) {
        let renderer = RENDERER.lock().unwrap();
        (renderer.width, renderer.height)
    }

    pub fn rect_for(
        font_id: usize,
        glyph: PositionedGlyph<'static>,
    ) -> Option<(Rect<f32>, Rect<i32>)> {
        let mut renderer = RENDERER.lock().unwrap();
        renderer.cache.queue_glyph(0, glyph.clone());
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
                gl::RED,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const std::ffi::c_void,
            );
        });
        renderer.cache.rect_for(font_id, &glyph).ok().flatten()
    }
}

impl TextMesh {
    fn new() -> TextMesh {
        TextMesh {
            vertex_array: DynamicVertexArray::new(),
            vertices: Vec::new(),
        }
    }

    fn update_vertices(&mut self, vertices: Vec<TextVertex>) {
        self.vertices = vertices;
        self.vertex_array.buffer_data(&self.vertices, &None);
    }
}

impl VertexAttributes for TextVertex {
    fn get_vertex_attributes() -> Vec<(usize, gl::types::GLuint)> {
        vec![(3, gl::FLOAT), (2, gl::FLOAT)]
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
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::R8 as i32,
                width,
                height,
                0,
                gl::RED,
                gl::UNSIGNED_BYTE,
                data.as_ptr() as *const std::ffi::c_void,
            );
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
