use std::sync::atomic::{AtomicBool, Ordering};

use super::window::Window;

mod application;

static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The game asks to end (an Exit button, say). A player closes its window
/// once the frame is done; the editor stops play mode instead, if it takes
/// the request first (`take_quit_request`).
pub fn request_quit() {
    QUIT_REQUESTED.store(true, Ordering::Relaxed);
}

/// Whether the game asked to end since the last call; clears the request.
pub fn take_quit_request() -> bool {
    QUIT_REQUESTED.swap(false, Ordering::Relaxed)
}

static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The game asks to start over: its startup scene loaded afresh, as when it
/// was started (back to the main menu after a game, say). The player and
/// the editor's play mode do it after the frame.
pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::Relaxed);
}

/// Whether the game asked to start over since the last call; clears the
/// request.
pub fn take_restart_request() -> bool {
    RESTART_REQUESTED.swap(false, Ordering::Relaxed)
}

pub struct Application {
    window: Window,
    layers: Vec<Box<dyn Layer>>,
    clear_color: (f32, f32, f32, f32),
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
