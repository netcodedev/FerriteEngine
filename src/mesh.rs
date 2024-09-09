use gl::types::{GLsizei, GLsizeiptr, GLuint, GLvoid};
use cgmath::Matrix;
use libnoise::prelude::*;
use ndarray::{ArrayBase, Dim, Array3};

const CHUNK_SIZE: usize = 100;

pub struct Mesh {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    vao: u32,
    vbo: u32,
    ebo: u32,
}

impl Mesh {
    pub fn new(vertices: Vec<f32>, indices: Vec<u32>) -> Self {
        let mut mesh = Mesh {
            vertices,
            indices,
            vao: 0,
            vbo: 0,
            ebo: 0,
        };
        mesh.init();
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
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (self.vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                self.vertices.as_ptr() as *const GLvoid,
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
            gl::VertexAttribPointer(0, 3, gl::FLOAT, gl::FALSE, 3 * std::mem::size_of::<f32>() as GLsizei, std::ptr::null());
            gl::EnableVertexAttribArray(0);

            // Unbind VBO and VAO (optional, but good practice)
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }
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
    pub position: (u32, u32, u32),
}

impl Block {
    pub fn new(position: (u32, u32, u32)) -> Self {
        /*
        let vertices: Vec<f32> = vec![
            // Position
            -0.5, -0.5, -0.5,
             0.5, -0.5, -0.5,
             0.5,  0.5, -0.5,
            -0.5,  0.5, -0.5,
            -0.5, -0.5,  0.5,
             0.5, -0.5,  0.5,
             0.5,  0.5,  0.5,
            -0.5,  0.5,  0.5,
        ];

        let indices: Vec<u32> = vec![
            0, 1, 2, 2, 3, 0, // Back face
            4, 5, 6, 6, 7, 4, // Front face
            4, 5, 1, 1, 0, 4, // Bottom face
            7, 6, 2, 2, 3, 7, // Top face
            4, 7, 3, 3, 0, 4, // Left face
            5, 6, 2, 2, 1, 5  // Right face
        ];

        let mesh = Mesh::new(vertices, indices);
        */
        Block { position }
    }
}

pub struct Chunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, ndarray::Dim<[usize; 3]>>,
    mesh: Option<Mesh>,
}

impl Chunk {
    pub fn new(position: (f32, f32, f32)) -> Self {
        let generator = Source::perlin(1).scale([0.01; 3]);
        let blocks: ArrayBase<ndarray::OwnedRepr<Option<Block>>, Dim<[usize; 3]>> = Array3::<Option<Block>>::from_shape_fn([CHUNK_SIZE, CHUNK_SIZE, CHUNK_SIZE], |(x,y,z)| {
            let noise_value = generator.sample([x as f64, y as f64, z as f64]);
            let threshold = 0.0; // Adjust the threshold value to control the density of blocks
            if noise_value < threshold {
                return None;
            }
            Some(Block::new(((position.0 as usize + x) as u32, (position.1 as usize + y) as u32, (position.2 as usize + z) as u32)))
        });
        Chunk { position, blocks, mesh: None }
    }

    pub fn render(&mut self, shader_program: GLuint) {
        if self.mesh.is_none() {
            self.mesh = Some(self.calculateMesh());
        }
        if let Some(mesh) = &self.mesh {
            mesh.render(shader_program, self.position);
        }
    }

    pub fn calculateMesh(&self) -> Mesh {
        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if let Some(_) = &self.blocks[[x, y, z]] {
                        // Check if the block is visible
                        if self.isBlockVisible(x, y, z) {
                            // Calculate the block's vertices and indices
                            let (block_vertices, block_indices) = self.calculateBlockMesh(x, y, z);
                            let base_index = vertices.len() as u32 / 3;

                            // Add the block's vertices and indices to the chunk's mesh
                            vertices.extend(block_vertices);
                            indices.extend(block_indices.iter().map(|index| index + base_index));
                        }
                    }
                }
            }
        }

        Mesh::new(vertices, indices)
    }
    fn isBlockVisible(&self, x: usize, y: usize, z: usize) -> bool {
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
            if let Some(block) = self.blocks.get(*neighbor) {
                if block.is_none() {
                    return true;
                }
            }
        }

        false
    }

    fn calculateBlockMesh(&self, x: usize, y: usize, z: usize) -> (Vec<f32>, Vec<u32>) {
        let position = (
            self.position.0 + x as f32,
            self.position.1 + y as f32,
            self.position.2 + z as f32,
        );

        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        // Calculate the block's vertices and indices based on its position
        let (x, y, z) = (position.0, position.1, position.2);
        let vertices: Vec<f32> = vec![
            // Position
            x - 0.5, y - 0.5, z - 0.5,
            x + 0.5, y - 0.5, z - 0.5,
            x + 0.5, y + 0.5, z - 0.5,
            x - 0.5, y + 0.5, z - 0.5,
            x - 0.5, y - 0.5, z + 0.5,
            x + 0.5, y - 0.5, z + 0.5,
            x + 0.5, y + 0.5, z + 0.5,
            x - 0.5, y + 0.5, z + 0.5,
        ];

        let indices: Vec<u32> = vec![
            0, 1, 2, 2, 3, 0, // Back face
            4, 5, 6, 6, 7, 4, // Front face
            4, 5, 1, 1, 0, 4, // Bottom face
            7, 6, 2, 2, 3, 7, // Top face
            4, 7, 3, 3, 0, 4, // Left face
            5, 6, 2, 2, 1, 5  // Right face
        ];

        (vertices, indices)
    }

}