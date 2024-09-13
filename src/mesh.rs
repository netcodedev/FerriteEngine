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
        let generator = Source::perlin(1).scale([0.01; 3]);
        let offset: f64 = 16777216.0;
        let blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, Dim<[usize; 3]>> = Array3::<Option<Block>>::from_shape_fn([CHUNK_SIZE, CHUNK_SIZE, CHUNK_SIZE], |(x,y,z)| {
            let sample_point = (
                (position.0 * CHUNK_SIZE as f32) as f64 + x as f64 + offset,
                (position.1 * CHUNK_SIZE as f32) as f64 + y as f64 + offset,
                (position.2 * CHUNK_SIZE as f32) as f64 + z as f64 + offset,
            );
            let noise_value = generator.sample([sample_point.0, sample_point.1, sample_point.2]);
            let threshold = 0.0; // Adjust the threshold value to control the density of blocks
            if noise_value < threshold {
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
            mesh.render(shader_program, (self.position.0 * CHUNK_SIZE as f32 - self.position.0, self.position.1 * CHUNK_SIZE as f32 - self.position.1, self.position.2 * CHUNK_SIZE as f32 - self.position.2));
        }
    }

    fn calculate_mesh(&self) -> Mesh {
        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut normals: Vec<f32> = Vec::new();

        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if let Some(_) = &self.blocks[[x, y, z]] {
                        // Check if the block is visible
                        if self.is_block_visible(x, y, z) {
                            // Calculate the block's vertices and indices
                            let (block_vertices, block_indices, block_normals) = self.calculate_block_mesh(x, y, z);
                            let base_index = vertices.len() as u32 / 3;

                            // Add the block's vertices and indices to the chunk's mesh
                            vertices.extend(block_vertices);
                            indices.extend(block_indices.iter().map(|index| index + base_index));
                            normals.extend(block_normals);
                        }
                    }
                }
            }
        }
        Mesh::new(vertices, indices, normals)
    }

    fn is_block_visible(&self, x: usize, y: usize, z: usize) -> bool {
        // Check if the block is at the chunk's boundaries
        if x == 0 || x == CHUNK_SIZE - 1 || y == 0 || y == CHUNK_SIZE - 1 || z == 0 || z == CHUNK_SIZE - 1 {
            return true;
        }
    
        // Check if any neighboring block is empty
        let neighbors = [
            (x - 1, y, z),
            (x + 1, y, z),
            (x, y - 1, z),
            (x, y + 1, z),
            (x, y, z - 1),
            (x, y, z + 1),
        ];
        for neighbor in neighbors.iter() {
            let block: Option<&Option<Block>> = self.blocks.get(*neighbor);
            if let Some(block) = block {
                if block.is_none() {
                    return true;
                }
            }
        }
    
        false
    }

    fn calculate_block_mesh(&self, x: usize, y: usize, z: usize) -> (Vec<f32>, Vec<u32>, Vec<f32>) {
        let position = (
            self.position.0 + x as f32,
            self.position.1 + y as f32,
            self.position.2 + z as f32,
        );

        // Calculate the block's vertices and indices based on its position
        let (x, y, z) = (position.0, position.1, position.2);
        let vertices: Vec<f32> = vec![
            // Position
            x      , y      , z      ,
            x + 1.0, y      , z      ,
            x + 1.0, y + 1.0, z      ,
            x      , y + 1.0, z      ,
            x      , y      , z + 1.0,
            x + 1.0, y      , z + 1.0,
            x + 1.0, y + 1.0, z + 1.0,
            x      , y + 1.0, z + 1.0,
        ];

        let indices: Vec<u32> = vec![
            0, 1, 2, 2, 3, 0, // Front face
            4, 5, 6, 6, 7, 4, // Back face
            4, 5, 1, 1, 0, 4, // Bottom face
            7, 6, 2, 2, 3, 7, // Top face
            4, 7, 3, 3, 0, 4, // Right face
            5, 6, 2, 2, 1, 5  // Left face
        ];

        let normals: Vec<f32> = vec![
            -1.0, -1.0, -1.0,
             1.0, -1.0, -1.0,
             1.0,  1.0, -1.0,
            -1.0,  1.0, -1.0,
            -1.0, -1.0,  1.0,
             1.0, -1.0,  1.0,
             1.0,  1.0,  1.0,
            -1.0,  1.0,  1.0,
        ];

        (vertices, indices, normals)
    }

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