use glfw::{Action, Glfw, Key, CursorMode};

pub struct DebugController {
    wireframe: bool,
    pub show_fps: bool,
    vsync: bool,
    pub show_rays: bool,
}

impl DebugController {
    pub fn new() -> Self {
        Self {
            wireframe: false,
            show_fps: false,
            vsync: true,
            show_rays: false,
        }
    }

    pub fn process_keyboard(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::Key(Key::F1, _, Action::Press, _) => {
                self.wireframe = !self.wireframe;
                unsafe {
                    if self.wireframe {
                        gl::PolygonMode(gl::FRONT_AND_BACK, gl::LINE);
                    } else {
                        gl::PolygonMode(gl::FRONT_AND_BACK, gl::FILL);
                    }
                }
            }
            glfw::WindowEvent::Key(Key::F2, _, Action::Press, _) => {
                self.vsync = !self.vsync;
                if self.vsync {
                    glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
                } else {
                    glfw.set_swap_interval(glfw::SwapInterval::None);
                }
            }
            glfw::WindowEvent::Key(Key::F3, _, Action::Press, _) => {
                self.show_fps = !self.show_fps;
            }
            glfw::WindowEvent::Key(Key::F4, _, Action::Press, _) => {
                self.show_rays = !self.show_rays;
            }
            glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => match window.get_cursor_mode() {
                CursorMode::Disabled => window.set_cursor_mode(CursorMode::Normal),
                CursorMode::Normal => window.set_cursor_mode(CursorMode::Disabled),
                _ => {}
            },
            _ => {}
        }
    }
}