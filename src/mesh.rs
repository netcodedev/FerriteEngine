use gl::types::{GLsizei, GLsizeiptr, GLuint, GLvoid};
use cgmath::Matrix;

const CHUNK_SIZE: usize = 16;

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

    pub fn render(&self) {
        unsafe {
            gl::BindVertexArray(self.vao);
            gl::DrawElements(gl::TRIANGLES, self.indices.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
        }
    }
}

pub struct Block {
    pub position: (f32, f32, f32),
    pub mesh: Mesh,
}

impl Block {
    pub fn new(position: (f32, f32, f32)) -> Self {
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
        Block { position, mesh }
    }

    pub fn render(&self, shader_program: GLuint) {
        unsafe {
            gl::UseProgram(shader_program);
            let model = cgmath::Matrix4::from_translation(cgmath::Vector3::new(self.position.0, self.position.1, self.position.2));
            let model_loc = gl::GetUniformLocation(shader_program, "model\0".as_ptr() as *const i8);
            gl::UniformMatrix4fv(model_loc, 1, gl::FALSE, model.as_ptr());
        }
        self.mesh.render();
    }
}

pub struct Chunk {
    blocks: Vec<Block>,
}

impl Chunk {
    pub fn new(position: (f32, f32, f32)) -> Self {
        let mut blocks = Vec::new();
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    blocks.push(Block::new((position.0 + x as f32, position.1 + y as f32, position.2 + z as f32)));
                }
            }
        }
        Chunk { blocks }
    }

    pub fn render(&self, shader_program: GLuint) {
        for block in &self.blocks {
            block.render(shader_program);
        }
    }
}