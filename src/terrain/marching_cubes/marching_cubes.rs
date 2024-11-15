use core::panic;

use cgmath::{InnerSpace, Matrix4, Point3, Vector3, Zero};
use gl::types::GLuint;
use glfw::{Glfw, MouseButton, WindowEvent};
use libnoise::prelude::*;
use ndarray::ArrayBase;

use crate::{
    core::{
        entity::{component::Component, Entity},
        renderer::{line::Line, shader::VertexAttributes, texture::Texture},
        scene::Scene,
    },
    terrain::{Chunk, ChunkBounds, Terrain, CHUNK_SIZE_FLOAT},
};

use super::{ChunkMesh, MarchingCubesChunk, Vertex, CHUNK_SIZE, EDGES, POINTS, TRIANGULATIONS};

impl MarchingCubesChunk {
    fn generate_mesh(&self) -> ChunkMesh<Vertex> {
        let mut vertices = Vec::<Vertex>::new();
        let isovalue = 0.3;
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    vertices.extend(self.march_cube((x, y, z), isovalue));
                }
            }
        }
        ChunkMesh::new(vertices, None)
    }

    fn march_cube(&self, (x, y, z): (usize, usize, usize), isovalue: f32) -> Vec<Vertex> {
        let triangulation = self.get_triangulation((x, y, z), isovalue);

        let mut vertices = Vec::new();

        for i in 0..5 {
            let edge_index = triangulation[i * 3];

            if edge_index.is_negative() {
                break;
            }

            let mut positions: [Vector3<f32>; 3] = [Vector3::zero(); 3];

            for j in 0..3 {
                let l_edge = triangulation[i * 3 + j];
                let point_indices = EDGES[l_edge as usize];

                let (x0, y0, z0) = POINTS[point_indices.0 as usize];
                let (x1, y1, z1) = POINTS[point_indices.1 as usize];

                let pos_a = Vector3::new((x + x0) as f32, (y + y0) as f32, (z + z0) as f32);
                let pos_b = Vector3::new((x + x1) as f32, (y + y1) as f32, (z + z1) as f32);

                let position = (pos_a + pos_b) * 0.5;

                positions[j] = position;
            }

            let normal = MarchingCubesChunk::comute_normal(&positions);

            for position in positions {
                vertices.push(Vertex {
                    position: [position[0], position[1], position[2]],
                    normal: [normal.x, normal.y, normal.z],
                    color: [0.0, 0.5, 0.1],
                });
            }
        }

        vertices
    }

    fn get_triangulation(&self, (x, y, z): (usize, usize, usize), isovalue: f32) -> [i8; 15] {
        let mut config_idx = 0b00000000;

        config_idx |= if self.blocks[[x, y, z]] <= isovalue {
            1
        } else {
            0
        };
        config_idx |= if self.blocks[[x, y, z + 1]] <= isovalue {
            1
        } else {
            0
        } << 1;
        config_idx |= if self.blocks[[x + 1, y, z + 1]] <= isovalue {
            1
        } else {
            0
        } << 2;
        config_idx |= if self.blocks[[x + 1, y, z]] <= isovalue {
            1
        } else {
            0
        } << 3;
        config_idx |= if self.blocks[[x, y + 1, z]] <= isovalue {
            1
        } else {
            0
        } << 4;
        config_idx |= if self.blocks[[x, y + 1, z + 1]] <= isovalue {
            1
        } else {
            0
        } << 5;
        config_idx |= if self.blocks[[x + 1, y + 1, z + 1]] <= isovalue {
            1
        } else {
            0
        } << 6;
        config_idx |= if self.blocks[[x + 1, y + 1, z]] <= isovalue {
            1
        } else {
            0
        } << 7;

        return TRIANGULATIONS[config_idx as usize];
    }

    fn comute_normal(triangle: &[Vector3<f32>; 3]) -> Vector3<f32> {
        (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize()
    }
}

impl Chunk for MarchingCubesChunk {
    fn new(seed: u64, position: (f32, f32, f32), _: usize) -> Self {
        let generator = Source::perlin(seed).scale([0.003; 2]);
        let hills = Source::perlin(seed).scale([0.01; 2]);
        let tiny_hills = Source::perlin(seed).scale([0.1; 2]);
        let cave = Source::perlin(seed).scale([0.1; 3]);
        let offset: f64 = 16777216.0;
        let blocks: ArrayBase<ndarray::OwnedRepr<f32>, ndarray::Dim<[usize; 3]>> =
            ArrayBase::from_shape_fn(
                (CHUNK_SIZE + 1, CHUNK_SIZE + 1, CHUNK_SIZE + 1),
                |(x, y, z)| {
                    let sample_point = (
                        (position.0 * CHUNK_SIZE as f32) as f64 + x as f64 + offset,
                        (position.1 * CHUNK_SIZE as f32) as f64 + y as f64 + offset,
                        (position.2 * CHUNK_SIZE as f32) as f64 + z as f64 + offset,
                    );

                    let noise_value =
                        (1.0 + generator.sample([sample_point.0, sample_point.2])) / 2.0;
                    let hills_value =
                        (1.0 + hills.sample([sample_point.0, sample_point.2])) / 2.0 * 0.2;
                    let tiny_hills_value =
                        (1.0 + tiny_hills.sample([sample_point.0, sample_point.2])) / 2.0 * 0.01;
                    if ((noise_value + hills_value + tiny_hills_value) * CHUNK_SIZE as f64)
                        < y as f64
                    {
                        return 0.0;
                    }
                    (1.0 + cave.sample([sample_point.0, sample_point.1, sample_point.2]) as f32)
                        / 2.0
                },
            );
        let mut chunk = Self {
            position,
            blocks,
            mesh: None,
        };
        chunk.mesh = Some(chunk.generate_mesh());
        chunk
    }

    fn buffer_data(&mut self) {
        if let Some(mesh) = &mut self.mesh {
            mesh.buffer_data();
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

    fn get_position(&self) -> Point3<f32> {
        Point3::new(
            self.position.0 * CHUNK_SIZE_FLOAT,
            self.position.1 * CHUNK_SIZE_FLOAT,
            self.position.2 * CHUNK_SIZE_FLOAT,
        )
    }

    fn get_shader_source() -> (String, String) {
        (
            include_str!("vertex.glsl").to_string(),
            include_str!("fragment.glsl").to_string(),
        )
    }

    fn get_textures() -> Vec<Texture> {
        Vec::new()
    }

    fn get_triangle_count(&self) -> usize {
        if let Some(mesh) = &self.mesh {
            mesh.get_triangle_count()
        } else {
            0
        }
    }

    fn get_vertices(&self) -> Vec<[f32; 3]> {
        if let Some(mesh) = &self.mesh {
            mesh.vertices
                .iter()
                .map(|v| [v.position[0], v.position[1], v.position[2]])
                .collect()
        } else {
            Vec::new()
        }
    }

    fn get_indices(&self) -> Vec<[u32; 3]> {
        if let Some(mesh) = &self.mesh {
            if let Some(indices) = &mesh.indices {
                return indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
        }
        Vec::new()
    }
}

impl Component for MarchingCubesChunk {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        if let Some(terrain) = scene.get_component::<Terrain<MarchingCubesChunk>>() {
            let shader = terrain.get_shader();
            if let Some(mesh) = &self.mesh {
                if !mesh.is_buffered() {
                    panic!("Mesh is not buffered");
                }
                shader.bind();
                shader.set_uniform_mat4("viewProjection", &view_projection);
                unsafe {
                    gl::Enable(gl::CULL_FACE);
                }
                mesh.render(
                    &shader,
                    &(parent_transform
                        * Matrix4::from_translation(Vector3::new(
                            self.position.0 * CHUNK_SIZE_FLOAT,
                            self.position.1 * CHUNK_SIZE_FLOAT,
                            self.position.2 * CHUNK_SIZE_FLOAT,
                        ))),
                    None,
                );
                unsafe {
                    gl::Disable(gl::CULL_FACE);
                }
            }
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}

impl VertexAttributes for Vertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![(3, gl::FLOAT), (3, gl::FLOAT), (3, gl::FLOAT)]
    }
}
