use crate::line::Line;
use crate::camera::{Camera, Projection};
use crate::shader::Shader;
use crate::texture::Texture;

use std::sync::mpsc;
use std::thread;
use std::collections::HashMap;
use ndarray::{Array3, ArrayBase, Dim};
use libnoise::prelude::*;
use cgmath::EuclideanSpace;
use crate::mesh::Mesh;

use super::{Block, Chunk, ChunkBounds, Terrain, CHUNK_SIZE};

impl Block {
    pub fn new(type_id: u32) -> Self {
        Block { type_id }
    }
}

impl ChunkBounds {
    pub fn parse(position: cgmath::Vector3<f32>) -> Self {
        let chunk_pos = (
            (position.x / CHUNK_SIZE as f32).floor() as i32,
            (position.y / CHUNK_SIZE as f32).floor() as i32,
            (position.z / CHUNK_SIZE as f32).floor() as i32,
        );
        let min = (
            chunk_pos.0 * CHUNK_SIZE as i32,
            chunk_pos.1 * CHUNK_SIZE as i32,
            chunk_pos.2 * CHUNK_SIZE as i32,
        );
        let max = (
            (chunk_pos.0 + 1) * CHUNK_SIZE as i32,
            (chunk_pos.1 + 1) * CHUNK_SIZE as i32,
            (chunk_pos.2 + 1) * CHUNK_SIZE as i32,
        );
        ChunkBounds { min, max }
    }

    pub fn contains(&self, position: cgmath::Point3<f32>) -> bool {
        position.x >= self.min.0 as f32 && position.x < self.max.0 as f32 &&
        position.y >= self.min.1 as f32 && position.y < self.max.1 as f32 &&
        position.z >= self.min.2 as f32 && position.z < self.max.2 as f32
    }

    pub fn get_chunk_bounds_on_line(line: &Line) -> Vec<ChunkBounds> {
        let mut bounds = Vec::new();
        let current_chunk = ChunkBounds::parse(line.position.to_vec());
        let step_size = 0.1;
        for i in 0..(line.length / step_size) as i32 {
            let position = line.position + line.direction * (i as f32 * step_size);
            let chunk = ChunkBounds::parse(position.to_vec());
            if current_chunk.contains(position) {
                continue;
            }
            if !bounds.contains(&chunk) {
                bounds.push(chunk);
            }
        }
        if !bounds.contains(&current_chunk) {
            bounds.push(current_chunk);
        }
        bounds
    }
}

impl Chunk {
    pub fn new(position: (f32, f32, f32)) -> Self {
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
        let mut chunk = Chunk { position, blocks, mesh: None };
        chunk.mesh = Some(chunk.calculate_mesh());
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
            mesh.render(&shader, (self.position.0 * CHUNK_SIZE as f32, self.position.1 * CHUNK_SIZE as f32, self.position.2 * CHUNK_SIZE as f32), None);
        }
    }

    pub fn process_line(&mut self, line: &Line, button: &glfw::MouseButton) -> bool {
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

    fn calculate_mesh(&self) -> Mesh {
        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut normals: Vec<f32> = Vec::new();
        let mut texture_coords: Vec<f32> = Vec::new();
        let mut block_type: Vec<u32> = Vec::new();

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
                                    (x[0] + du[0]) as f32,          (x[1] + du[1]) as f32,          (x[2] + du[2]) as f32,
                                    x[0] as f32,                    x[1] as f32,                    x[2] as f32,
                                    (x[0] + du[0] + dv[0]) as f32,  (x[1] + du[1] + dv[1]) as f32,  (x[2] + du[2] + dv[2]) as f32,
                                    (x[0] + dv[0]) as f32,          (x[1] + dv[1]) as f32,          (x[2] + dv[2]) as f32,
                                ]);
                            } else {
                                vertices.extend_from_slice(&[
                                    x[0] as f32,                    x[1] as f32,                    x[2] as f32,
                                    (x[0] + du[0]) as f32,          (x[1] + du[1]) as f32,          (x[2] + du[2]) as f32,
                                    (x[0] + dv[0]) as f32,          (x[1] + dv[1]) as f32,          (x[2] + dv[2]) as f32,
                                    (x[0] + du[0] + dv[0]) as f32,  (x[1] + du[1] + dv[1]) as f32,  (x[2] + du[2] + dv[2]) as f32,
                                ]);
                            }

                            let vert_count = vertices.len() as u32 / 3;
                            indices.extend_from_slice(&[
                                vert_count - 4, vert_count - 3, vert_count - 2,
                                vert_count - 2, vert_count - 3, vert_count - 1,
                            ]);

                            match d {
                                0 => normals.extend(vec![
                                    0.0, 1.0, 0.0,
                                    0.0, 1.0, 0.0,
                                    0.0, 1.0, 0.0,
                                    0.0, 1.0, 0.0,
                                ]),
                                1 => normals.extend(vec![
                                    1.0, 0.0, 0.0,
                                    1.0, 0.0, 0.0,
                                    1.0, 0.0, 0.0,
                                    1.0, 0.0, 0.0,
                                ]),
                                2 => normals.extend(vec![
                                    0.0, 0.0, 1.0,
                                    0.0, 0.0, 1.0,
                                    0.0, 0.0, 1.0,
                                    0.0, 0.0, 1.0,
                                ]),
                                _ => (),
                            }

                            texture_coords.extend(vec![
                                0.0, 0.0,
                                1.0 * w as f32, 0.0,
                                0.0, 1.0 * h as f32,
                                1.0 * w as f32, 1.0 * h as f32,
                            ]);

                            block_type.extend(vec![
                                b_t[n], b_t[n], b_t[n], b_t[n],
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
        
        Mesh::new(vertices, Some(indices), Some(normals), Some(texture_coords), Some(block_type))
    }
}

impl Terrain {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let origin = Chunk::new((0.0, 0.0, 0.0));
        tx.send(origin).unwrap();

        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));

        let tx1 = tx.clone();
        let tx2 = tx.clone();
        let tx3 = tx.clone();
        let tx4 = tx.clone();
        const RADIUS: i32 = 5;
        let _ = thread::spawn(move || chunkloader(RADIUS,1,1,tx1));
        let _ = thread::spawn(move || chunkloader(RADIUS,-1,1,tx2));
        let _ = thread::spawn(move || chunkloader(RADIUS,1,-1,tx3));
        let _ = thread::spawn(move || chunkloader(RADIUS,-1,-1,tx4));


        let grass_texture = Texture::new(std::path::Path::new("assets/grass.png"));
        let stone_texture = Texture::new(std::path::Path::new("assets/stone.png"));

        Self {
            chunks: HashMap::<ChunkBounds, Chunk>::new(),
            chunk_receiver: rx,
            shader,
            grass_texture,
            stone_texture,
        }
    }

    pub fn update(&mut self) {
        if let Ok(chunk) = self.chunk_receiver.try_recv() {
            self.chunks.insert(chunk.get_bounds(), chunk);
        }
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
            self.grass_texture.bind();
            gl::ActiveTexture(gl::TEXTURE1);
            self.stone_texture.bind();
        }
        for (_, chunk) in &mut self.chunks {
            chunk.render(camera, projection, &self.shader);
        }
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE1);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
    }

    pub fn process_line(&mut self, line: Option<(Line, glfw::MouseButton)>) {
        if let Some((line, button)) = line {
            for chunk_bounds in ChunkBounds::get_chunk_bounds_on_line(&line) {
                if let Some(chunk) = self.chunks.get_mut(&chunk_bounds) {
                    if chunk.process_line(&line, &button) {
                        break;
                    }
                }
            }
        }
    }
}

fn chunkloader(radius: i32, x_dir: i32, z_dir: i32, tx: mpsc::Sender<Chunk>) {
    let mut x: i32 = 1;
    let mut z: i32 = 0;

    loop {
        if x > radius {
            break;
        }
        let new_chunk: Chunk;
        if z_dir > 0 {
            new_chunk = Chunk::new(((x * x_dir) as f32, 0.0, z as f32));
        } else {
            new_chunk = Chunk::new(((z * z_dir) as f32, 0.0, (x * x_dir) as f32));
        }
        
        let result = tx.send(new_chunk);
        if result.is_err() {
            break;
        }

        z = -z;
        if z == -x*z_dir {
            x += 1;
            z = 0;
        } else if z >= 0 {
            z += 1;
        }
    }
}