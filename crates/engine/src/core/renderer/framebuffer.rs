//! Render targets made of textures: a colour and/or depth texture, and the
//! sun's cascaded shadow map, as `gfx` render targets.

use super::texture::Texture;
use crate::core::gfx;

/// A render target of up to one colour and one depth texture.
pub struct FrameBuffer {
    width: u32,
    height: u32,
    depth_texture: Option<Texture>,
    color_texture: Option<Texture>,
    /// Made again when an attachment changes.
    target: Option<gfx::RenderTarget>,
}

impl FrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, depth_texture: None, color_texture: None, target: None }
    }

    fn rebuild(&mut self) {
        let attach = |t: &Option<Texture>| t.as_ref().and_then(|t| t.gfx().map(|g| g.attachment(0, None)));
        let colors: Vec<gfx::Attachment> = attach(&self.color_texture).into_iter().collect();
        self.target = Some(gfx::RenderTarget::new(&colors, attach(&self.depth_texture)));
    }

    pub fn append_depth_texture(&mut self, texture: Texture) {
        self.depth_texture = Some(texture);
        self.rebuild();
    }

    pub fn append_color_texture(&mut self, texture: Texture) {
        self.color_texture = Some(texture);
        self.rebuild();
    }

    /// A pass drawing into it.
    pub fn begin_pass(&self, color: gfx::Load, depth: gfx::Load) {
        if let Some(target) = &self.target {
            gfx::begin_pass(target.reference(), (self.width, self.height), color, depth);
        }
    }

    pub fn reference(&self) -> Option<gfx::TargetRef> {
        self.target.as_ref().map(gfx::RenderTarget::reference)
    }

    pub fn get_depth_texture(&self) -> Option<&Texture> {
        self.depth_texture.as_ref()
    }

    pub fn get_color_texture(&self) -> Option<&Texture> {
        self.color_texture.as_ref()
    }

    pub fn get_id(&self) -> u32 {
        self.target.as_ref().map_or(0, |t| t.raw() as u32)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

pub struct ShadowFrameBuffer(pub FrameBuffer);

impl ShadowFrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let mut fbo = FrameBuffer::new(width, height);
        let texture = Texture::new();
        texture.set_as_depth_texture(width, height);
        fbo.append_depth_texture(texture);

        let color_texture = Texture::new();
        color_texture.set_as_color_texture(width, height);
        fbo.append_color_texture(color_texture);

        Self(fbo)
    }

    pub fn get_depth_texture(&self) -> Option<&Texture> {
        self.0.get_depth_texture()
    }

    pub fn get_color_texture(&self) -> Option<&Texture> {
        self.0.get_color_texture()
    }

    pub fn get_id(&self) -> u32 {
        self.0.get_id()
    }
}

/// The sun's cascaded shadow map: one depth layer per cascade, drawn one at
/// a time.
pub struct CascadedShadowMap {
    size: u32,
    texture: Texture,
    /// A render target per layer.
    layers: Vec<gfx::RenderTarget>,
}

impl CascadedShadowMap {
    pub fn new(size: u32, cascades: u32) -> Self {
        let mut texture = Texture::new();
        texture.set_as_depth_array(size, size, cascades);
        let layers = (0..cascades)
            .map(|i| {
                let depth = texture.gfx().map(|t| t.attachment(0, Some(i)));
                gfx::RenderTarget::new(&[], depth)
            })
            .collect();
        Self { size, texture, layers }
    }

    /// A pass drawing into the depth layer of `cascade`, cleared.
    pub fn begin_cascade(&self, cascade: u32) {
        if let Some(target) = self.layers.get(cascade as usize) {
            gfx::begin_pass(target.reference(), (self.size, self.size), gfx::Load::Keep, gfx::Load::Depth(1.0));
        }
    }

    /// Texels per side of a layer.
    pub fn layer_size(&self) -> u32 {
        self.size
    }

    pub fn texture(&self) -> &Texture {
        &self.texture
    }
}
