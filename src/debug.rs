use glfw::{Action, Glfw, Key};

pub struct DebugController {
    wireframe: bool,
    pub show_fps: bool,
    vsync: bool,
}

impl DebugController {
    pub fn new() -> Self {
        Self {
            wireframe: false,
            show_fps: false,
            vsync: true,
        }
    }

    pub fn process_keyboard(&mut self, glfw: &mut Glfw, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::Key(Key::F, _, Action::Press, _) => {
                self.wireframe = !self.wireframe;
                unsafe {
                    if self.wireframe {
                        gl::PolygonMode(gl::FRONT_AND_BACK, gl::LINE);
                    } else {
                        gl::PolygonMode(gl::FRONT_AND_BACK, gl::FILL);
                    }
                }
            }
            glfw::WindowEvent::Key(Key::V, _, Action::Press, _) => {
                self.vsync = !self.vsync;
                if self.vsync {
                    glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
                } else {
                    glfw.set_swap_interval(glfw::SwapInterval::None);
                }
            }
            glfw::WindowEvent::Key(Key::P, _, Action::Press, _) => {
                self.show_fps = !self.show_fps;
            }
            _ => {}
        }
    }
}