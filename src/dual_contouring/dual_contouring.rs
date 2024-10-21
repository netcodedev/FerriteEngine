use cgmath::{EuclideanSpace, InnerSpace, Point3, Vector3};
use gl::types::GLuint;
use libnoise::prelude::*;
use ndarray::ArrayBase;

use crate::{camera::{Camera, Projection}, shader::{DynamicVertexArray, Shader, VertexAttributes}, terrain::ChunkBounds};

use super::{Chunk, ChunkMesh, Vertex, CHUNK_SIZE, ISO_VALUE};

impl Chunk {
    pub fn new(position: (f32, f32, f32)) -> Self {
        let generator = Source::perlin(1).scale([0.003; 2]);
        let hills = Source::perlin(1).scale([0.01; 2]);
        let tiny_hills = Source::perlin(1).scale([0.1; 2]);
        let cave = Source::perlin(1).scale([0.1; 3]);
        let offset: f64 = 16777216.0;
        let blocks: ArrayBase<ndarray::OwnedRepr<f32>, ndarray::Dim<[usize; 3]>> = ArrayBase::from_shape_fn((CHUNK_SIZE + 1, CHUNK_SIZE + 1, CHUNK_SIZE + 1), |(x, y, z)| {
            let sample_point = (
                (position.0 * CHUNK_SIZE as f32) as f64 + x as f64 + offset,
                (position.1 * CHUNK_SIZE as f32) as f64 + y as f64 + offset,
                (position.2 * CHUNK_SIZE as f32) as f64 + z as f64 + offset,
            );
            
            let noise_value = (1.0 + generator.sample([sample_point.0, sample_point.2]))/2.0;
            let hills_value = (1.0 + hills.sample([sample_point.0, sample_point.2]))/2.0 * 0.2;
            let tiny_hills_value = (1.0 + tiny_hills.sample([sample_point.0, sample_point.2]))/2.0 * 0.01;
            if ((noise_value + hills_value + tiny_hills_value) * CHUNK_SIZE as f64) < y as f64 {
                return ISO_VALUE - 1e-6;
            }
            (1.0 + cave.sample([sample_point.0, sample_point.1, sample_point.2]) as f32) / 2.0
        });
        let mut chunk = Self {
            position,
            blocks,
            mesh: None,
        };
        chunk.mesh = Some(chunk.generate_mesh());
        chunk
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection, shader: &Shader) {
        if let Some(mesh) = &mut self.mesh {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            shader.bind();
            shader.set_uniform_mat4("view", &camera.calc_matrix());
            shader.set_uniform_mat4("projection", &projection.calc_matrix());
            mesh.render(&shader, (self.position.0 * CHUNK_SIZE as f32, self.position.1 * CHUNK_SIZE as f32, self.position.2 * CHUNK_SIZE as f32));
        }
    }

    pub fn get_bounds(&self) -> ChunkBounds {
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

    fn generate_mesh(&self) -> ChunkMesh {
        let mut vertices = Vec::<Vertex>::new();
        let mut indices = Vec::<u32>::new();
        let mut vertex_grid = vec![[[false; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];
        let mut index_grid = vec![[[0; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];
        let mut index: u32 = 0;
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if self.is_surface_voxel((x, y, z)) {
                        let mut corners: [(Point3<f32>, f32); 8] = [(Point3::new(0.0, 0.0, 0.0), ISO_VALUE); 8];
                        for i in 0..8 {
                            let x_add = i & 1;
                            let y_add = (i >> 1) & 1;
                            let z_add = (i >> 2) & 1;
                            let x_n = x + x_add;
                            let y_n = y + y_add;
                            let z_n = z + z_add;
                            corners[i] = (Point3::new(x_add as f32, y_add as f32, z_add as f32), self.blocks[[x_n, y_n, z_n]]);
                        }
                        let position = self.calculate_vertex_position((x, y, z), &corners);
                        let normal = Chunk::calculate_gradient(&corners, position);

                        let vertex = Vertex {
                            position: [position.x, position.y, position.z],
                            normal: [normal.x, normal.y, normal.z],
                            color: [0.0, 0.5, 0.1],
                        };
                        index_grid[x][y][z] = index;
                        vertices.push(vertex);
                        vertex_grid[x][y][z] = true;
                        if x > 0 && vertex_grid[x - 1][y][z] {
                            if y > 0 && vertex_grid[x][y - 1][z] {
                                indices.push(index);
                                indices.push(index_grid[x][y - 1][z] as u32);
                                indices.push(index_grid[x - 1][y][z] as u32);

                                if vertex_grid[x-1][y-1][z] {
                                    indices.push(index_grid[x][y - 1][z] as u32);
                                    indices.push(index_grid[x-1][y-1][z] as u32);
                                    indices.push(index_grid[x - 1][y][z] as u32);
                                }
                            }
                            if z > 0 && vertex_grid[x][y][z - 1] {
                                indices.push(index);
                                indices.push(index_grid[x][y][z - 1] as u32);
                                indices.push(index_grid[x - 1][y][z] as u32);

                                if vertex_grid[x-1][y][z-1] {
                                    indices.push(index_grid[x][y][z - 1] as u32);
                                    indices.push(index_grid[x-1][y][z-1] as u32);
                                    indices.push(index_grid[x - 1][y][z] as u32);
                                }
                            }
                        }
                        if y > 0 && vertex_grid[x][y - 1][z] {
                            if z > 0 && vertex_grid[x][y][z - 1] {
                                indices.push(index);
                                indices.push(index_grid[x][y][z - 1] as u32);
                                indices.push(index_grid[x][y - 1][z] as u32);

                                if vertex_grid[x][y-1][z-1] {
                                    indices.push(index_grid[x][y - 1][z] as u32);
                                    indices.push(index_grid[x][y][z - 1] as u32);
                                    indices.push(index_grid[x][y - 1][z-1] as u32);
                                }
                            }
                        }
                        index += 1;
                    }
                }
            }
        }
        println!("Generated mesh with {} vertices and {} indices", vertices.len(), indices.len());
        ChunkMesh::new(vertices, Some(indices))
    }

    fn calculate_vertex_position(&self, position: (usize, usize, usize), corners: &[(Point3<f32>, f32)]) -> Point3<f32> {
        let mut v_pos = Point3::new(0.0,0.0,0.0);
        let relative_coordinates = Chunk::calculate_relative_coordinates(&corners);
        v_pos.x = position.0 as f32 + relative_coordinates.x;
        v_pos.y = position.1 as f32 + relative_coordinates.y;
        v_pos.z = position.2 as f32 + relative_coordinates.z;

        v_pos
    }

    fn interpolate(p1: (Point3<f32>, f32), p2: (Point3<f32>, f32)) -> Point3<f32> {
        let t = (ISO_VALUE - p1.1) / (p2.1 - p1.1);
        p1.0 + (p2.0 - p1.0) * t
    }
    
    fn find_crossing_edges(vertices: &[(Point3<f32>, f32)]) -> Vec<((Point3<f32>, f32), (Point3<f32>, f32))> {
        let mut crossing_edges = Vec::new();
        for (i, p1) in vertices.iter().enumerate() {
            for p2 in vertices.iter().skip(i + 1) {
                if (p1.1 <= ISO_VALUE && ISO_VALUE <= p2.1) || (p2.1 <= ISO_VALUE && ISO_VALUE <= p1.1) {
                    crossing_edges.push((*p1, *p2));
                }
            }
        }
        crossing_edges
    }
    
    fn calculate_relative_coordinates(vertices: &[(Point3<f32>, f32)]) -> Point3<f32> {
        let crossing_edges = Chunk::find_crossing_edges(vertices);
        let interpolated_points: Vec<Point3<f32>> = crossing_edges.iter().map(|&edge| Chunk::interpolate(edge.0, edge.1)).collect();
    
        // Berechne den Schwerpunkt der interpolierten Punkte
        let center_of_mass = interpolated_points.iter().fold(Vector3::new(0.0, 0.0, 0.0), |acc, &p| acc + p.to_vec()) / (interpolated_points.len() as f32);
    
        Point3::from_vec(center_of_mass)
    }

    fn calculate_corner_gradients(vertices: &[(Point3<f32>, f32)]) -> Vec<(Point3<f32>, Vector3<f32>)> {
        let mut corner_gradients = Vec::new();
        for (i, &(point, value)) in vertices.iter().enumerate() {
            let mut gradient = Vector3::new(0.0, 0.0, 0.0);
            for j in 0..vertices.len() {
                if i != j {
                    let other_point = vertices[j].0;
                    let other_value = vertices[j].1;
                    let direction = other_point - point;
                    let distance = direction.magnitude();
                    if distance > 0.0 {
                        gradient += direction * (other_value - value) / distance.powi(2);
                    }
                }
            }
            corner_gradients.push((point, gradient.normalize()));
        }
        corner_gradients
    }
    
    fn calculate_gradient(vertices: &[(Point3<f32>, f32)], point: Point3<f32>) -> Vector3<f32> {
        let corner_gradients = Chunk::calculate_corner_gradients(vertices);
    
        // Trilineare Interpolation der Gradienten
        let mut gradient = Vector3::new(0.0, 0.0, 0.0);
        for i in 0..8 {
            let c = corner_gradients[i].1;
            let p = corner_gradients[i].0;
            let weight = (1.0 - (point.x - p.x).abs()) * (1.0 - (point.y - p.y).abs()) * (1.0 - (point.z - p.z).abs());
            gradient += c * weight;
        }
    
        gradient.normalize()
    }

    fn is_surface_voxel(&self, position: (usize, usize, usize)) -> bool {
        let mut corners = [0.0; 8];
        for i in 0..8 {
            let x = position.0 + (i & 1);
            let y = position.1 + ((i >> 1) & 1);
            let z = position.2 + ((i >> 2) & 1);
            corners[i] = self.blocks[[x, y, z]];
        }
        let mut cube_index = 0;
        for i in 0..8 {
            if corners[i] < ISO_VALUE {
                cube_index |= 1 << i;
            }
        }
        cube_index != 0 && cube_index != 255
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
            gl::Disable(gl::CULL_FACE);

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