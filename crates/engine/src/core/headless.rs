//! Running without a window or GPU (a dedicated game server): the scene
//! loads and simulates — terrain and its colliders, physics, scripts — but
//! nothing touches OpenGL. Whatever creates GPU resources asks
//! `is_headless` first and skips that work.

use std::sync::atomic::{AtomicBool, Ordering};

static HEADLESS: AtomicBool = AtomicBool::new(false);

/// From now on this process has no GL context. Call before loading
/// anything.
pub fn enable() {
    HEADLESS.store(true, Ordering::Relaxed);
}

pub fn is_headless() -> bool {
    HEADLESS.load(Ordering::Relaxed)
}
