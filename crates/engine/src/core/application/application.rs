use crate::core::{
    renderer::{plane::PlaneRenderer, text::TextRenderer},
    window::Window,
};

use super::{Application, Layer};

/// Seconds a frame counts as at most (see `Application::start`).
const MAX_FRAME_TIME: f64 = 0.25;

impl Application {
    pub fn new(width: u32, height: u32, title: &str) -> Self {
        Self::with_decorations(width, height, title, true)
    }

    /// Like `new`, but without the system title bar when `decorated` is
    /// false.
    pub fn with_decorations(width: u32, height: u32, title: &str, decorated: bool) -> Self {
        // stderr shows errors unless RUST_LOG says otherwise; the editor's
        // console keeps more (see `logging`).
        crate::core::logging::init("error");
        // A log file, crash reports and the GPU's details (see
        // `diagnostics`), so a crash on a player's machine leaves a trace.
        crate::core::diagnostics::install(title);
        let window = Window::with_decorations(width, height, title, decorated);

        TextRenderer::resize(window.width, window.height);
        PlaneRenderer::resize(window.width, window.height);

        Self {
            window,
            layers: Vec::new(),
            clear_color: (0.3, 0.3, 0.5, 1.0),
        }
    }

    pub fn start(&mut self) {
        let mut shown = false;
        // The first frame's time starts now, not when the window was made:
        // loading the first scene in between is not a frame.
        self.window.calculate_frametime();
        while !self.window.should_close() {
            self.window.clear(
                self.clear_color,
                gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT,
            );

            self.window.handle_events(|window, glfw, event| {
                PlaneRenderer::resize_from_event(&event);
                TextRenderer::resize_from_event(&event);

                for layer in &mut self.layers {
                    layer.on_event(glfw, window, &event);
                }
            });

            // Once per frame, the same for every layer. A frame that took
            // long (a stall, a scene loaded in an update) counts as at most
            // `MAX_FRAME_TIME`: what moves with time does not jump.
            let delta_time = self.window.calculate_frametime().min(MAX_FRAME_TIME);
            for layer in &mut self.layers {
                layer.on_update(&self.window, delta_time);
            }
            // Vertex arrays dropped this frame (text meshes of a rebuilt
            // UI, unloaded models), whatever the scene holds.
            crate::core::renderer::shader::delete_released_vertex_arrays();

            self.window.swap_buffers();
            crate::core::diagnostics::end_frame();
            // No layer took the game's request to end: it ends.
            if super::take_quit_request() {
                break;
            }
            if !shown {
                // The first frame is drawn: now the window can be seen.
                self.window.show();
                shown = true;
            }
        }
        // A server hears at once that this player left.
        crate::core::net::disconnect();
    }

    /// Colour the window is cleared to every frame.
    pub fn set_clear_color(&mut self, color: (f32, f32, f32, f32)) {
        self.clear_color = color;
    }

    pub fn add_layer(&mut self, mut layer: Box<dyn Layer>) {
        layer.on_attach();
        self.layers.push(layer);
    }
}

impl Drop for Application {
    /// Layers own GL objects (textures, buffers, framebuffers) whose `Drop`
    /// calls into OpenGL. Fields drop in declaration order, which would
    /// destroy the window — and its GL context — first, so tear the layers
    /// down explicitly while the context is still current.
    fn drop(&mut self) {
        for mut layer in self.layers.drain(..) {
            layer.on_detach();
            drop(layer);
        }
        // Thread-local GL caches would otherwise outlive the context too.
        // (Any not freed here — a cache in a script, say — does nothing
        // once the window is dropped: see `Window`'s `Drop`.)
        crate::terrain::scatter::release_gl_resources();
        crate::core::renderer::particles::release_gl_resources();
        crate::core::renderer::run_gl_teardown();
    }
}
