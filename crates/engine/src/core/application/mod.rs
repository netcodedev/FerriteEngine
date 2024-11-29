use super::window::Window;

mod application;

pub struct Application {
    window: Window,
    layers: Vec<Box<dyn Layer>>,
}

pub trait Layer {
    fn on_attach(&mut self) {}
    fn on_detach(&mut self) {}
    fn on_update(&mut self, window: &Window, delta_time: f64);
    fn on_event(
        &mut self,
        glfw: &mut glfw::Glfw,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    );

    fn get_name(&self) -> &str;
}
