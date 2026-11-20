use std::sync::atomic::{AtomicU32, Ordering};

use gl::types::{GLenum, GLuint};

use super::texture::Texture;
use crate::core::gfx;

/// `BOUND_DRAW` before anything is known: the next `bound_draw` asks GL.
const UNKNOWN: u32 = u32::MAX;

/// GL name of the draw framebuffer the engine bound last (see `bind`), so
/// passes can tell where they draw without asking the driver: a
/// `glGetIntegerv` waits for a threaded driver to catch up, and the bound
/// target is asked for on every lit draw.
static BOUND_DRAW: AtomicU32 = AtomicU32::new(UNKNOWN);

/// Bind `framebuffer` to `target` (`FRAMEBUFFER`, `DRAW_FRAMEBUFFER` or
/// `READ_FRAMEBUFFER`) and remember the draw one. The engine binds its
/// framebuffers only through this, so `bound_draw` stays right.
pub unsafe fn bind(target: GLenum, framebuffer: GLuint) {
    gl::BindFramebuffer(target, framebuffer);
    if target != gl::READ_FRAMEBUFFER {
        BOUND_DRAW.store(framebuffer, Ordering::Relaxed);
    }
}

/// Delete `framebuffers`. One of them still bound for drawing leaves the
/// default framebuffer bound, as GL does.
pub unsafe fn delete(framebuffers: &[GLuint]) {
    gl::DeleteFramebuffers(framebuffers.len() as gl::types::GLsizei, framebuffers.as_ptr());
    let bound = BOUND_DRAW.load(Ordering::Relaxed);
    if bound != UNKNOWN && bound != 0 && framebuffers.contains(&bound) {
        BOUND_DRAW.store(0, Ordering::Relaxed);
    }
}

/// The framebuffer bound for drawing now. Asks GL only while it is not
/// known (before the engine bound one, or after `forget_bound`).
pub fn bound_draw() -> GLuint {
    let bound = BOUND_DRAW.load(Ordering::Relaxed);
    if bound != UNKNOWN {
        return bound;
    }
    let mut queried = 0;
    unsafe { gl::GetIntegerv(gl::DRAW_FRAMEBUFFER_BINDING, &mut queried) };
    let queried = queried as GLuint;
    BOUND_DRAW.store(queried, Ordering::Relaxed);
    queried
}

/// Code that binds framebuffers past `bind` (another library on the same
/// context) calls this after, so the next `bound_draw` asks GL again.
pub fn forget_bound() {
    BOUND_DRAW.store(UNKNOWN, Ordering::Relaxed);
}

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

    /// Draw into it from now on (OpenGL code not drawing through passes
    /// yet), with the viewport over all of it.
    pub fn bind(&self) {
        unsafe {
            bind(gl::FRAMEBUFFER, self.get_id());
            gl::Viewport(0, 0, self.width as i32, self.height as i32);
        }
    }

    pub fn unbind() {
        unsafe {
            bind(gl::FRAMEBUFFER, 0);
        }
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

    pub fn depth_only(&self) {}

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

    pub fn bind(&self) {
        self.0.bind();
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

    /// Draw into the depth layer of `cascade` from now on.
    pub fn bind_cascade(&self, cascade: u32) {
        if let Some(target) = self.layers.get(cascade as usize) {
            unsafe {
                bind(gl::FRAMEBUFFER, target.raw() as GLuint);
                gl::Viewport(0, 0, self.size as i32, self.size as i32);
            }
        }
    }

    pub fn texture(&self) -> &Texture {
        &self.texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bound_draw_framebuffer_is_kept_track_of() {
        // GL calls do nothing from here on (no context in tests).
        crate::core::window::disconnect_gl();
        unsafe {
            bind(gl::FRAMEBUFFER, 5);
            assert_eq!(bound_draw(), 5);
            // Reading from another one leaves where it draws.
            bind(gl::READ_FRAMEBUFFER, 7);
            assert_eq!(bound_draw(), 5);
            bind(gl::DRAW_FRAMEBUFFER, 9);
            assert_eq!(bound_draw(), 9);
            delete(&[3, 5]);
            assert_eq!(bound_draw(), 9);
            // Deleting the bound one leaves the window's bound, as in GL.
            delete(&[4, 9]);
            assert_eq!(bound_draw(), 0);
            bind(gl::FRAMEBUFFER, 2);
        }
        forget_bound();
        // Unknown: asked from GL (which answers nothing here).
        assert_eq!(bound_draw(), 0);
    }

    /// Framebuffers bound past `bind` would leave `bound_draw` wrong, and
    /// lit draws would shade for the wrong target.
    #[test]
    fn framebuffers_are_bound_only_through_bind() {
        // The impostor bake binds back what it found before it returns;
        // the window's test only deletes with GL disconnected.
        let allowed = ["framebuffer.rs", "impostor.rs", "window.rs", "gl_backend.rs"];
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut stack = vec![crates];
        let mut offenders = Vec::new();
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if path.is_dir() {
                    if name != "target" && !name.starts_with('.') {
                        stack.push(path);
                    }
                } else if name.ends_with(".rs") && !allowed.contains(&name.as_str()) {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    if text.contains("gl::BindFramebuffer(") || text.contains("gl::DeleteFramebuffers(") {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "use renderer::framebuffer::bind/delete in {offenders:?}");
    }
}
