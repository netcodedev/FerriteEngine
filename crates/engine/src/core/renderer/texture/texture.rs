use std::path::Path;

use crate::core::gfx::{self, Format, Rect, SamplerDesc, TextureDesc, TextureKind, UploadFormat};

use super::{Shader, Texture, TextureRenderer};

impl Texture {
    pub fn new() -> Self {
        Texture {
            inner: std::cell::RefCell::new(None),
            sampler: std::cell::Cell::new(SamplerDesc::LINEAR_CLAMP),
        }
    }

    /// Wrap a texture made with `gfx`, sampled with `sampler`.
    pub fn from_gfx(texture: gfx::Texture, sampler: SamplerDesc) -> Self {
        Texture {
            inner: std::cell::RefCell::new(Some(texture)),
            sampler: std::cell::Cell::new(sampler),
        }
    }

    /// Another handle on this texture, for many models to use one
    /// texture. It does not free the texture: this one must outlive it.
    pub fn share(&self) -> Texture {
        Texture {
            inner: std::cell::RefCell::new(self.inner.borrow().as_ref().map(gfx::Texture::share)),
            sampler: std::cell::Cell::new(self.sampler.get()),
        }
    }

    /// The GPU texture, once there is one.
    pub fn gfx(&self) -> Option<std::cell::Ref<'_, gfx::Texture>> {
        std::cell::Ref::filter_map(self.inner.borrow(), Option::as_ref).ok()
    }

    pub fn raw(&self) -> gfx::RawId {
        self.inner.borrow().as_ref().map_or(0, gfx::Texture::raw)
    }

    pub fn sampler(&self) -> SamplerDesc {
        self.sampler.get()
    }

    pub fn set_sampler(&self, sampler: SamplerDesc) {
        self.sampler.set(sampler);
    }

    pub fn size(&self) -> (u32, u32) {
        self.inner.borrow().as_ref().map_or((0, 0), |t| (t.width(), t.height()))
    }

    fn replace(&self, texture: gfx::Texture, sampler: SamplerDesc) {
        *self.inner.borrow_mut() = Some(texture);
        self.sampler.set(sampler);
    }

    pub fn set_as_depth_texture(&self, width: u32, height: u32) {
        let mut sampler = SamplerDesc::NEAREST_CLAMP;
        sampler.wrap = [gfx::Wrap::ClampToBorder; 3];
        sampler.border = gfx::BorderColor::OpaqueWhite;
        self.replace(gfx::Texture::new(TextureDesc::d2(Format::Depth32F, width, height)), sampler);
    }

    /// An array of `layers` depth maps (16 bit), one per shadow cascade.
    /// Outside the maps the depth reads as far away, i.e. lit.
    pub fn set_as_depth_array(&mut self, width: u32, height: u32, layers: u32) {
        let desc = TextureDesc {
            kind: TextureKind::D2Array,
            layers,
            ..TextureDesc::d2(Format::Depth16, width, height)
        };
        self.replace(gfx::Texture::new(desc), SamplerDesc::LINEAR_CLAMP.compare(gfx::CompareFunc::LessEqual));
    }

    pub fn set_as_color_texture(&self, width: u32, height: u32) {
        self.replace(gfx::Texture::new(TextureDesc::d2(Format::Rgba8, width, height)), SamplerDesc::LINEAR_CLAMP);
    }

    /// Like `load_from_file`, but reports a file that cannot be read as an
    /// image instead of panicking. Returns the image's size in pixels.
    pub fn try_load_from_file(&self, path: &Path) -> Result<(u32, u32), image::ImageError> {
        let img = image::open(path)?.flipv().to_rgba8();
        let size = (img.width(), img.height());
        self.load_from_data(size.0, size.1, img.into_raw());
        Ok(size)
    }

    pub fn load_from_file(&self, path: &Path) {
        let img = image::open(path).expect("Image not found").flipv().to_rgba8();
        let (width, height) = (img.width(), img.height());
        let mut sampler = SamplerDesc::new(gfx::Filter::Linear, gfx::Wrap::Repeat);
        sampler.mag = gfx::Filter::Nearest;
        self.upload(width, height, &img.into_raw(), Format::Rgba8, false, sampler);
    }

    pub fn load_from_data(&self, width: u32, height: u32, data: Vec<u8>) {
        self.upload(width, height, &data, Format::Rgba8, false, SamplerDesc::LINEAR_REPEAT);
    }

    /// Upload a material map with mipmaps and anisotropic filtering. A colour map is `srgb`, so the
    /// shader samples it in linear space; data maps (normal, roughness, ...)
    /// are stored as they are.
    pub fn load_material_map(&self, width: u32, height: u32, data: Vec<u8>, srgb: bool) {
        let format = if srgb { Format::Srgba8 } else { Format::Rgba8 };
        // Sharp at a glancing angle too, as the terrain's textures.
        let sampler = SamplerDesc::LINEAR_REPEAT.mipmapped().anisotropic(8);
        self.upload(width, height, &data, format, true, sampler);
    }

    fn upload(&self, width: u32, height: u32, data: &[u8], format: Format, mipmaps: bool, sampler: SamplerDesc) {
        let desc = TextureDesc::d2(format, width.max(1), height.max(1)).with_mips(if mipmaps { 0 } else { 1 });
        let texture = gfx::Texture::new(desc);
        texture.write(0, Rect::sized(width, height), 0, 1, UploadFormat::Rgba8, data);
        if mipmaps {
            texture.generate_mips();
        }
        self.replace(texture, sampler);
    }

    /// Load `layers` (RGBA8, all `width` x `height`) as a 2D array texture
    /// with mipmaps and anisotropic filtering. `srgb` as for
    /// `load_material_map`.
    pub fn load_array(&mut self, width: u32, height: u32, layers: &[Vec<u8>], srgb: bool) {
        let format = if srgb { Format::Srgba8 } else { Format::Rgba8 };
        let desc = TextureDesc {
            kind: TextureKind::D2Array,
            layers: layers.len().max(1) as u32,
            mips: 0,
            ..TextureDesc::d2(format, width, height)
        };
        let texture = gfx::Texture::new(desc);
        for (index, layer) in layers.iter().enumerate() {
            texture.write(0, Rect::sized(width, height), index as u32, 1, UploadFormat::Rgba8, layer);
        }
        texture.generate_mips();
        self.replace(texture, SamplerDesc::LINEAR_REPEAT.mipmapped().anisotropic(8));
    }

    /// Bind it to `slot` (a texture unit) with its sampling.
    pub fn bind_to(&self, slot: u32) {
        if let Some(texture) = self.inner.borrow().as_ref() {
            gfx::bind_texture(slot, texture, &self.sampler.get());
        }
    }

    /// Bind it to `slot` sampled as `sampler` (e.g. a shadow map read
    /// without comparison).
    pub fn bind_to_with(&self, slot: u32, sampler: &SamplerDesc) {
        if let Some(texture) = self.inner.borrow().as_ref() {
            gfx::bind_texture(slot, texture, sampler);
        }
    }
}

impl TextureRenderer {
    pub fn new() -> Self {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        // The unit square every quad is drawn from (see `vertex.glsl`).
        let corners: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        let corners = gfx::Buffer::from_slice(gfx::BufferUsage::VERTEX, gfx::BufferMemory::Static, &corners);
        Self { shader, corners }
    }

    pub fn render_depth(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.render_inner(texture, x, y, w, h, true);
    }

    pub fn render_color(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.render_inner(texture, x, y, w, h, false);
    }

    /// Kept for compatibility — defaults to colour mode.
    pub fn render(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32) {
        self.render_inner(texture, x, y, w, h, false);
    }

    fn render_inner(&self, texture: &Texture, x: f32, y: f32, w: f32, h: f32, is_depth: bool) {
        let desc = gfx::PipelineDesc {
            vertex_layouts: crate::core::gfx::Layouts::of(&[gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x2])]),
            depth: gfx::DepthState::OFF,
            blend: Some(gfx::BlendState::ALPHA),
            polygon: gfx::PolygonMode::Solid,
            ..Default::default()
        };
        gfx::set_pipeline(self.shader.pipeline(&desc));
        // A depth map is shown as it is, not compared.
        let mut sampler = texture.sampler();
        sampler.compare = None;
        texture.bind_to_with(0, &sampler);
        // Standard UV layout: (0,0) = bottom-left, (1,1) = top-right.
        self.shader.set_uniform_4f("rect", x, y, w, h);
        self.shader.set_uniform_1i("isDepth", if is_depth { 1 } else { 0 });
        gfx::set_vertex_buffer(0, &self.corners, 0, 8);
        gfx::draw(0, 6);
    }
}
