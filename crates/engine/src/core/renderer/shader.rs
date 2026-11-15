use cgmath::{Array, Matrix};
use gl::types::*;
use std::{cell::RefCell, collections::HashMap, ffi::CString, ptr};

pub struct Shader {
    pub id: GLuint,
    /// Uniform locations looked up so far, by name: asking the driver (and
    /// building a C string for it) on every set is a hot path, as uniforms
    /// are set per draw.
    locations: RefCell<HashMap<String, GLint>>,
}

pub struct DynamicVertexArray<T> {
    id: GLuint,
    vbo: GLuint,
    ebo: GLuint,
    /// What was buffered: vertices, and indices if any. The data itself
    /// lives on the GPU only.
    vertex_count: usize,
    index_count: Option<usize>,
    vertex_type: std::marker::PhantomData<T>,
}

pub trait VertexAttributes {
    fn get_vertex_attributes() -> Vec<(usize, GLuint)>;
}

impl Shader {
    #[track_caller]
    pub fn new(vertex_source: &str, fragment_source: &str) -> Self {
        Shader {
            id: Shader::create_shader(vertex_source, fragment_source),
            locations: RefCell::new(HashMap::new()),
        }
    }

    pub fn bind(&self) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            gl::UseProgram(self.id);
        }
    }

    /// Location of the uniform `name` (-1 if the program has none, which
    /// GL ignores on set), asked of the driver once per name.
    fn location(&self, name: &str) -> GLint {
        if let Some(&location) = self.locations.borrow().get(name) {
            return location;
        }
        let location = match CString::new(name) {
            Ok(c_name) => unsafe { gl::GetUniformLocation(self.id, c_name.as_ptr()) },
            Err(_) => -1,
        };
        self.locations.borrow_mut().insert(name.to_owned(), location);
        location
    }

    pub fn set_uniform_1fv(&self, name: &str, values: &[f32]) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform1fv(location, values.len() as GLsizei, values.as_ptr());
        }
    }

    pub fn set_uniform_mat4(&self, name: &str, matrix: &cgmath::Matrix4<f32>) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::UniformMatrix4fv(location, 1, gl::FALSE, matrix.as_ptr());
        }
    }

    pub fn set_uniform_mat4_array(&self, name: &str, matrices: &[cgmath::Matrix4<f32>]) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::UniformMatrix4fv(
                location,
                matrices.len() as i32,
                gl::FALSE,
                matrices.as_ptr() as *const f32,
            );
        }
    }

    pub fn set_uniform_1i(&self, name: &str, value: i32) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform1i(location, value);
        }
    }

    pub fn set_uniform_1f(&self, name: &str, value: f32) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform1f(location, value);
        }
    }

    pub fn set_uniform_2i(&self, name: &str, int1: i32, int2: i32) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform2i(location, int1, int2);
        }
    }

    pub fn set_uniform_2f(&self, name: &str, float1: f32, float2: f32) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform2f(location, float1, float2);
        }
    }

    pub fn set_uniform_3f(&self, name: &str, float1: f32, float2: f32, float3: f32) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform3f(location, float1, float2, float3);
        }
    }

    pub fn set_uniform_4f(&self, name: &str, float1: f32, float2: f32, float3: f32, float4: f32) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform4f(location, float1, float2, float3, float4);
        }
    }

    pub fn set_uniform_3fv(&self, name: &str, value: &cgmath::Vector3<f32>) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            let location = self.location(name);
            gl::Uniform3fv(location, 1, value.as_ptr());
        }
    }

    /// Compiles and links a program. On failure the info log is reported
    /// (with the offending source lines) and 0 is returned, so the caller
    /// draws nothing instead of crashing.
    #[track_caller]
    pub fn create_shader(vertex_shader_source: &str, fragment_shader_source: &str) -> GLuint {
        if crate::core::headless::is_headless() {
            return 0;
        }
        let caller = std::panic::Location::caller();
        unsafe {
            let vertex_shader = compile_stage(gl::VERTEX_SHADER, vertex_shader_source, caller);
            let fragment_shader =
                compile_stage(gl::FRAGMENT_SHADER, fragment_shader_source, caller);
            let (Some(vertex_shader), Some(fragment_shader)) = (vertex_shader, fragment_shader)
            else {
                vertex_shader.map(|s| gl::DeleteShader(s));
                fragment_shader.map(|s| gl::DeleteShader(s));
                return 0;
            };

            let shader_program = gl::CreateProgram();
            gl::AttachShader(shader_program, vertex_shader);
            gl::AttachShader(shader_program, fragment_shader);
            gl::LinkProgram(shader_program);
            // The shaders live on in the program; flag them for deletion now.
            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);

            let mut success = gl::FALSE as GLint;
            gl::GetProgramiv(shader_program, gl::LINK_STATUS, &mut success);
            if success != gl::TRUE as GLint {
                let mut len: GLint = 0;
                gl::GetProgramiv(shader_program, gl::INFO_LOG_LENGTH, &mut len);
                let mut buffer = vec![0u8; len.max(1) as usize];
                let mut written: GLsizei = 0;
                gl::GetProgramInfoLog(
                    shader_program,
                    buffer.len() as GLsizei,
                    &mut written,
                    buffer.as_mut_ptr() as *mut GLchar,
                );
                buffer.truncate(written.max(0) as usize);
                report_shader_error(&format!(
                    "shader program (created at {caller}) failed to link:\n{}",
                    String::from_utf8_lossy(&buffer).trim_end()
                ));
                gl::DeleteProgram(shader_program);
                return 0;
            }

            shader_program
        }
    }
}

/// Compiles one stage; on failure reports the log and returns `None`.
unsafe fn compile_stage(
    stage: GLenum,
    source: &str,
    caller: &std::panic::Location<'_>,
) -> Option<GLuint> {
    let stage_name = match stage {
        gl::VERTEX_SHADER => "vertex",
        gl::FRAGMENT_SHADER => "fragment",
        _ => "unknown",
    };
    let c_source = match CString::new(source.as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            report_shader_error(&format!(
                "{stage_name} shader (created at {caller}) contains a NUL byte"
            ));
            return None;
        }
    };
    let shader = gl::CreateShader(stage);
    gl::ShaderSource(shader, 1, &c_source.as_ptr(), ptr::null());
    gl::CompileShader(shader);

    let mut success = gl::FALSE as GLint;
    gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
    if success == gl::TRUE as GLint {
        return Some(shader);
    }
    let mut len: GLint = 0;
    gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
    let mut buffer = vec![0u8; len.max(1) as usize];
    let mut written: GLsizei = 0;
    gl::GetShaderInfoLog(
        shader,
        buffer.len() as GLsizei,
        &mut written,
        buffer.as_mut_ptr() as *mut GLchar,
    );
    buffer.truncate(written.max(0) as usize);
    gl::DeleteShader(shader);
    let log = String::from_utf8_lossy(&buffer);
    report_shader_error(&format!(
        "{stage_name} shader (created at {caller}) failed to compile:\n{}",
        annotate_info_log(&log, source)
    ));
    None
}

fn report_shader_error(message: &str) {
    // Many objects compile the same source (one shader per model), so report
    // each distinct error only once.
    static REPORTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut reported) = REPORTED.lock() {
        if reported.iter().any(|m| m == message) {
            return;
        }
        reported.push(message.to_owned());
    }
    // Use the engine's logger when one is installed, otherwise stderr, so the
    // error is never swallowed.
    if log::log_enabled!(log::Level::Error) {
        log::error!("{message}");
    } else {
        eprintln!("[shader error] {message}");
    }
}

/// Appends the source line each info-log message refers to.
fn annotate_info_log(log: &str, source: &str) -> String {
    let source_lines: Vec<&str> = source.lines().collect();
    let mut out = String::new();
    let mut last_shown = None;
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        out.push_str("  ");
        out.push_str(line.trim_end());
        out.push('\n');
        let number = info_log_line_number(line);
        if number == last_shown {
            continue;
        }
        last_shown = number;
        if let Some(n) = number {
            if let Some(text) = n.checked_sub(1).and_then(|i| source_lines.get(i)) {
                out.push_str(&format!("    {n:>4} | {}\n", text.trim_end()));
            }
        }
    }
    if out.is_empty() {
        out.push_str("  (empty info log)\n");
    }
    out.truncate(out.trim_end().len());
    out
}

/// Extracts the source line number from a driver info-log line. Handles
/// Mesa (`0:12(5): error: ...`), NVIDIA (`0(12) : error C1234: ...`) and
/// AMD/Intel-style (`ERROR: 0:12: ...`) formats.
fn info_log_line_number(line: &str) -> Option<usize> {
    let line = line.trim_start();
    let line = line
        .strip_prefix("ERROR:")
        .or_else(|| line.strip_prefix("WARNING:"))
        .unwrap_or(line)
        .trim_start();
    // Source-string index first.
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.strip_prefix(':').or_else(|| rest.strip_prefix('('))?;
    let n = rest.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return None;
    }
    rest[..n].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_driver_log_line_numbers() {
        assert_eq!(info_log_line_number("0:12(5): error: `foo' undeclared"), Some(12));
        assert_eq!(info_log_line_number("0(7) : error C1008: undefined variable"), Some(7));
        assert_eq!(info_log_line_number("ERROR: 0:3: 'x' : undeclared identifier"), Some(3));
        assert_eq!(info_log_line_number("error: linking failed"), None);
    }

    #[test]
    fn annotates_log_with_source() {
        let source = "#version 330 core\nvoid main() {\n  foo = 1;\n}";
        let out = annotate_info_log("0:3(3): error: `foo' undeclared\n", source);
        assert!(out.contains("0:3(3): error"));
        assert!(out.contains("3 |   foo = 1;"));
    }
}

impl<T: VertexAttributes + Clone> DynamicVertexArray<T> {
    pub fn new() -> Self {
        let mut vao = 0;
        let mut vbo = 0;
        let mut ebo = 0;
        if !crate::core::headless::is_headless() {
            unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);
            }
        }
        DynamicVertexArray {
            id: vao,
            vbo,
            ebo,
            vertex_count: 0,
            index_count: None,
            vertex_type: std::marker::PhantomData,
        }
    }

    pub fn buffer_data(&mut self, data: &Vec<T>, indices: &Option<Vec<u32>>) {
        self.vertex_count = data.len();
        self.index_count = indices.as_ref().map(Vec::len);
        if crate::core::headless::is_headless() {
            return;
        }
        self.bind();
        unsafe {
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            let mut current_attrib = 0;
            let mut offset = 0;
            for (size, gl_type) in T::get_vertex_attributes() {
                gl::EnableVertexAttribArray(current_attrib);
                match gl_type {
                    gl::FLOAT => {
                        gl::VertexAttribPointer(
                            current_attrib,
                            size as i32,
                            gl::FLOAT,
                            gl::FALSE,
                            std::mem::size_of::<T>() as i32,
                            offset as *const _,
                        );
                        offset += size * std::mem::size_of::<f32>();
                    }
                    gl::UNSIGNED_INT => {
                        gl::VertexAttribIPointer(
                            current_attrib,
                            size as i32,
                            gl::UNSIGNED_INT,
                            std::mem::size_of::<T>() as i32,
                            offset as *const _,
                        );
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
    }
    pub fn get_element_count(&self) -> usize {
        self.index_count.unwrap_or(self.vertex_count)
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
