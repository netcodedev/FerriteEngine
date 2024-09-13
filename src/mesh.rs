use gl::types::{GLsizeiptr, GLuint, GLvoid};
use cgmath::Matrix;
use libnoise::prelude::*;
use ndarray::{ArrayBase, Dim, Array3};
use std::sync::mpsc;

const CHUNK_SIZE: usize = 128;

pub struct Mesh {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    normals: Vec<f32>,
    vao: u32,
    vbo: u32,
    ebo: u32,
    pub initialized: bool,
}

impl Mesh {
    pub fn new(vertices: Vec<f32>, indices: Vec<u32>, normals: Vec<f32>) -> Self {
        let mesh = Mesh {
            vertices,
            indices,
            normals,
            vao: 0,
            vbo: 0,
            ebo: 0,
            initialized: false,
        };
        mesh
    }

    fn init(&mut self) {
        unsafe {
            // Generate VAO, VBO, and EBO
            gl::GenVertexArrays(1, &mut self.vao);
            gl::GenBuffers(1, &mut self.vbo);
            gl::GenBuffers(1, &mut self.ebo);

            // Bind VAO
            gl::BindVertexArray(self.vao);

            // Bind and fill VBO
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            let vertex_data: Vec<f32> = self.vertices.iter().cloned().chain(self.normals.iter().cloned()).collect();
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertex_data.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertex_data.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );

            // Bind and fill EBO
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (self.indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                self.indices.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );

            // Set vertex attribute pointers
            gl::VertexAttribPointer(0, 3, gl::FLOAT, gl::FALSE, 0, std::ptr::null());
            gl::EnableVertexAttribArray(0);
            gl::VertexAttribPointer(1, 3, gl::FLOAT, gl::FALSE, 0, (self.vertices.len() * std::mem::size_of::<f32>()) as *const GLvoid);
            gl::EnableVertexAttribArray(1);

            // Unbind VBO and VAO (optional, but good practice)
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }
        self.initialized = true;
    }

    pub fn render(&self, shader_program: GLuint, position: (f32, f32, f32)) {
        unsafe {
            gl::UseProgram(shader_program);
            let model = cgmath::Matrix4::from_translation(cgmath::Vector3::new(position.0, position.1, position.2));
            let model_loc = gl::GetUniformLocation(shader_program, "model\0".as_ptr() as *const i8);
            gl::UniformMatrix4fv(model_loc, 1, gl::FALSE, model.as_ptr());

            gl::BindVertexArray(self.vao);
            gl::DrawElements(gl::TRIANGLES, self.indices.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
        }
    }
}

pub struct Block {
}

impl Block {
    pub fn new() -> Self {
        Block { }
    }
}

pub struct Chunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, ndarray::Dim<[usize; 3]>>,
    pub mesh: Option<Mesh>,
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
            Some(Block::new())
        });
        let mut chunk = Chunk { position, blocks, mesh: None };
        chunk.mesh = Some(chunk.calculate_mesh());
        chunk
    }

    pub fn render(&mut self, shader_program: GLuint) {
        if let Some(mesh) = &mut self.mesh {
            if !mesh.initialized {
                mesh.init();
            }
            mesh.render(shader_program, (self.position.0 * CHUNK_SIZE as f32, self.position.1 * CHUNK_SIZE as f32, self.position.2 * CHUNK_SIZE as f32));
        }
    }

    fn calculate_mesh(&self) -> Mesh {
        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut normals: Vec<f32> = Vec::new();

        // Sweep over each axis (X, Y and Z)
        for d in 0..3 {
            let u = (d + 1) % 3;
            let v = (d + 2) % 3;
            let mut x = vec![0i32; 3];
            let mut q = vec![0i32; 3];

            let mut mask = vec![false; CHUNK_SIZE * CHUNK_SIZE];
            let mut flip = vec![false; CHUNK_SIZE * CHUNK_SIZE];
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
                        let block_current = if 0 <= x[d] {
                            self.blocks.get(((x[0]) as usize, (x[1]) as usize, (x[2]) as usize)).unwrap().is_none()
                        } else {
                            true
                        };
                        let block_compare = if x[d] < CHUNK_SIZE as i32 - 1 {
                            self.blocks.get(((x[0] + q[0]) as usize, (x[1] + q[1]) as usize, (x[2] + q[2]) as usize)).unwrap().is_none()
                        } else {
                            true
                        };
                        mask[n] = block_current != block_compare;
                        flip[n] = block_compare;
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
                            while i + w < CHUNK_SIZE && mask[n + w] && flip[n] == flip[n + w] {
                                w += 1;
                            }

                            // Compute the height of this quad and store it in h
                            // This is done by checking if every block next to this row (range 0 to w) is also part of the mask.
                            // For example, if w is 5 we currently have a quad of dimensions 1 x 5. To reduce triangle count,
                            // greedy meshing will attempt to expand this quad out to CHUNK_SIZE x 5, but will stop if it reaches a hole in the mask
                            let mut h = 1;
                            'outer: while j + h < CHUNK_SIZE {
                                for k in 0..w {
                                    if !mask[n + k + h * CHUNK_SIZE] || flip[n] != flip[n + k + h * CHUNK_SIZE] {
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

        Mesh::new(vertices, indices, normals)
    }

    // fn is_block_visible(&self, x: usize, y: usize, z: usize) -> bool {
    //     // Check if the block is at the chunk's boundaries
    //     if x == 0 || x == CHUNK_SIZE - 1 || y == 0 || y == CHUNK_SIZE - 1 || z == 0 || z == CHUNK_SIZE - 1 {
    //         return true;
    //     }
    
    //     // Check if any neighboring block is empty
    //     let neighbors = [
    //         (x - 1, y, z),
    //         (x + 1, y, z),
    //         (x, y - 1, z),
    //         (x, y + 1, z),
    //         (x, y, z - 1),
    //         (x, y, z + 1),
    //     ];
    //     for neighbor in neighbors.iter() {
    //         let block: Option<&Option<Block>> = self.blocks.get(*neighbor);
    //         if let Some(block) = block {
    //             if block.is_none() {
    //                 return true;
    //             }
    //         }
    //     }
    
    //     false
    // }

    // fn calculate_block_mesh(&self, x: usize, y: usize, z: usize) -> (Vec<f32>, Vec<u32>, Vec<f32>) {
    //     let position = (
    //         self.position.0 + x as f32,
    //         self.position.1 + y as f32,
    //         self.position.2 + z as f32,
    //     );

    //     // Calculate the block's vertices and indices based on its position
    //     let (x, y, z) = (position.0, position.1, position.2);
    //     let vertices: Vec<f32> = vec![
    //         // Position
    //         x      , y      , z      ,
    //         x + 1.0, y      , z      ,
    //         x + 1.0, y + 1.0, z      ,
    //         x      , y + 1.0, z      ,
    //         x      , y      , z + 1.0,
    //         x + 1.0, y      , z + 1.0,
    //         x + 1.0, y + 1.0, z + 1.0,
    //         x      , y + 1.0, z + 1.0,
    //     ];

    //     let indices: Vec<u32> = vec![
    //         0, 2, 1, 2, 0, 3, // Front face
    //         4, 5, 6, 6, 7, 4, // Back face
    //         4, 1, 5, 1, 4, 0, // Bottom face
    //         7, 6, 2, 2, 3, 7, // Top face
    //         4, 7, 3, 3, 0, 4, // Right face
    //         5, 2, 6, 2, 5, 1  // Left face
    //     ];

    //     let normals: Vec<f32> = vec![
    //         -1.0, -1.0, -1.0,
    //          1.0, -1.0, -1.0,
    //          1.0,  1.0, -1.0,
    //         -1.0,  1.0, -1.0,
    //         -1.0, -1.0,  1.0,
    //          1.0, -1.0,  1.0,
    //          1.0,  1.0,  1.0,
    //         -1.0,  1.0,  1.0,
    //     ];

    //     (vertices, indices, normals)
    // }

}

pub fn chunkloader(radius: i32, x_dir: i32, z_dir: i32, tx: mpsc::Sender<Chunk>) {
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