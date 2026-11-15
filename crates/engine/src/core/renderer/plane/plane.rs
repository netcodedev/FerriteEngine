use gl::types::{GLint, GLintptr, GLsizei, GLsizeiptr, GLvoid};

use crate::core::{
    primitives::{Position, Region, Size},
    renderer::shader::{Shader, VertexAttributes},
};

use super::{Plane, PlaneBuilder, PlaneRenderer, PlaneVertex};
use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref RENDERER: Mutex<PlaneRenderer> = Mutex::new(PlaneRenderer::new(1280.0, 720.0));
}

/// Planes the stream buffer holds before it is orphaned and refilled.
const STREAM_PLANES: usize = 4096;
const QUAD: [u32; 6] = [0, 1, 2, 2, 3, 0];

impl PlaneRenderer {
    fn new(width: f32, height: f32) -> Self {
        Self {
            shader: Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl")),
            width,
            height,
            projection_for: None,
            buffers: None,
            written: 0,
            scratch: Vec::new(),
        }
    }

    pub fn render(plane: &Plane) {
        Self::render_batch(std::iter::once(plane));
    }

    /// Draw `planes` in their order, in as few draw calls as fit the
    /// stream buffer (one, usually), as if each were drawn on its own.
    pub fn render_batch<'a>(planes: impl IntoIterator<Item = &'a Plane>) {
        if crate::core::headless::is_headless() {
            return;
        }
        let mut renderer = RENDERER.lock().unwrap();
        let renderer = &mut *renderer;
        let mut vertices = std::mem::take(&mut renderer.scratch);
        vertices.clear();
        for plane in planes {
            vertices.extend_from_slice(&plane.get_vertices());
        }
        if !vertices.is_empty() {
            renderer.bind();
            for chunk in vertices.chunks(STREAM_PLANES * 4) {
                renderer.draw(chunk);
            }
            unsafe { gl::BindVertexArray(0) };
        }
        renderer.scratch = vertices;
    }

    fn bind(&mut self) {
        let (vao, vbo, _) = *self.buffers.get_or_insert_with(create_buffers);
        self.shader.bind();
        let size = (self.width, self.height);
        if self.projection_for != Some(size) {
            let ortho = cgmath::ortho(0.0, self.width, self.height, 0.0, -100.0, 100.0);
            self.shader.set_uniform_mat4("projection", &ortho);
            self.projection_for = Some(size);
        }
        unsafe {
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
        }
    }

    /// Stream `vertices` (whole planes, at most `STREAM_PLANES`) and draw
    /// them. The buffer is bound.
    fn draw(&mut self, vertices: &[PlaneVertex]) {
        let planes = vertices.len() / 4;
        let stride = std::mem::size_of::<PlaneVertex>();
        unsafe {
            if self.written + planes > STREAM_PLANES {
                // Orphan it: the GPU keeps what it still draws from.
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    (STREAM_PLANES * 4 * stride) as GLsizeiptr,
                    std::ptr::null(),
                    gl::STREAM_DRAW,
                );
                self.written = 0;
            }
            gl::BufferSubData(
                gl::ARRAY_BUFFER,
                (self.written * 4 * stride) as GLintptr,
                (vertices.len() * stride) as GLsizeiptr,
                vertices.as_ptr() as *const GLvoid,
            );
            gl::DrawElementsBaseVertex(
                gl::TRIANGLES,
                (planes * 6) as GLsizei,
                gl::UNSIGNED_INT,
                std::ptr::null(),
                (self.written * 4) as GLint,
            );
        }
        self.written += planes;
    }

    pub fn resize(width: u32, height: u32) {
        let mut renderer = RENDERER.lock().unwrap();
        renderer.width = width as f32;
        renderer.height = height as f32;
    }

    pub fn get_size() -> (f32, f32) {
        let renderer = RENDERER.lock().unwrap();
        (renderer.width, renderer.height)
    }

    pub fn resize_from_event(event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::FramebufferSize(width, height) => {
                Self::resize(*width as u32, *height as u32);
            }
            _ => {}
        }
    }
}

/// The vertex array, the stream buffer and the index buffer of
/// `STREAM_PLANES` quads.
fn create_buffers() -> (u32, u32, u32) {
    let (mut vao, mut vbo, mut ibo) = (0, 0, 0);
    let stride = std::mem::size_of::<PlaneVertex>();
    let indices: Vec<u32> = (0..STREAM_PLANES as u32)
        .flat_map(|plane| QUAD.map(|i| plane * 4 + i))
        .collect();
    unsafe {
        gl::GenVertexArrays(1, &mut vao);
        gl::GenBuffers(1, &mut vbo);
        gl::GenBuffers(1, &mut ibo);
        gl::BindVertexArray(vao);
        gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
        gl::BufferData(
            gl::ARRAY_BUFFER,
            (STREAM_PLANES * 4 * stride) as GLsizeiptr,
            std::ptr::null(),
            gl::STREAM_DRAW,
        );
        let mut offset = 0;
        for (location, (size, _)) in PlaneVertex::get_vertex_attributes().into_iter().enumerate() {
            gl::EnableVertexAttribArray(location as u32);
            gl::VertexAttribPointer(
                location as u32,
                size as GLint,
                gl::FLOAT,
                gl::FALSE,
                stride as GLsizei,
                offset as *const GLvoid,
            );
            offset += size * std::mem::size_of::<f32>();
        }
        gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ibo);
        gl::BufferData(
            gl::ELEMENT_ARRAY_BUFFER,
            (indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
            indices.as_ptr() as *const GLvoid,
            gl::STATIC_DRAW,
        );
        gl::BindVertexArray(0);
    }
    (vao, vbo, ibo)
}

impl PlaneBuilder {
    pub fn new() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            color: (0.0, 0.0, 0.0, 0.0),
            border_thickness: 0.0,
            // Transparent by default: even with zero thickness the shader
            // blends the anti-aliased edge towards the border colour, which
            // left a dark outline around borderless planes.
            border_color: (0.0, 0.0, 0.0, 0.0),
            border_radius: (0.0, 0.0, 0.0, 0.0),
        }
    }
    pub fn position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn color(mut self, color: (f32, f32, f32, f32)) -> Self {
        self.color = color;
        self
    }
    pub fn border_thickness(mut self, border_thickness: f32) -> Self {
        self.border_thickness = border_thickness;
        self
    }
    pub fn border_color(mut self, border_color: (f32, f32, f32, f32)) -> Self {
        self.border_color = border_color;
        self
    }
    pub fn border_radius(mut self, border_radius: (f32, f32, f32, f32)) -> Self {
        self.border_radius = (
            border_radius.2,
            border_radius.1,
            border_radius.3,
            border_radius.0,
        );
        self
    }
    pub fn border_radius_uniform(mut self, border_radius: f32) -> Self {
        self.border_radius = (border_radius, border_radius, border_radius, border_radius);
        self
    }
    pub fn build(self) -> Plane {
        Plane::new(
            self.position,
            self.size,
            self.color,
            self.border_thickness,
            self.border_color,
            self.border_radius,
        )
    }
}

impl Plane {
    pub fn new(
        position: Position,
        size: Size,
        color: (f32, f32, f32, f32),
        border_thickness: f32,
        border_color: (f32, f32, f32, f32),
        border_radius: (f32, f32, f32, f32),
    ) -> Self {
        Self {
            region: Region::new(position, size),
            color,
            border_thickness,
            border_color,
            border_radius,
        }
    }

    /// Draw the plane. Fully invisible planes (transparent fill, no visible
    /// border) are skipped: they would only write depth and could hide
    /// whatever is drawn behind them later. Stencil clipping calls
    /// `PlaneRenderer::render` directly and is unaffected.
    pub fn render(&self) {
        if self.is_visible() {
            PlaneRenderer::render(self);
        }
    }

    /// Draw several planes as `render` draws each, in one draw call.
    pub fn render_all<'a>(planes: impl IntoIterator<Item = &'a Plane>) {
        PlaneRenderer::render_batch(planes.into_iter().filter(|plane| plane.is_visible()));
    }

    fn is_visible(&self) -> bool {
        let border_visible = self.border_thickness > 0.0 && self.border_color.3 > 0.0;
        self.color.3 > 0.0 || border_visible
    }

    pub fn get_region(&self) -> &Region {
        &self.region
    }

    fn get_vertices(&self) -> [PlaneVertex; 4] {
        let (position, size) = (&self.region.position, &self.region.size);
        let corner = |x: f32, y: f32| PlaneVertex {
            position: (x, y, position.z),
            color: self.color,
            dimensions: (size.width, size.height, position.x, position.y),
            border_color: self.border_color,
            border_radius: self.border_radius,
            border_thickness: self.border_thickness,
        };
        [
            corner(position.x, position.y + size.height),
            corner(position.x + size.width, position.y + size.height),
            corner(position.x + size.width, position.y),
            corner(position.x, position.y),
        ]
    }

    pub fn set_position(&mut self, position: Position) {
        self.region.position = position;
    }

    pub fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
    }

    pub fn set_color(&mut self, color: (f32, f32, f32, f32)) {
        self.color = color;
    }
}

impl VertexAttributes for PlaneVertex {
    fn get_vertex_attributes() -> Vec<(usize, gl::types::GLuint)> {
        vec![
            (3, gl::FLOAT),
            (4, gl::FLOAT),
            (4, gl::FLOAT),
            (4, gl::FLOAT),
            (4, gl::FLOAT),
            (1, gl::FLOAT),
        ]
    }
}
