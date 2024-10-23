use crate::line::Line;
use crate::camera::{Camera, Projection};
use crate::shader::{Shader, VertexAttributes};
use crate::terrain::{Chunk, ChunkBounds};
use crate::texture::Texture;

use gl::types::GLuint;
use ndarray::{Array3, ArrayBase, Dim};
use libnoise::prelude::*;

use super::{Block, BlockVertex, VoxelChunk, ChunkMesh, CHUNK_SIZE};

impl Block {
    pub fn new(type_id: u32) -> Self {
        Block { type_id }
    }
}

impl VertexAttributes for BlockVertex {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)> {
        vec![
            (3, gl::FLOAT), // position
            (3, gl::FLOAT), // normal
            (2, gl::FLOAT), // texture_coords
            (1, gl::UNSIGNED_INT)  // block_type
        ]
    }
}

impl VoxelChunk {
    fn calculate_mesh(&self) -> ChunkMesh<BlockVertex> {
        let mut vertices: Vec<BlockVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        // Sweep over each axis (X, Y and Z)
        for d in 0..3 {
            let u = (d + 1) % 3;
            let v = (d + 2) % 3;
            let mut x = vec![0i32; 3];
            let mut q = vec![0i32; 3];

            let mut mask = vec![false; CHUNK_SIZE * CHUNK_SIZE];
            let mut flip = vec![false; CHUNK_SIZE * CHUNK_SIZE];
            let mut b_t = vec![0; CHUNK_SIZE * CHUNK_SIZE];
            q[d] = 1;

            // Check each slice of the chunk one at a time
            x[d] = -1;
            while x[d] < CHUNK_SIZE as i32 {
                // Compute the mask
                let mut n = 0;
                x[v] = 0;
                while x[v] < CHUNK_SIZE as i32 {
                    x[u] = 0;
                    while x[u] < CHUNK_SIZE as i32 {
                        let current_block = self.blocks.get(((x[0]) as usize, (x[1]) as usize, (x[2]) as usize));
                        let current_block_type = if let Some(block) = current_block {
                            if block.is_some() {
                                block.as_ref().unwrap().type_id
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        let compare_block = self.blocks.get(((x[0] + q[0]) as usize, (x[1] + q[1]) as usize, (x[2] + q[2]) as usize));
                        let compare_block_type = if let Some(block) = compare_block {
                            if block.is_some() {
                                block.as_ref().unwrap().type_id
                            } else {
                                0
                            }
                        } else {
                            0
                        };
                        let block_type = if current_block_type != 0 {
                            current_block_type
                        } else {
                            compare_block_type
                        };
                        let block_current = if 0 <= x[d] {
                            current_block.unwrap().is_none()
                        } else {
                            true
                        };
                        let block_compare = if x[d] < CHUNK_SIZE as i32 - 1 {
                            compare_block.unwrap().is_none()
                        } else {
                            true
                        };
                        mask[n] = block_current != block_compare;
                        flip[n] = block_compare;
                        b_t[n] = block_type;
                        x[u] += 1;
                        n += 1;
                    }
                    x[v] += 1;
                }

                x[d] += 1;

                n = 0;
                
                // Generate a mesh from the mask using lexicographic ordering,
                // by looping over each block in this slice of the chunk
                for j in 0..CHUNK_SIZE {
                    let mut i = 0;
                    while i < CHUNK_SIZE {
                        if mask[n] {
                            // Compute the width of this quad and store it in w
                            // This is done by searching along the current axis until mask[n + w] is false
                            let mut w = 1;
                            while i + w < CHUNK_SIZE && mask[n + w] && flip[n] == flip[n + w] && b_t[n] == b_t[n + w] {
                                w += 1;
                            }

                            // Compute the height of this quad and store it in h
                            // This is done by checking if every block next to this row (range 0 to w) is also part of the mask.
                            // For example, if w is 5 we currently have a quad of dimensions 1 x 5. To reduce triangle count,
                            // greedy meshing will attempt to expand this quad out to CHUNK_SIZE x 5, but will stop if it reaches a hole in the mask
                            let mut h = 1;
                            'outer: while j + h < CHUNK_SIZE {
                                for k in 0..w {
                                    if !mask[n + k + h * CHUNK_SIZE] || flip[n] != flip[n + k + h * CHUNK_SIZE] || b_t[n] != b_t[n + k + h * CHUNK_SIZE] {
                                        break 'outer;
                                    }
                                }
                                h += 1;
                            }

                            x[u] = i as i32;
                            x[v] = j as i32;

                            // du and dv determine the size and orientation of this face
                            let mut du = vec![0; 3];
                            du[u] = w as i32;

                            let mut dv = vec![0; 3];
                            dv[v] = h as i32;

                            // Create a quad for this face. Colour, normal or textures are not stored in this block vertex format.
                            if !flip[n] {
                                vertices.extend_from_slice(&[
                                    BlockVertex {
                                        position: ((x[0] + du[0]) as f32,(x[1] + du[1]) as f32, (x[2] + du[2]) as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (0.0, 0.0),
                                        block_type: b_t[n],
                                    },
                                    BlockVertex {
                                        position: (x[0] as f32, x[1] as f32, x[2] as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (1.0 * w as f32, 0.0),
                                        block_type: b_t[n],
                                    },
                                    BlockVertex {
                                        position: ((x[0] + du[0] + dv[0]) as f32,  (x[1] + du[1] + dv[1]) as f32,  (x[2] + du[2] + dv[2]) as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (0.0, 1.0 * h as f32),
                                        block_type: b_t[n],
                                    },
                                    BlockVertex {
                                        position: ((x[0] + dv[0]) as f32,  (x[1] + dv[1]) as f32,  (x[2] + dv[2]) as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (1.0 * w as f32, 1.0 * h as f32),
                                        block_type: b_t[n],
                                    }
                                ]);
                            } else {
                                vertices.extend_from_slice(&[
                                    BlockVertex {
                                        position: (x[0] as f32,                    x[1] as f32,                    x[2] as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (0.0, 0.0),
                                        block_type: b_t[n],
                                    },
                                    BlockVertex {
                                        position: ((x[0] + du[0]) as f32,          (x[1] + du[1]) as f32,          (x[2] + du[2]) as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (1.0 * w as f32, 0.0),
                                        block_type: b_t[n],
                                    },
                                    BlockVertex {
                                        position: ((x[0] + dv[0]) as f32,          (x[1] + dv[1]) as f32,          (x[2] + dv[2]) as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (0.0, 1.0 * h as f32),
                                        block_type: b_t[n],
                                    },
                                    BlockVertex {
                                        position: ((x[0] + du[0] + dv[0]) as f32,  (x[1] + du[1] + dv[1]) as f32,  (x[2] + du[2] + dv[2]) as f32),
                                        normal: match d {
                                            0 => (0.0, 1.0, 0.0),
                                            1 => (1.0, 0.0, 0.0),
                                            2 => (0.0, 0.0, 1.0),
                                            _ => (0.0, 0.0, 0.0),
                                        },
                                        texture_coords: (1.0 * w as f32, 1.0 * h as f32),
                                        block_type: b_t[n],
                                    }
                                ]);
                            }

                            let vert_count = vertices.len() as u32;
                            indices.extend_from_slice(&[
                                vert_count - 4, vert_count - 3, vert_count - 2,
                                vert_count - 2, vert_count - 3, vert_count - 1,
                            ]);

                            // Clear this part of the mask, so we don't add duplicate faces
                            for l in 0..h {
                                for k in 0..w {
                                    mask[n + k + l * CHUNK_SIZE] = false;
                                }
                            }

                            // Increment counters and continue
                            i += w;
                            n += w;
                        } else {
                            i += 1;
                            n += 1;
                        }
                    }
                }
            }
        }
        ChunkMesh::new(vertices, Some(indices))
    }
}

impl Chunk for VoxelChunk {
    fn new(position: (f32, f32, f32), _: usize) -> Self {
        let generator = Source::perlin(1).scale([0.003; 2]);
        let hills = Source::perlin(1).scale([0.01; 2]);
        let tiny_hills = Source::perlin(1).scale([0.1; 2]);
        let offset: f64 = 16777216.0;
        let blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, Dim<[usize; 3]>> = Array3::<Option<Block>>::from_shape_fn([CHUNK_SIZE, CHUNK_SIZE, CHUNK_SIZE], |(x,y,z)| {
            let sample_point = (
                (position.0 * CHUNK_SIZE as f32) as f64 + x as f64 + offset,
                (position.2 * CHUNK_SIZE as f32) as f64 + z as f64 + offset,
            );
            let noise_value = (1.0 + generator.sample([sample_point.0, sample_point.1]))/2.0;
            let hills_value = (1.0 + hills.sample([sample_point.0, sample_point.1]))/2.0 * 0.2;
            let tiny_hills_value = (1.0 + tiny_hills.sample([sample_point.0, sample_point.1]))/2.0 * 0.01;
            if ((noise_value + hills_value + tiny_hills_value) * CHUNK_SIZE as f64) < (y as f64) {
                return None;
            }
            Some(Block::new(1))
        });
        let mut chunk = VoxelChunk { position, blocks, mesh: None };
        chunk.mesh = Some(chunk.calculate_mesh());
        chunk
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

    fn render(&mut self, camera: &Camera, projection: &Projection, shader: &Shader) {
        if let Some(mesh) = &mut self.mesh {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            shader.bind();
            shader.set_uniform_mat4("view", &camera.calc_matrix());
            shader.set_uniform_mat4("projection", &projection.calc_matrix());
            mesh.render(&shader, (self.position.0 * CHUNK_SIZE as f32, self.position.1 * CHUNK_SIZE as f32, self.position.2 * CHUNK_SIZE as f32), None);
        }
    }
    
    fn process_line(&mut self, line: &Line, button: &glfw::MouseButton) -> bool {
        // calculate the block that the line intersects with
        let step_size = 0.1;
        let max_distance = line.length;

        let mut modified = false;
        let mut last_position = (0,0,0);
        for i in 0..(max_distance / step_size) as i32 {
            let position = line.position + line.direction * (i as f32 * step_size);
            // check if position is within the bounds of this chunk
            if position.x < self.position.0 * CHUNK_SIZE as f32 || position.x >= (self.position.0 + 1.0) * CHUNK_SIZE as f32 {
                continue;
            }
            if position.y < self.position.1 * CHUNK_SIZE as f32 || position.y >= (self.position.1 + 1.0) * CHUNK_SIZE as f32 {
                continue;
            }
            if position.z < self.position.2 * CHUNK_SIZE as f32 || position.z >= (self.position.2 + 1.0) * CHUNK_SIZE as f32 {
                continue;
            }
            let block_position = (
                (position.x - self.position.0 * CHUNK_SIZE as f32) as usize,
                (position.y - self.position.1 * CHUNK_SIZE as f32) as usize,
                (position.z - self.position.2 * CHUNK_SIZE as f32) as usize,
            );
            if let Some(block) = self.blocks.get(block_position){
                if block.is_some() {
                    if button == &glfw::MouseButton::Button1 {
                        // println!("(Terrain {},{},{}) Block hit at {:?}", self.position.0, self.position.1, self.position.2, block_position);
                        self.blocks[[block_position.0, block_position.1, block_position.2]] = None;
                        self.mesh = Some(self.calculate_mesh());
                        modified = true;
                        break;
                    }
                    if button == &glfw::MouseButton::Button2 {
                        // println!("(Terrain {},{},{}) Block hit at {:?}", self.position.0, self.position.1, self.position.2, block_position);
                        self.blocks[[last_position.0, last_position.1, last_position.2]] = Some(Block::new(2));
                        self.mesh = Some(self.calculate_mesh());
                        modified = true;
                        break;
                    }
                }
            }
            last_position = block_position;
        }
        modified
    }

    fn get_shader_source() -> (String, String) {
        (
            include_str!("vertex.glsl").to_string(),
            include_str!("fragment.glsl").to_string(),
        )
    }
    
    fn get_textures() -> Vec<Texture> {
        let grass_texture = Texture::new(std::path::Path::new("assets/grass.png"));
        let stone_texture = Texture::new(std::path::Path::new("assets/stone.png"));

        vec![grass_texture, stone_texture]
    }
}
