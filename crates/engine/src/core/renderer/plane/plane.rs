use crate::core::{
    gfx,
    primitives::{Position, Region, Size},
    renderer::shader::{DynamicVertexArray, Shader, VertexAttributes},
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
        if !gfx::is_drawing() {
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
        }
        renderer.scratch = vertices;
    }

    fn bind(&mut self) {
        let desc = gfx::PipelineDesc {
            vertex_layouts: vec![DynamicVertexArray::<PlaneVertex>::layout()],
            depth: gfx::DepthState::LESS,
            blend: Some(gfx::BlendState::ALPHA),
            ..Default::default()
        };
        gfx::set_pipeline(self.shader.pipeline(&desc));
        let size = (self.width, self.height);
        if self.projection_for != Some(size) {
            let ortho = cgmath::ortho(0.0, self.width, self.height, 0.0, -100.0, 100.0);
            self.shader.set_uniform_mat4("projection", &ortho);
            self.projection_for = Some(size);
        }
        let (vertices, indices) = self.buffers.get_or_insert_with(create_buffers);
        gfx::set_vertex_buffer(0, vertices, 0, std::mem::size_of::<PlaneVertex>() as u32);
        gfx::set_index_buffer(indices, gfx::IndexFormat::U32);
    }

    /// Stream `vertices` (whole planes, at most `STREAM_PLANES`) and draw
    /// them. The buffers are bound.
    fn draw(&mut self, vertices: &[PlaneVertex]) {
        let planes = vertices.len() / 4;
        let stride = std::mem::size_of::<PlaneVertex>();
        let Some((buffer, _)) = self.buffers.as_mut() else {
            return;
        };
        if self.written + planes > STREAM_PLANES {
            // A fresh store: the GPU keeps what it still draws from.
            buffer.replace(gfx::as_bytes(vertices));
            self.written = 0;
        } else {
            buffer.write(self.written * 4 * stride, gfx::as_bytes(vertices));
        }
        gfx::draw_indexed(0, (planes * 6) as u32, (self.written * 4) as i32);
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

/// The stream vertex buffer and the index buffer of `STREAM_PLANES`
/// quads.
fn create_buffers() -> (gfx::Buffer, gfx::Buffer) {
    let stride = std::mem::size_of::<PlaneVertex>();
    let indices: Vec<u32> = (0..STREAM_PLANES as u32)
        .flat_map(|plane| QUAD.map(|i| plane * 4 + i))
        .collect();
    let vertices = gfx::Buffer::new(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Dynamic, STREAM_PLANES * 4 * stride);
    let indices = gfx::Buffer::from_slice(gfx::BufferUsage::INDEX, gfx::BufferMemory::Static, &indices);
    (vertices, indices)
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
    fn vertex_formats() -> Vec<crate::core::gfx::VertexFormat> {
        use crate::core::gfx::VertexFormat::*;
        vec![F32x3, F32x4, F32x4, F32x4, F32x4, F32]
    }
}
