use std::path::PathBuf;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::texture::{Texture, TextureRenderer},
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Textured image. The underlying texture is loaded from disk on construction.
pub struct Image {
    handle: UIElementHandle,
    region: Region,
    texture: Texture,
    renderer: TextureRenderer,
}

impl Image {
    pub fn new(position: Position, size: Size, path: PathBuf) -> Self {
        let texture = Texture::new();
        texture.load_from_file(&path);
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            texture,
            renderer: TextureRenderer::new(),
        }
    }
}

impl UIElement for Image {
    fn update(&mut self, _scene: &mut Scene) {}

    fn render(&self) {
        let p = &self.region.position + &self.region.offset;
        self.renderer
            .render_color(&self.texture, p.x, p.y, self.region.size.width, self.region.size.height);
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        _window: &mut Window,
        _glfw: &mut Glfw,
        _event: &WindowEvent,
    ) -> bool {
        false
    }

    leaf_children_methods!();

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
    }
}
