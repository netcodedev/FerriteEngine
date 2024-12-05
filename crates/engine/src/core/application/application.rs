use crate::core::{
    renderer::{plane::PlaneRenderer, text::TextRenderer},
    window::Window,
};

use super::{Application, Layer};

impl Application {
    pub fn new(width: u32, height: u32, title: &str) -> Self {
        env_logger::init();
        let mut window = Window::new(width, height, title);

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
        }
    }

    pub fn start(&mut self) {
        while !self.window.should_close() {
            self.window.clear(
                (0.3, 0.3, 0.5, 1.0),
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

    pub fn add_layer(&mut self, mut layer: Box<dyn Layer>) {
        layer.on_attach();
        self.layers.push(layer);
    }
}
