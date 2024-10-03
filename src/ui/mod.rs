pub mod button;
pub mod container;
pub mod input;
pub mod panel;
pub mod text;
pub mod ui;

pub struct UIRenderer {
    children: Vec<Box<dyn UIElement>>,
}

pub trait UIElement {
    fn render(&mut self);
    fn handle_events(&mut self, window: &mut glfw::Window, glfw: &mut glfw::Glfw, event: &glfw::WindowEvent) -> bool;
    fn add_children(&mut self, children: Vec<Box<dyn UIElement>>);
    fn set_offset(&mut self, offset: (f32, f32));
    fn get_size(&self) -> (f32, f32);
}