use cgmath::{InnerSpace, Vector3, Zero};
use gl::types::GLuint;
use libnoise::prelude::*;
use ndarray::ArrayBase;

use crate::{camera::{Camera, Projection}, shader::{DynamicVertexArray, Shader, VertexAttributes}};

use super::{Chunk, ChunkMesh, Vertex, CHUNK_SIZE, EDGES, POINTS, TRIANGULATIONS};

impl Chunk {
    pub fn new(position: (f32, f32, f32)) -> Self {
        let height = Source::perlin(1).scale([0.003; 2]);
        let generator = Source::perlin(1).scale([0.1; 3]);
        let blocks: ArrayBase<ndarray::OwnedRepr<f32>, ndarray::Dim<[usize; 3]>> = ArrayBase::from_shape_fn((CHUNK_SIZE, CHUNK_SIZE, CHUNK_SIZE), |(x, y, z)| {
            let x = x as f64 + position.0 as f64;
            let y = y as f64 + position.1 as f64;
            let z = z as f64 + position.2 as f64;
            let h = (1.0 + height.sample([x, z])) / 2.0;
            if (h * CHUNK_SIZE as f64) < y as f64 {
                return 0.0;
            }
            (1.0 + generator.sample([x, y, z]) as f32) / 2.0
        });
        let mut chunk = Self {
            position,
            blocks,
            mesh: None,
            shader: Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl")),
        };
        chunk.mesh = Some(chunk.generate_mesh());
        chunk
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        if let Some(mesh) = &mut self.mesh {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            self.shader.bind();
            self.shader.set_uniform_mat4("view", &camera.calc_matrix());
            self.shader.set_uniform_mat4("projection", &projection.calc_matrix());
            mesh.render(&self.shader, self.position);
        }
    }

    fn generate_mesh(&self) -> ChunkMesh {
        let mut vertices = Vec::<Vertex>::new();
        let isovalue = 0.3;
        for z in 0..CHUNK_SIZE - 1 {
            for y in 0..CHUNK_SIZE - 1 {
                for x in 0..CHUNK_SIZE - 1 {
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
            
            let normal = Chunk::comute_normal(&positions);

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

    fn get_triangulation(&self, (x,y,z): (usize, usize, usize), isovalue: f32) -> [i8; 15] {
        let mut config_idx = 0b00000000;

        config_idx |= if self.blocks[[x,        y,      z       ]] <= isovalue { 1 } else { 0 };
        config_idx |= if self.blocks[[x,        y,      z + 1   ]] <= isovalue { 1 } else { 0 } << 1;
        config_idx |= if self.blocks[[x + 1,    y,      z + 1   ]] <= isovalue { 1 } else { 0 } << 2;
        config_idx |= if self.blocks[[x + 1,    y,      z       ]] <= isovalue { 1 } else { 0 } << 3;
        config_idx |= if self.blocks[[x,        y + 1,  z       ]] <= isovalue { 1 } else { 0 } << 4;
        config_idx |= if self.blocks[[x,        y + 1,  z + 1   ]] <= isovalue { 1 } else { 0 } << 5;
        config_idx |= if self.blocks[[x + 1,    y + 1,  z + 1   ]] <= isovalue { 1 } else { 0 } << 6;
        config_idx |= if self.blocks[[x + 1,    y + 1,  z       ]] <= isovalue { 1 } else { 0 } << 7;

        return TRIANGULATIONS[config_idx as usize];
    }

    fn comute_normal(triangle: &[Vector3<f32>; 3]) -> Vector3<f32> {
        (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]).normalize()
    }
}

impl ChunkMesh {
    pub fn new(vertices: Vec<Vertex>, indices: Option<Vec<u32>>) -> Self {
        Self {
            vertex_array: None,
            indices,
            vertices,
        }
    }

    pub fn buffer_data(&mut self) {
        let mut vertex_array = DynamicVertexArray::new();
        vertex_array.buffer_data_dyn(&self.vertices, &self.indices.clone());
        self.vertex_array = Some(vertex_array);
    }

    pub fn render(&self, shader: &Shader, position: (f32, f32, f32)) {
        unsafe {
            gl::Enable(gl::DEPTH_TEST);
            gl::Enable(gl::CULL_FACE);

            shader.bind();
            let model = cgmath::Matrix4::from_translation(cgmath::Vector3::new(position.0, position.1, position.2));
            shader.set_uniform_mat4("model", &model);

            if let Some(vertex_array) = &self.vertex_array {
                vertex_array.bind();
                if let Some(indices) = &self.indices {
                    gl::DrawElements(gl::TRIANGLES, indices.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
                } else {
                    gl::DrawArrays(gl::TRIANGLES, 0, self.vertices.len() as i32);
                }
            }

            gl::Disable(gl::CULL_FACE);
            gl::Disable(gl::DEPTH_TEST);
        }
    }

    pub fn is_buffered(&self) -> bool {
        self.vertex_array.is_some()
    }
}

impl VertexAttributes for Vertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![
            (3, gl::FLOAT),
            (3, gl::FLOAT),
            (3, gl::FLOAT),
        ]
    }
}