use std::path::PathBuf;

use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::PlaneRenderer,
        texture::{Texture, TextureRenderer},
    },
    scene::Scene,
};
use glfw::{Glfw, Window, WindowEvent};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// Textured image, positioned and sized in window pixels like every other
/// element. The image is loaded from disk on construction; a file that
/// cannot be read leaves the element empty (see `error`) instead of failing.
pub struct Image {
    handle: UIElementHandle,
    region: Region,
    texture: Texture,
    renderer: TextureRenderer,
    /// Size of the loaded image in pixels; `None` if loading failed.
    natural_size: Option<(u32, u32)>,
    error: Option<String>,
}

impl Image {
    pub fn new(position: Position, size: Size, path: PathBuf) -> Self {
        let texture = Texture::new();
        let (natural_size, error) = match texture.try_load_from_file(&path) {
            Ok(size) => (Some(size), None),
            Err(e) => {
                let message = format!("{}: {e}", path.display());
                log::warn!("image: could not load {message}");
                (None, Some(message))
            }
        };
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            texture,
            renderer: TextureRenderer::new(),
            natural_size,
            error,
        }
    }

    /// Size of the image file in pixels, if it could be loaded.
    pub fn natural_size(&self) -> Option<(u32, u32)> {
        self.natural_size
    }

    /// Why the image could not be loaded, if it could not.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Size the image is drawn at, in pixels.
    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
    }
}

/// A rectangle given in window pixels (origin top left, y down) as the
/// clip-space rectangle `(x, y, width, height)` the texture renderer takes
/// (origin at the centre, y up, `x`/`y` being the lower left corner).
fn to_clip_space(x: f32, y: f32, w: f32, h: f32, screen: (f32, f32)) -> (f32, f32, f32, f32) {
    let (screen_w, screen_h) = (screen.0.max(1.0), screen.1.max(1.0));
    (
        x / screen_w * 2.0 - 1.0,
        1.0 - (y + h) / screen_h * 2.0,
        w / screen_w * 2.0,
        h / screen_h * 2.0,
    )
}

impl UIElement for Image {
    fn update(&mut self, _scene: &mut Scene) {}

    fn render(&self) {
        if self.natural_size.is_none() {
            return;
        }
        let p = &self.region.position + &self.region.offset;
        let size = &self.region.size;
        let (x, y, w, h) = to_clip_space(p.x, p.y, size.width, size.height, PlaneRenderer::get_size());
        self.renderer.render_color(&self.texture, x, y, w, h);
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

#[cfg(test)]
mod tests {
    use super::to_clip_space;

    fn close(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
        [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
            .iter()
            .all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn the_whole_window_is_the_whole_clip_space() {
        let rect = to_clip_space(0.0, 0.0, 1280.0, 720.0, (1280.0, 720.0));
        assert!(close(rect, (-1.0, -1.0, 2.0, 2.0)), "{rect:?}");
    }

    #[test]
    fn pixels_count_from_the_top_left_clip_space_from_the_bottom_left() {
        // The top left quarter of the window.
        let top_left = to_clip_space(0.0, 0.0, 640.0, 360.0, (1280.0, 720.0));
        assert!(close(top_left, (-1.0, 0.0, 1.0, 1.0)), "{top_left:?}");
        // A 128 px square whose top left corner is at (640, 360), the centre.
        let centre = to_clip_space(640.0, 360.0, 128.0, 128.0, (1280.0, 720.0));
        assert!(
            close(centre, (0.0, -128.0 / 360.0, 0.2, 128.0 / 360.0)),
            "{centre:?}"
        );
    }

    #[test]
    fn an_empty_window_does_not_divide_by_zero() {
        let rect = to_clip_space(0.0, 0.0, 10.0, 10.0, (0.0, 0.0));
        assert!(rect.0.is_finite() && rect.1.is_finite() && rect.2.is_finite() && rect.3.is_finite());
    }
}
