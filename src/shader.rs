use gl::types::*;
use std::{ffi::CString, ptr};
use cgmath::{Array, Matrix};

pub struct Shader {
    pub id: GLuint,
}

pub struct DynamicVertexArray<T> {
    id: GLuint,
    vbo: GLuint,
    ebo: GLuint,
    current_vertex_data: Option<Vec<T>>,
    indices: Option<Vec<u32>>,
}

pub trait VertexAttributes {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)>;
}

impl Shader {
    pub fn new(vertex_source: &str, fragment_source: &str) -> Self {
        Shader { id: Shader::create_shader(vertex_source, fragment_source)}
    }

    pub fn bind(&self) {
        unsafe {
            gl::UseProgram(self.id);
        }
    }

    pub fn set_uniform_mat4(&self, name: &str, matrix: &cgmath::Matrix4<f32>) {
        unsafe {
            let name = CString::new(name).unwrap();
            let location = gl::GetUniformLocation(self.id, name.as_ptr());
            gl::UniformMatrix4fv(location, 1, gl::FALSE, matrix.as_ptr());
        }
    }

    pub fn set_uniform_mat4_array(&self, name: &str, matrices: &Vec<cgmath::Matrix4<f32>>) {
        unsafe {
            let name = CString::new(name).unwrap();
            let location = gl::GetUniformLocation(self.id, name.as_ptr());
            gl::UniformMatrix4fv(location, matrices.len() as i32, gl::FALSE, matrices.as_ptr() as *const f32);
        }
    }

    pub fn set_uniform_1i(&self, name: &str, value: i32) {
        unsafe {
            let name = CString::new(name).unwrap();
            let location = gl::GetUniformLocation(self.id, name.as_ptr());
            gl::Uniform1i(location, value);
        }
    }

    pub fn set_uniform_3f(&self, name: &str, float1: f32, float2: f32, float3: f32) {
        unsafe {
            let name = CString::new(name).unwrap();
            let location = gl::GetUniformLocation(self.id, name.as_ptr());
            gl::Uniform3f(location, float1, float2, float3);
        }
    }

    pub fn set_uniform_3fv(&self, name: &str, value: &cgmath::Vector3<f32>) {
        unsafe {
            let name = CString::new(name).unwrap();
            let location = gl::GetUniformLocation(self.id, name.as_ptr());
            gl::Uniform3fv(location, 1, value.as_ptr());
        }
    }

    pub fn create_shader(vertex_shader_source: &str, fragment_shader_source: &str) -> GLuint {
        unsafe {
            // 1. Compile vertex shader
            let vertex_shader = gl::CreateShader(gl::VERTEX_SHADER);
            let c_str_vert = CString::new(vertex_shader_source.as_bytes()).unwrap();
            gl::ShaderSource(vertex_shader, 1, &c_str_vert.as_ptr(), ptr::null());
            gl::CompileShader(vertex_shader);
    
            // 2. Check for vertex shader compilation errors
            let mut success = gl::FALSE as GLint;
            let mut info_log = Vec::with_capacity(512);
            info_log.set_len(512 - 1); // subtract 1 to skip the trailing null character
            gl::GetShaderiv(vertex_shader, gl::COMPILE_STATUS, &mut success);
            if success != gl::TRUE as GLint {
                gl::GetShaderInfoLog(
                    vertex_shader,
                    512,
                    ptr::null_mut(),
                    info_log.as_mut_ptr() as *mut GLchar,
                );
                println!(
                    "ERROR::SHADER::VERTEX::COMPILATION_FAILED\n{}",
                    String::from_utf8_lossy(&info_log)
                );
            }
    
            // 3. Compile fragment shader
            let fragment_shader = gl::CreateShader(gl::FRAGMENT_SHADER);
            let c_str_frag = CString::new(fragment_shader_source.as_bytes()).unwrap();
            gl::ShaderSource(fragment_shader, 1, &c_str_frag.as_ptr(), ptr::null());
            gl::CompileShader(fragment_shader);
    
            // 4. Check for fragment shader compilation errors
            gl::GetShaderiv(fragment_shader, gl::COMPILE_STATUS, &mut success);
            if success != gl::TRUE as GLint {
                gl::GetShaderInfoLog(
                    fragment_shader,
                    512,
                    ptr::null_mut(),
                    info_log.as_mut_ptr() as *mut GLchar,
                );
                println!(
                    "ERROR::SHADER::FRAGMENT::COMPILATION_FAILED\n{}",
                    String::from_utf8_lossy(&info_log)
                );
            }
    
            // 5. Link shaders
            let shader_program = gl::CreateProgram();
            gl::AttachShader(shader_program, vertex_shader);
            gl::AttachShader(shader_program, fragment_shader);
            gl::LinkProgram(shader_program);
    
            // 6. Check for linking errors
            gl::GetProgramiv(shader_program, gl::LINK_STATUS, &mut success);
            if success != gl::TRUE as GLint {
                gl::GetProgramInfoLog(
                    shader_program,
                    512,
                    ptr::null_mut(),
                    info_log.as_mut_ptr() as *mut GLchar,
                );
                println!(
                    "ERROR::SHADER::PROGRAM::LINKING_FAILED\n{}",
                    String::from_utf8_lossy(&info_log)
                );
            }
    
            // 7. Delete the shaders as they're linked into our program now and no longer necessary
            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);
    
            shader_program
        }
    }
}

impl<T: VertexAttributes + Clone> DynamicVertexArray<T> {
    pub fn new() -> Self {
        let mut vao = 0;
        let mut vbo = 0;
        let mut ebo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);
        }
        DynamicVertexArray {
            id: vao,
            vbo,
            ebo,
            current_vertex_data: None,
            indices: None,
        }
    }

    pub fn buffer_data_dyn(&mut self, data: &Vec<T>, indices: &Option<Vec<u32>>) {
        self.bind();
        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            let mut current_attrib = 0;
            let mut offset = 0;
            for (size, gl_type) in T::get_vertex_attributes() {
                gl::EnableVertexAttribArray(current_attrib);
                match gl_type {
                    gl::FLOAT => {
                        gl::VertexAttribPointer(current_attrib, size as i32, gl::FLOAT, gl::FALSE, std::mem::size_of::<T>() as i32, offset as *const _);
                        offset += size * std::mem::size_of::<f32>();
                    }
                    gl::UNSIGNED_INT => {
                        gl::VertexAttribIPointer(current_attrib, size as i32, gl::UNSIGNED_INT, std::mem::size_of::<T>() as i32, offset as *const _);
                        offset += size * std::mem::size_of::<u32>();
                    }
                    _ => {}
                }
                current_attrib += 1;
            }
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (data.len() * std::mem::size_of::<T>()) as GLsizeiptr,
                data.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );
            if let Some(indices) = indices {
                gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.ebo);
                gl::BufferData(
                    gl::ELEMENT_ARRAY_BUFFER,
                    (indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                    indices.as_ptr() as *const GLvoid,
                    gl::STATIC_DRAW,
                );
            }
            // Unbind VBO and VAO (optional, but good practice)
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindVertexArray(0);
        }
        self.current_vertex_data = Some(data.to_vec());
        self.indices = indices.clone();
    }
    pub fn get_element_count(&self) -> usize {
        if let Some(indices) = &self.indices {
            indices.len()
        } else {
            if let Some(current_vertex_data) = &self.current_vertex_data {
                current_vertex_data.len()
            } else {
                0
            }
        }
    }

    pub fn bind(&self) {
        unsafe {
            gl::BindVertexArray(self.id);
        }
    }

    pub fn unbind() {
        unsafe {
            gl::BindVertexArray(0);
        }
    }
}