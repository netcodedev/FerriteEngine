//! Sets of chunk columns around the camera as small pictures the GPU reads
//! (see `shaders/columns.glsl`): which columns are loaded, so the horizon
//! and the far water step aside exactly there (not by distance, which the
//! chunks lag behind when the camera moves fast), and which carry their
//! models (see `far_scatter`).

use std::collections::HashSet;

use cgmath::Point3;

use crate::core::{
    gfx::{self, Format, Rect, SamplerDesc, TextureDesc, TextureKind, UploadFormat},
    renderer::shader::Shader,
};

use super::{streaming::ColumnKey, CHUNK_SIZE};

/// `columnIn` and `columnLoaded` for a program, after its `#version`.
pub const COLUMNS_GLSL: &str = include_str!("shaders/columns.glsl");

/// Texture unit of the loaded columns' picture (`loadedColumns`).
pub const LOADED_UNIT: u32 = 18;

/// Columns from the camera's to the picture's edge: past where chunks stay
/// loaded.
const REACH: i32 = 12;
const SIDE: i32 = 2 * REACH + 1;
/// Read texel by texel.
const SAMPLER: SamplerDesc = SamplerDesc::new(gfx::Filter::Nearest, gfx::Wrap::Repeat);

/// One set's picture.
pub struct ColumnMap {
    /// Made on the first `set`.
    texture: Option<gfx::Texture>,
    /// The column of the first texel.
    origin: ColumnKey,
    texels: Vec<u8>,
}

impl ColumnMap {
    pub fn new() -> Self {
        Self {
            texture: None,
            origin: (0, 0),
            texels: Vec::new(),
        }
    }

    /// Picture `columns` around the camera at `eye`.
    pub fn set(&mut self, eye: Point3<f32>, columns: &HashSet<ColumnKey>) {
        if crate::core::headless::is_headless() {
            return;
        }
        let size = CHUNK_SIZE as f32;
        let origin = ((eye.x / size).floor() as i32 - REACH, (eye.z / size).floor() as i32 - REACH);
        let mut texels = vec![0u8; (SIDE * SIDE) as usize];
        for &(x, z) in columns {
            let (i, j) = (x - origin.0, z - origin.1);
            if (0..SIDE).contains(&i) && (0..SIDE).contains(&j) {
                texels[(j * SIDE + i) as usize] = 255;
            }
        }
        if self.texture.is_some() && origin == self.origin && texels == self.texels {
            return;
        }
        let side = SIDE as u32;
        let texture = self
            .texture
            .get_or_insert_with(|| gfx::Texture::new(TextureDesc::d2(Format::R8, side, side)));
        texture.write(0, Rect::sized(side, side), 0, 1, UploadFormat::R8, &texels);
        self.origin = origin;
        self.texels = texels;
    }

    /// Bind the picture to texture `unit` for the bound `shader`, as the
    /// sampler `map` with its first column in `origin` (and `columnSize`).
    /// Without a picture yet the sampler reads as an empty set.
    pub fn bind(&self, shader: &Shader, unit: u32, map: &str, origin: &str) {
        shader.set_uniform_1i(map, unit as i32);
        shader.set_uniform_2i(origin, self.origin.0, self.origin.1);
        shader.set_uniform_1f("columnSize", CHUNK_SIZE as f32);
        match &self.texture {
            Some(texture) => gfx::bind_texture(unit, texture, &SAMPLER),
            None => gfx::unbind_texture(unit, TextureKind::D2),
        }
    }

    /// Bind as the loaded columns (`loadedColumns`, see `LOADED_UNIT`).
    pub fn bind_loaded(&self, shader: &Shader) {
        self.bind(shader, LOADED_UNIT, "loadedColumns", "loadedOrigin");
    }

    /// The raw texture (0 without a picture yet) and first column, for
    /// programs bound elsewhere.
    pub fn raw(&self) -> (u32, ColumnKey) {
        (self.texture.as_ref().map_or(0, |t| t.raw() as u32), self.origin)
    }
}
