use super::window::Window;

mod application;

pub struct Application {
    window: Window,
    layers: Vec<Box<dyn Layer>>,
}

#[allow(dead_code)]
pub trait Layer {
    fn on_attach(&self) {}
    fn on_detach(&self) {}
    fn on_update(&mut self, delta_time: f64);
    fn on_event(
        &mut self,
        glfw: &mut glfw::Glfw,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    );

    fn get_name(&self) -> &str;
}
