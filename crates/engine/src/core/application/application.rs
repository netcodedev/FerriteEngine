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
        let mut graphics = crate::core::graphics::Watch::default();
        let mut vsync = None;
        while !self.window.should_close() {
            use crate::core::gfx;
            let frame_start = std::time::Instant::now();
            // Graphics settings the window carries out.
            if let Some(settings) = graphics.changed() {
                if vsync != Some(settings.engine.vsync) {
                    vsync = Some(settings.engine.vsync);
                    crate::core::window::set_vsync(settings.engine.vsync);
                }
            }
            gfx::begin_frame();
            let (r, g, b, a) = self.clear_color;
            gfx::begin_window_pass(gfx::Load::Color([r, g, b, a]), gfx::Load::Depth(1.0));

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
            gfx::end_pass();
            gfx::collect_garbage();
            gfx::end_frame();

            self.window.swap_buffers();
            // A frame rate cap from the graphics settings: wait out the
            // rest of the frame's share of a second.
            let cap = crate::core::graphics::engine().fps_cap;
            if cap > 0.0 {
                let share = std::time::Duration::from_secs_f64(1.0 / cap as f64);
                if let Some(rest) = share.checked_sub(frame_start.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
            crate::core::spike_probe::end_frame(frame_start.elapsed());
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
        crate::core::gfx::collect_garbage();
    }
}
