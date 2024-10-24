use gl::types::GLuint;
use glfw::MouseButton;
use libnoise::prelude::*;

use crate::{
    camera::{Camera, Projection},
    line::Line,
    shader::{Shader, VertexAttributes},
    terrain::{Chunk, ChunkBounds},
};

use fast_surface_nets::{ndshape::{ConstShape, ConstShape3u32}, {surface_nets, SurfaceNetsBuffer}};

use super::{ChunkMesh, DualContouringChunk, Vertex, CHUNK_SIZE, CHUNK_SIZE_FLOAT};

impl DualContouringChunk {
    fn get_density_at(&self, (x, y, z): (usize, usize, usize)) -> f32 {
        let offset: f64 = 16777216.0;
        let sample_point = (
            (self.position.0 * CHUNK_SIZE_FLOAT) as f64 + x as f64 + offset,
            (self.position.1 * CHUNK_SIZE_FLOAT) as f64 + y as f64 + offset,
            (self.position.2 * CHUNK_SIZE_FLOAT) as f64 + z as f64 + offset,
        );

        let noise = ((1.0 + self.noise.sample([sample_point.0, sample_point.2])) / 2.0) as f32;
        let iso = self
                .cave
                .sample([sample_point.0, sample_point.1, sample_point.2]) as f32;
        let height_iso = 1.0 - ((noise) / ((1.0 + y as f32) / CHUNK_SIZE_FLOAT));
        height_iso
    }

    fn generate_mesh(&self) -> ChunkMesh<Vertex> {
        let mut vertices = Vec::<Vertex>::new();
        let mut indices = Vec::<u32>::new();
        type ChunkShape = ConstShape3u32<{CHUNK_SIZE as u32 + 2}, {CHUNK_SIZE as u32 + 2}, {CHUNK_SIZE as u32 + 2}>;
        let mut sdf = vec![0.0; ChunkShape::USIZE];
        for i in 0..ChunkShape::SIZE {
            let [x, y, z] = ChunkShape::delinearize(i);
            sdf[i as usize] = self.get_density_at((x as usize, y as usize, z as usize));
        }
        let mut buffer = SurfaceNetsBuffer::default();
        surface_nets(&sdf, &ChunkShape {}, [0; 3], [CHUNK_SIZE as u32+1; 3], &mut buffer);
        for (i, vertex) in buffer.positions.into_iter().enumerate() {
            let normal = buffer.normals[i];
            vertices.push(Vertex {
                position: vertex,
                normal,
                color: [0.0, 0.5, 0.1],
            });
        }
        for index in buffer.indices {
            indices.push(index);
        }
        ChunkMesh::new(vertices, Some(indices))
    }

    fn calculate_chunk_size(lod: usize) -> usize {
        std::cmp::max(
            2,
            std::cmp::min(CHUNK_SIZE, CHUNK_SIZE / 2usize.pow(lod as u32 / 2)),
        )
    }
}

impl Chunk for DualContouringChunk {
    fn new(position: (f32, f32, f32), lod: usize) -> Self {
        let noise = Source::perlin(1).scale([0.003; 2]).fbm(6, 1.0, 2.0, 0.5);
        let cave = Source::perlin(1).scale([0.1; 3]);
        let mut chunk = Self {
            position,
            cave,
            noise,
            chunk_size: DualContouringChunk::calculate_chunk_size(lod),
            mesh: None,
        };
        chunk.mesh = Some(chunk.generate_mesh());
        chunk
    }

    fn render(&mut self, camera: &Camera, projection: &Projection, shader: &Shader) {
        if let Some(mesh) = &mut self.mesh {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            shader.bind();
            shader.set_uniform_mat4("view", &camera.calc_matrix());
            shader.set_uniform_mat4("projection", &projection.calc_matrix());
            unsafe {
                gl::Enable(gl::CULL_FACE);
            }
            mesh.render(
                &shader,
                (
                    self.position.0 * CHUNK_SIZE as f32,
                    self.position.1 * CHUNK_SIZE as f32,
                    self.position.2 * CHUNK_SIZE as f32,
                ),
                None,
            );
            unsafe {
                gl::Disable(gl::CULL_FACE);
            }
        }
    }

    fn get_bounds(&self) -> ChunkBounds {
        ChunkBounds {
            min: (
                (self.position.0 * CHUNK_SIZE as f32) as i32,
                (self.position.1 * CHUNK_SIZE as f32) as i32,
                (self.position.2 * CHUNK_SIZE as f32) as i32,
            ),
            max: (
                ((self.position.0 + 1.0) * CHUNK_SIZE as f32) as i32,
                ((self.position.1 + 1.0) * CHUNK_SIZE as f32) as i32,
                ((self.position.2 + 1.0) * CHUNK_SIZE as f32) as i32,
            ),
        }
    }

    fn process_line(&mut self, _: &Line, _: &MouseButton) -> bool {
        false
    }

    fn get_shader_source() -> (String, String) {
        (
            include_str!("vertex.glsl").to_string(),
            include_str!("fragment.glsl").to_string(),
        )
    }

    fn get_textures() -> Vec<crate::texture::Texture> {
        Vec::new()
    }
    
    fn get_triangle_count(&self) -> usize {
        if let Some(mesh) = &self.mesh {
            mesh.get_triangle_count()
        } else {
            0
        }
    }
}

impl VertexAttributes for Vertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (3, gl::FLOAT), (3, gl::FLOAT)]
    }
}
