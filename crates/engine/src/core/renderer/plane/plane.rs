use crate::core::{
    primitives::{Position, Region, Size},
    renderer::shader::{DynamicVertexArray, Shader, VertexAttributes},
};

use super::{Plane, PlaneBuilder, PlaneRenderer, PlaneVertex};
use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref RENDERER: Mutex<PlaneRenderer> = Mutex::new(PlaneRenderer::new(1280.0, 720.0));
}

impl PlaneRenderer {
    fn new(width: f32, height: f32) -> Self {
        Self {
            shader: Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl")),
            width,
            height,
        }
    }
    pub fn render(plane: &Plane) {
        let renderer = RENDERER.lock().unwrap();
        // calculate plane vertices

        plane.vertex_array.bind();
        renderer.shader.bind();
        let ortho = cgmath::ortho(0.0, renderer.width, renderer.height, 0.0, -100.0, 100.0);
        renderer.shader.set_uniform_mat4("projection", &ortho);
        renderer
            .shader
            .set_uniform_1f("borderThickness", plane.border_thickness);
        renderer.shader.set_uniform_4f(
            "borderRadius",
            plane.border_radius.0,
            plane.border_radius.1,
            plane.border_radius.2,
            plane.border_radius.3,
        );
        renderer.shader.set_uniform_4f(
            "borderColor",
            plane.border_color.0,
            plane.border_color.1,
            plane.border_color.2,
            plane.border_color.3,
        );
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::DrawElements(
                gl::TRIANGLES,
                plane.vertex_array.get_element_count() as i32,
                gl::UNSIGNED_INT,
                std::ptr::null(),
            );
        }
    }

    pub fn resize(width: u32, height: u32) {
        let mut renderer = RENDERER.lock().unwrap();
        renderer.width = width as f32;
        renderer.height = height as f32;
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

impl PlaneBuilder {
    pub fn new() -> Self {
        Self {
            position: Position::default(),
            size: Size::default(),
            color: (0.0, 0.0, 0.0, 0.0),
            border_thickness: 0.0,
            border_color: (0.0, 0.0, 0.0, 1.0),
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
        let indices: Vec<u32> = vec![0, 1, 2, 2, 3, 0];
        let vertex_array = DynamicVertexArray::<PlaneVertex>::new();
        let mut plane = Self {
            region: Region::new(position, size),
            color,
            border_thickness,
            border_color,
            border_radius,
            vertex_array,
        };
        let vertices = plane.get_vertices();
        plane.vertex_array.buffer_data(&vertices, &Some(indices));
        plane
    }

    pub fn render(&self) {
        PlaneRenderer::render(self);
    }

    pub fn get_region(&self) -> &Region {
        &self.region
    }

    fn get_vertices(&self) -> Vec<PlaneVertex> {
        vec![
            PlaneVertex {
                position: (
                    self.region.position.x,
                    self.region.position.y + self.region.size.height,
                    self.region.position.z,
                ),
                color: self.color,
                dimensions: (
                    self.region.size.width,
                    self.region.size.height,
                    self.region.position.x,
                    self.region.position.y,
                ),
            },
            PlaneVertex {
                position: (
                    self.region.position.x + self.region.size.width,
                    self.region.position.y + self.region.size.height,
                    self.region.position.z,
                ),
                color: self.color,
                dimensions: (
                    self.region.size.width,
                    self.region.size.height,
                    self.region.position.x,
                    self.region.position.y,
                ),
            },
            PlaneVertex {
                position: (
                    self.region.position.x + self.region.size.width,
                    self.region.position.y,
                    self.region.position.z,
                ),
                color: self.color,
                dimensions: (
                    self.region.size.width,
                    self.region.size.height,
                    self.region.position.x,
                    self.region.position.y,
                ),
            },
            PlaneVertex {
                position: (
                    self.region.position.x,
                    self.region.position.y,
                    self.region.position.z,
                ),
                color: self.color,
                dimensions: (
                    self.region.size.width,
                    self.region.size.height,
                    self.region.position.x,
                    self.region.position.y,
                ),
            },
        ]
    }

    pub fn set_position(&mut self, position: Position) {
        self.region.position = position;
        self.recalculate_vertices();
    }

    pub fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.recalculate_vertices();
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.recalculate_vertices();
    }

    pub fn set_color(&mut self, color: (f32, f32, f32, f32)) {
        self.color = color;
        self.recalculate_vertices();
    }

    fn recalculate_vertices(&mut self) {
        let vertices = self.get_vertices();
        let indices: Vec<u32> = vec![0, 1, 2, 2, 3, 0];
        self.vertex_array.buffer_data(&vertices, &Some(indices));
    }
}

impl VertexAttributes for PlaneVertex {
    fn get_vertex_attributes() -> Vec<(usize, gl::types::GLuint)> {
        vec![(3, gl::FLOAT), (4, gl::FLOAT), (4, gl::FLOAT)]
    }
}
