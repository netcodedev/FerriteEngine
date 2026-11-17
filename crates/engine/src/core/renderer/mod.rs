pub mod debug_views;
pub mod depth_pass;
pub mod framebuffer;
pub mod hdr;
pub mod light;
pub mod line;
pub mod particles;
pub mod pbr;
pub mod plane;
pub mod profiler;
pub mod shader;
pub mod sky;
pub mod ssao;
pub mod text;
pub mod texture;
pub mod weather;

static GL_TEARDOWN: std::sync::Mutex<Vec<fn()>> = std::sync::Mutex::new(Vec::new());

/// Run `release` when the application closes, on the GL thread while the
/// context is still current: for caches of GL objects (textures, buffers)
/// that live in thread-locals or statics, a script's say, which would
/// otherwise be dropped after the context is gone. Registering the same
/// function again does nothing.
pub fn on_gl_teardown(release: fn()) {
    let mut teardown = GL_TEARDOWN.lock().unwrap();
    if !teardown.iter().any(|&f| f as usize == release as usize) {
        teardown.push(release);
    }
}

/// Run what `on_gl_teardown` registered (see `Application`'s `Drop`).
pub(crate) fn run_gl_teardown() {
    let teardown = std::mem::take(&mut *GL_TEARDOWN.lock().unwrap());
    for release in teardown {
        release();
    }
}
