use glfw::{Context, GlfwReceiver};

pub struct Window {
    window: glfw::PWindow,
    glfw: glfw::Glfw,
    events: GlfwReceiver<(f64, glfw::WindowEvent)>
}

impl Window {
    pub fn new(width: u32, height: u32) -> Self {
        let mut glfw = glfw::init(glfw::log_errors).unwrap_or_else(|err| {
            eprintln!("Fehler bei der GLFW-Initialisierung: {}", err);
            std::process::exit(1);
        });

        let (mut window, events) = glfw.create_window(width, height, "Voxel engine", glfw::WindowMode::Windowed)
            .expect("Fenster konnte nicht erstellt werden");

        window.make_current();
        window.set_key_polling(true);
        window.set_mouse_button_polling(true);
        window.set_cursor_pos_polling(true);
        window.set_framebuffer_size_polling(true);
        // window.set_cursor_mode(glfw::CursorMode::Disabled);
        window.set_cursor_pos(0.0, 0.0);

        gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

        Self {
            window,
            glfw,
            events
        }
    }

    pub fn handle_events<F>(&mut self, mut event_handler: F)
    where
        F: FnMut(&mut glfw::Window, &mut glfw::Glfw, glfw::WindowEvent)
    {
        self.glfw.poll_events();
        for (_, event) in glfw::flush_messages(&self.events) {
            event_handler(&mut self.window, &mut self.glfw, event);
        }
    }

    pub fn should_close(&mut self) -> bool {
        self.window.should_close()
    }

    pub fn swap_buffers(&mut self) {
        self.window.swap_buffers();
    }

    pub fn calculate_frametime(&self) -> f64 {
        static mut LAST_FRAME_TIME: f64 = 0.0;
        let current_time = self.glfw.get_time();
        let delta_time;
        unsafe {
            delta_time = current_time - LAST_FRAME_TIME;
            LAST_FRAME_TIME = current_time;
        }
        delta_time
    }
}