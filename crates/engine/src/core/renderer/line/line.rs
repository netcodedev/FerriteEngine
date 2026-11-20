use cgmath::{Matrix4, Point3, Vector3};

use super::{Line, LineRenderer, Shader};
use crate::core::gfx;

use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref RENDERER: Mutex<LineRenderer> = Mutex::new(LineRenderer::new());
}

impl Line {
    pub fn new(position: Point3<f32>, direction: Vector3<f32>, length: f32) -> Self {
        Self {
            position,
            direction,
            length,
        }
    }
}

/// A line's ends: positions at location 0.
fn layout() -> gfx::VertexLayout {
    gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x3])
}

impl LineRenderer {
    fn new() -> Self {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        let vertices = gfx::Buffer::new(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Dynamic, 0);
        Self { shader, vertices }
    }

    pub fn render(
        view_projection: &Matrix4<f32>,
        line: &Line,
        color: Vector3<f32>,
        always_on_top: bool,
    ) {
        Self::render_lines(view_projection, &vec![line.clone()], color, always_on_top);
    }

    pub fn render_lines(
        view_projection: &Matrix4<f32>,
        lines: &Vec<Line>,
        color: Vector3<f32>,
        always_on_top: bool,
    ) {
        let mut ends: Vec<f32> = Vec::with_capacity(lines.len() * 6);
        for line in lines {
            let end = line.position + line.direction * line.length;
            ends.extend_from_slice(&[line.position.x, line.position.y, line.position.z, end.x, end.y, end.z]);
        }
        if ends.is_empty() {
            return;
        }
        let mut renderer = RENDERER.lock().unwrap();
        let renderer = &mut *renderer;
        // On top: no depth test (which writes no depth either).
        let desc = gfx::PipelineDesc {
            vertex_layouts: crate::core::gfx::Layouts::of(&[layout()]),
            topology: gfx::Topology::Lines,
            depth: if always_on_top { gfx::DepthState::OFF } else { gfx::DepthState::LESS },
            ..Default::default()
        };
        gfx::set_pipeline(renderer.shader.pipeline(&desc));
        renderer.shader.set_uniform_mat4("viewProjection", view_projection);
        renderer.shader.set_uniform_3fv("color", &color);
        // A fresh store: lines drawn before may still be read from the old.
        renderer.vertices.replace(gfx::as_bytes(&ends));
        gfx::set_vertex_buffer(0, &renderer.vertices, 0, layout().stride);
        gfx::draw(0, (ends.len() / 3) as u32);
    }
}
