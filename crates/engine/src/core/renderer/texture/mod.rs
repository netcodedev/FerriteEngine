use std::cell::{Cell, RefCell};

use crate::core::{gfx, renderer::shader::Shader};

pub mod texture;

/// A texture loaded or allocated after it was made (`Texture::new`, then
/// one of the `load_*`/`set_as_*` calls), with the sampling it was made
/// for. A thin owner of a `gfx::Texture`.
pub struct Texture {
    inner: RefCell<Option<gfx::Texture>>,
    sampler: Cell<gfx::SamplerDesc>,
}

pub struct TextureRenderer {
    shader: Shader,
    /// The unit square quads are drawn from.
    corners: gfx::Buffer,
}
