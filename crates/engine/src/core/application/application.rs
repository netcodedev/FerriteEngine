use crate::core::{
    renderer::{plane::PlaneRenderer, text::TextRenderer},
    window::Window,
};

use super::{Application, Layer};

impl Application {
    pub fn new(width: u32, height: u32, title: &str) -> Self {
        Self::with_decorations(width, height, title, true)
    }

    /// Like `new`, but without the system title bar when `decorated` is
    /// false.
    pub fn with_decorations(width: u32, height: u32, title: &str, decorated: bool) -> Self {
        env_logger::init();
        let mut window = Window::with_decorations(width, height, title, decorated);

        TextRenderer::resize(width, height);
        PlaneRenderer::resize(width, height);

        window.clear(
            (0.3, 0.3, 0.5, 1.0),
            gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT,
        );
        window.swap_buffers();

        Self {
            window,
            layers: Vec::new(),
            clear_color: (0.3, 0.3, 0.5, 1.0),
        }
    }

    pub fn start(&mut self) {
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

            for layer in &mut self.layers {
                layer.on_update(&self.window, self.window.calculate_frametime());
            }

            self.window.swap_buffers();
        }
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
        crate::terrain::tree_forest::release_gl_resources();
    }
}
