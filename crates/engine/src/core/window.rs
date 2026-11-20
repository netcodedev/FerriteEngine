use glfw::{Context, GlfwReceiver};
use std::ptr;

pub struct Window {
    window: glfw::PWindow,
    glfw: glfw::Glfw,
    events: GlfwReceiver<(f64, glfw::WindowEvent)>,
    pub width: u32,
    pub height: u32,
    /// Drawn with Vulkan (no GL context; the device presents).
    vulkan: bool,
}

impl Window {
    pub fn new(width: u32, height: u32, title: &str) -> Self {
        Self::with_decorations(width, height, title, true)
    }

    /// `decorated = false` removes the system title bar and borders, for
    /// applications that draw their own (like the editor).
    pub fn with_decorations(width: u32, height: u32, title: &str, decorated: bool) -> Self {
        #[allow(unused_mut)]
        let mut glfw = glfw::init(glfw::log_errors).unwrap_or_else(|err| {
            eprintln!("Fehler bei der GLFW-Initialisierung: {}", err);
            std::process::exit(1);
        });

        // Vulkan draws, where the build has it; `FERRITE_RENDERER=gl` (or
        // `opengl`) asks for OpenGL, which also draws when Vulkan cannot.
        let default = if cfg!(feature = "vulkan") {
            crate::core::gfx::BackendKind::Vulkan
        } else {
            crate::core::gfx::BackendKind::OpenGl
        };
        let requested = std::env::var("FERRITE_RENDERER")
            .ok()
            .and_then(|v| crate::core::gfx::BackendKind::parse(&v))
            .unwrap_or(default);
        if requested == crate::core::gfx::BackendKind::Vulkan {
            #[cfg(feature = "vulkan")]
            match Self::with_vulkan(glfw, width, height, title, decorated) {
                Ok(window) => return window,
                Err((g, e)) => {
                    glfw = g;
                    log::error!("Vulkan cannot draw ({e}); OpenGL draws instead");
                }
            }
            #[cfg(not(feature = "vulkan"))]
            log::error!("this build has no Vulkan backend; OpenGL draws instead");
            glfw.default_window_hints();
        }

        glfw.window_hint(glfw::WindowHint::Samples(Some(8)));
        glfw.window_hint(glfw::WindowHint::Decorated(decorated));
        // Shown once the first frame is drawn (see `show`), so it never
        // appears empty while the application is still starting.
        glfw.window_hint(glfw::WindowHint::Visible(false));

        // GL debug output only when asked for: it costs frame time.
        let debug = crate::core::diagnostics::gl_debug_requested();
        glfw.window_hint(glfw::WindowHint::OpenGlDebugContext(debug));
        // A context that tells about a GPU reset (a TDR on Windows) instead
        // of leaving the game to crash in the driver; see `diagnostics`.
        glfw.window_hint(glfw::WindowHint::ContextRobustness(
            glfw::ContextRobustnessHint::LoseContextOnReset,
        ));
        let (mut window, events) = glfw
            .create_window(width, height, title, glfw::WindowMode::Windowed)
            .or_else(|| {
                // Not every driver offers it: the same without.
                log::info!("no robust GL context offered; GPU resets will go unnoticed");
                glfw.window_hint(glfw::WindowHint::ContextRobustness(
                    glfw::ContextRobustnessHint::NoRobustness,
                ));
                glfw.create_window(width, height, title, glfw::WindowMode::Windowed)
            })
            .expect("Fenster konnte nicht erstellt werden");

        window.make_current();
        // Wait for the display (vsync) unless asked otherwise: what the
        // driver would pick on its own differs.
        glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
        window.set_key_polling(true);
        window.set_mouse_button_polling(true);
        window.set_scroll_polling(true);
        window.set_cursor_pos_polling(true);
        window.set_framebuffer_size_polling(true);
        window.set_char_polling(true);
        // Only reported: the window still closes on its own unless a layer
        // clears `should_close` again (the editor asks about unsaved edits).
        window.set_close_polling(true);
        window.set_cursor_mode(glfw::CursorMode::Disabled);
        window.set_cursor_pos(0.0, 0.0);

        gl::load_with(|symbol| {
            window
                .get_proc_address(symbol)
                .map_or(ptr::null(), |f| f as *const _)
        });
        unsafe {
            gl::Enable(gl::MULTISAMPLE);
        }
        crate::core::diagnostics::context_created(&mut glfw, debug);
        crate::core::gfx::init(crate::core::gfx::opengl_device());

        // In pixels, which differ from the screen coordinates asked for on
        // scaled displays; no `FramebufferSize` event tells it at first.
        let (width, height) = match window.get_framebuffer_size() {
            (w, h) if w > 0 && h > 0 => (w as u32, h as u32),
            _ => (width, height),
        };
        crate::core::gfx::set_window_size(width, height);
        Self {
            window,
            glfw,
            events,
            width,
            height,
            vulkan: false,
        }
    }

    /// A window without a GL context, drawn by the Vulkan device.
    #[cfg(feature = "vulkan")]
    fn with_vulkan(
        mut glfw: glfw::Glfw,
        width: u32,
        height: u32,
        title: &str,
        decorated: bool,
    ) -> Result<Self, (glfw::Glfw, String)> {
        match Self::vulkan_window(&mut glfw, width, height, title, decorated) {
            Ok((window, events, width, height)) => Ok(Self { window, glfw, events, width, height, vulkan: true }),
            Err(e) => Err((glfw, e)),
        }
    }

    #[cfg(feature = "vulkan")]
    #[allow(clippy::type_complexity)]
    fn vulkan_window(
        glfw: &mut glfw::Glfw,
        width: u32,
        height: u32,
        title: &str,
        decorated: bool,
    ) -> Result<(glfw::PWindow, GlfwReceiver<(f64, glfw::WindowEvent)>, u32, u32), String> {
        if !glfw.vulkan_supported() {
            return Err("GLFW finds no Vulkan loader".into());
        }
        glfw.window_hint(glfw::WindowHint::ClientApi(glfw::ClientApiHint::NoApi));
        glfw.window_hint(glfw::WindowHint::Decorated(decorated));
        glfw.window_hint(glfw::WindowHint::Visible(false));
        let (mut window, events) = glfw
            .create_window(width, height, title, glfw::WindowMode::Windowed)
            .ok_or("no window")?;
        window.set_key_polling(true);
        window.set_mouse_button_polling(true);
        window.set_scroll_polling(true);
        window.set_cursor_pos_polling(true);
        window.set_framebuffer_size_polling(true);
        window.set_char_polling(true);
        window.set_close_polling(true);
        window.set_cursor_mode(glfw::CursorMode::Disabled);
        window.set_cursor_pos(0.0, 0.0);
        let (width, height) = match window.get_framebuffer_size() {
            (w, h) if w > 0 && h > 0 => (w as u32, h as u32),
            _ => (width, height),
        };
        crate::core::gfx::set_window_size(width, height);
        let extensions = glfw
            .get_required_instance_extensions()
            .ok_or("the window system offers no Vulkan surface")?;
        let device = crate::core::gfx::vulkan_device(
            &extensions,
            &|instance| {
                let mut surface: glfw::ffi::VkSurfaceKHR = ptr::null_mut();
                let result = unsafe {
                    window.create_window_surface(instance as usize as glfw::ffi::VkInstance, ptr::null(), &mut surface)
                };
                if result == 0 {
                    Ok(surface as usize as u64)
                } else {
                    Err(format!("glfwCreateWindowSurface: {result}"))
                }
            },
            true,
        )?;
        crate::core::gfx::init(device);
        crate::core::diagnostics::vulkan_device_created(glfw, &crate::core::gfx::device().describe());
        Ok((window, events, width, height))
    }

    /// Make the window visible (it is created hidden).
    pub fn show(&mut self) {
        self.window.show();
    }

    pub fn clear(&self, clear_color: (f32, f32, f32, f32), mask: u32) {
        unsafe {
            gl::ClearColor(clear_color.0, clear_color.1, clear_color.2, clear_color.3);
            gl::Clear(mask);
        }
    }

    pub fn clear_mask(&self, mask: u32) {
        unsafe {
            gl::Clear(mask);
        }
    }

    pub fn handle_events<F>(&mut self, mut event_handler: F)
    where
        F: FnMut(&mut glfw::Window, &mut glfw::Glfw, glfw::WindowEvent),
    {
        self.glfw.poll_events();
        for (_, event) in glfw::flush_messages(&self.events) {
            match event {
                glfw::WindowEvent::FramebufferSize(width, height) => {
                    self.width = width as u32;
                    self.height = height as u32;
                    crate::core::gfx::set_window_size(self.width, self.height);
                }
                _ => {}
            }
            event_handler(&mut self.window, &mut self.glfw, event);
        }
    }

    pub fn should_close(&mut self) -> bool {
        self.window.should_close()
    }

    pub fn swap_buffers(&mut self) {
        // Vulkan presents in `gfx::end_frame`.
        if !self.vulkan {
            self.window.swap_buffers();
        }
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

    /// Save the current back buffer (what is about to be presented) as a
    /// PNG. Reads only this window's own framebuffer.
    pub fn save_screenshot(&self, path: &std::path::Path) -> Result<(), image::ImageError> {
        use crate::core::gfx;
        let (w, h) = (self.width.max(1), self.height.max(1));
        let pixels = gfx::read_pixels(gfx::TargetRef::Window, gfx::Rect::sized(w, h));
        let image = image::RgbaImage::from_raw(w, h, pixels).expect("buffer sized for image");
        // Rows start at the bottom.
        image::imageops::flip_vertical(&image).save(path)
    }

    pub fn reset_viewport(&self) {
        crate::core::gfx::set_viewport(crate::core::gfx::Rect::sized(self.width, self.height));
    }
}

impl Drop for Window {
    /// Whatever still owns GL objects past here (thread-locals, statics,
    /// caches in scripts) would call into the driver once the context and
    /// the driver are gone, and crash the process on exit: from here on
    /// every GL call does nothing.
    fn drop(&mut self) {
        crate::core::gfx::shutdown();
        disconnect_gl();
    }
}

/// What every GL function is once the context is gone: it does nothing and
/// returns 0 (no object, no error), whatever its arguments (the caller
/// cleans those up in the C calling conventions).
extern "system" fn gl_gone() -> usize {
    0
}

/// Point every GL function at `gl_gone` (see `Window`'s `Drop`). GL objects
/// dropped after this are not freed: the process is ending, and the driver
/// frees them with the context.
pub fn disconnect_gl() {
    gl::load_with(|_| gl_gone as *const std::ffi::c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gl_calls_after_the_context_is_gone_do_nothing() {
        disconnect_gl();
        unsafe {
            let texture = 7;
            gl::DeleteTextures(1, &texture);
            gl::DeleteBuffers(1, &texture);
            gl::DeleteVertexArrays(1, &texture);
            gl::DeleteFramebuffers(1, &texture);
            gl::DeleteProgram(3);
            // Loops waiting for no error end.
            assert_eq!(gl::GetError(), gl::NO_ERROR);
            assert!(gl::GetString(gl::VERSION).is_null());
        }
        assert!(gl::DeleteTextures::is_loaded());
    }
}

/// Whether frames wait for the display (vsync), for the GL context current on
/// this thread.
///
/// Lives here so crates that link the engine as a shared library can switch
/// it: the dylib exports Rust items but not GLFW's C symbols, so calling
/// `glfw::ffi::glfwSwapInterval` from outside the engine fails to link.
#[inline(never)]
pub fn set_vsync(on: bool) {
    use crate::core::gfx;
    if gfx::backend() == gfx::BackendKind::Vulkan {
        gfx::set_vsync(on);
        return;
    }
    unsafe { glfw::ffi::glfwSwapInterval(if on { 1 } else { 0 }) };
}
