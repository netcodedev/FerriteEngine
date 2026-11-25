use crate::core::{
    primitives::{Position, Region, Size},
    renderer::shader::Shader,
};

pub mod plane;

/// Draws planes from one buffer that every draw streams its planes into:
/// a plane holds no GL objects of its own, and planes drawn together with
/// `render_batch` take one draw call.
pub struct PlaneRenderer {
    shader: Shader,
    width: f32,
    height: f32,
    /// The size the shader's projection was last set for.
    projection_for: Option<(f32, f32)>,
    /// The stream vertex buffer and the quads' index buffer; created on
    /// the first draw.
    buffers: Option<(crate::core::gfx::Buffer, crate::core::gfx::Buffer)>,
    /// Planes written to the stream buffer since it was last orphaned.
    written: usize,
    scratch: Vec<PlaneVertex>,
    /// The backdrop texture is bound (to be unbound after blurred planes).
    backdrop_bound: bool,
}

pub struct Plane {
    region: Region,
    color: (f32, f32, f32, f32),
    pub border_thickness: f32,
    pub border_color: (f32, f32, f32, f32),
    pub border_radius: (f32, f32, f32, f32),
    /// Shows the view behind it blurred (frosted glass), tinted by its
    /// colour: the colour's alpha is how much of the tint covers the
    /// blur. See `hdr::backdrop`.
    pub blur: bool,
}

#[derive(Clone, Copy)]
pub struct PlaneBuilder {
    position: Position,
    size: Size,
    color: (f32, f32, f32, f32),
    border_thickness: f32,
    border_color: (f32, f32, f32, f32),
    border_radius: (f32, f32, f32, f32),
    blur: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PlaneVertex {
    pub position: (f32, f32, f32),
    pub color: (f32, f32, f32, f32),
    pub dimensions: (f32, f32, f32, f32),
    pub border_color: (f32, f32, f32, f32),
    pub border_radius: (f32, f32, f32, f32),
    pub border_thickness: f32,
}
