//! The world's wind, as materials sway models in it (the `Wind` node).

use std::sync::atomic::{AtomicU32, Ordering};

use crate::core::renderer::shader::Shader;

/// Direction (x, z, unit length) and strength. One for now; later the
/// weather's.
pub const WIND: ([f32; 2], f32) = ([0.894, 0.447], 1.0);

/// The time the wind sways models by (seconds, as bits), set once a frame:
/// every pass of a frame must place a swaying model at the very same spot,
/// or the shading pass misses the depth prepass' leaves (`LEQUAL`) and the
/// ambient occlusion draws them as dark shapes.
static CLOCK: AtomicU32 = AtomicU32::new(0);

/// Set the wind's time for this frame (once a frame, by the scene).
pub fn set_clock(seconds: f32) {
    CLOCK.store(seconds.to_bits(), Ordering::Relaxed);
}

pub fn clock() -> f32 {
    f32::from_bits(CLOCK.load(Ordering::Relaxed))
}

/// Set the wind's uniforms on `shader`: the wind itself and how much this
/// draw's models sway (`scale`).
pub fn apply(shader: &Shader, scale: f32) {
    let (direction, strength) = WIND;
    shader.set_uniform_4f("wind", direction[0], direction[1], strength, clock());
    shader.set_uniform_1f("windScale", scale);
}
