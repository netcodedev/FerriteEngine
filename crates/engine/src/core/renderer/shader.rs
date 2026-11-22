
use crate::core::gfx::{self, PipelineDesc, StepMode, VertexFormat, VertexLayout};

/// A linked program, with uniforms set by name. Setting uniforms by name
/// is OpenGL only (it goes away when the shaders move to uniform blocks,
/// see docs/render-backends.md); the program itself is a `gfx::Program`.
pub struct Shader {
    program: gfx::Program,
}

/// Vertices and (optionally) indices on the GPU, drawn with one layout.
pub struct DynamicVertexArray<T> {
    vertices: Option<gfx::Buffer>,
    indices: Option<gfx::Buffer>,
    /// What was buffered: vertices, and indices if any. The data itself
    /// lives on the GPU only.
    vertex_count: usize,
    index_count: Option<usize>,
    vertex_type: std::marker::PhantomData<T>,
}

/// The attributes of a vertex type, interleaved in this order at
/// locations 0, 1, 2…
pub trait VertexAttributes {
    fn vertex_formats() -> Vec<VertexFormat>;
}

impl Shader {
    #[track_caller]
    pub fn new(vertex_source: &str, fragment_source: &str) -> Self {
        let label = std::panic::Location::caller().to_string();
        Shader {
            program: gfx::Program::labelled(vertex_source, fragment_source, &label)
                .unwrap_or_else(gfx::Program::empty),
        }
    }

    /// The backend's name for the program (0 if it failed to build).
    pub fn id(&self) -> u32 {
        self.program.raw() as u32
    }

    pub fn program(&self) -> &gfx::Program {
        &self.program
    }

    /// This program with `desc`'s fixed-function state.
    pub fn pipeline(&self, desc: &PipelineDesc) -> gfx::Pipeline {
        self.program.pipeline(desc)
    }

    fn set(&self, name: &str, value: gfx::UniformValue) {
        gfx::device().set_uniform(self.program.raw(), name, value);
    }

    pub fn set_uniform_1fv(&self, name: &str, values: &[f32]) {
        self.set(name, gfx::UniformValue::Floats(values));
    }

    pub fn set_uniform_mat4(&self, name: &str, matrix: &cgmath::Matrix4<f32>) {
        let m: &[f32; 16] = matrix.as_ref();
        self.set(name, gfx::UniformValue::Mat4(m));
    }

    pub fn set_uniform_mat4_array(&self, name: &str, matrices: &[cgmath::Matrix4<f32>]) {
        // SAFETY: Matrix4<f32> is 16 f32 in a row (repr(C)).
        let m = unsafe { std::slice::from_raw_parts(matrices.as_ptr() as *const [f32; 16], matrices.len()) };
        self.set(name, gfx::UniformValue::Mat4s(m));
    }

    pub fn set_uniform_1i(&self, name: &str, value: i32) {
        self.set(name, gfx::UniformValue::Int(value));
    }

    pub fn set_uniform_1f(&self, name: &str, value: f32) {
        self.set(name, gfx::UniformValue::Float(value));
    }

    pub fn set_uniform_2i(&self, name: &str, int1: i32, int2: i32) {
        self.set(name, gfx::UniformValue::Int2([int1, int2]));
    }

    pub fn set_uniform_2f(&self, name: &str, float1: f32, float2: f32) {
        self.set(name, gfx::UniformValue::Float2([float1, float2]));
    }

    pub fn set_uniform_3f(&self, name: &str, float1: f32, float2: f32, float3: f32) {
        self.set(name, gfx::UniformValue::Float3([float1, float2, float3]));
    }

    pub fn set_uniform_4f(&self, name: &str, float1: f32, float2: f32, float3: f32, float4: f32) {
        self.set(name, gfx::UniformValue::Float4([float1, float2, float3, float4]));
    }

    pub fn set_uniform_3fv(&self, name: &str, value: &cgmath::Vector3<f32>) {
        self.set(name, gfx::UniformValue::Float3([value.x, value.y, value.z]));
    }
}

/// Appends the source line each info-log message refers to.
pub(crate) fn annotate_info_log(log: &str, source: &str) -> String {
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

/// Kept for callers of the old deferred deletion: buffers now free
/// themselves through `gfx::collect_garbage`.
pub fn delete_released_vertex_arrays() {
    gfx::collect_garbage();
}

impl<T: VertexAttributes + Clone> DynamicVertexArray<T> {
    pub fn new() -> Self {
        DynamicVertexArray {
            vertices: None,
            indices: None,
            vertex_count: 0,
            index_count: None,
            vertex_type: std::marker::PhantomData,
        }
    }

    /// The vertex layout of `T` (interleaved at locations 0, 1, 2…, with
    /// the stride of `T` itself).
    pub fn layout() -> VertexLayout {
        let mut layout = VertexLayout::packed(0, StepMode::Vertex, &T::vertex_formats());
        layout.stride = std::mem::size_of::<T>() as u32;
        layout
    }

    pub fn buffer_data(&mut self, data: &Vec<T>, indices: &Option<Vec<u32>>) {
        self.vertex_count = data.len();
        self.index_count = indices.as_ref().map(Vec::len);
        let usage = gfx::BufferUsage::VERTEX | gfx::BufferUsage::RAY_TRACING_INPUT;
        self.vertices = Some(gfx::Buffer::from_slice(usage, gfx::BufferMemory::Static, data));
        self.indices = indices.as_ref().map(|indices| {
            let usage = gfx::BufferUsage::INDEX | gfx::BufferUsage::RAY_TRACING_INPUT;
            gfx::Buffer::from_slice(usage, gfx::BufferMemory::Static, indices)
        });
    }

    /// Raw ids of the vertex and index buffers, for a draw recorded now
    /// and drawn later in the frame (`taa::motion`).
    pub fn raw_buffers(&self) -> Option<(gfx::RawId, Option<gfx::RawId>)> {
        let vertices = self.vertices.as_ref()?;
        Some((vertices.raw(), self.indices.as_ref().map(gfx::Buffer::raw)))
    }

    pub fn get_element_count(&self) -> usize {
        self.index_count.unwrap_or(self.vertex_count)
    }

    /// Bind the buffers to the current pipeline (vertex slot 0).
    pub fn bind(&self) {
        let Some(vertices) = &self.vertices else {
            return;
        };
        gfx::set_vertex_buffer(0, vertices, 0, std::mem::size_of::<T>() as u32);
        if let Some(indices) = &self.indices {
            gfx::set_index_buffer(indices, gfx::IndexFormat::U32);
        }
    }

    /// Draw it all with the current pipeline.
    pub fn draw(&self) {
        self.draw_range(0, self.get_element_count() as u32);
    }

    /// Draw `count` elements from `first` (indices, or vertices without).
    pub fn draw_range(&self, first: u32, count: u32) {
        if self.vertices.is_none() {
            return;
        }
        self.bind();
        if self.indices.is_some() {
            gfx::draw_indexed(first, count, 0);
        } else {
            gfx::draw(first, count);
        }
    }
}
