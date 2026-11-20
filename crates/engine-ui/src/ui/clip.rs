//! Nested rectangular clipping via the stencil buffer.
//!
//! Each `push` increments the stencil value inside the given plane, so the
//! active clip region is the intersection of every pushed plane. `pop` must
//! be called with the same plane to undo it. `UI::render` resets the stack
//! once per frame. Draws meanwhile pass only where the stencil holds the
//! current depth (`gfx::StencilMode::Test`).

use std::sync::atomic::{AtomicI32, Ordering};

use ferrite::core::{
    gfx::{self, PassOverrides, Planes, StencilMode, StencilOp},
    renderer::plane::{Plane, PlaneRenderer},
};

static DEPTH: AtomicI32 = AtomicI32::new(0);

pub fn reset() {
    DEPTH.store(0, Ordering::Relaxed);
    gfx::device().clear(Planes { color: false, depth: false, stencil: true }, [0.0; 4], 1.0, 0);
    set_mode(StencilMode::Off);
}

pub fn push(plane: &Plane) {
    let depth = DEPTH.load(Ordering::Relaxed);
    write_stencil(plane, depth, StencilOp::IncrementClamp);
    set_mode(StencilMode::Test((depth + 1) as u8));
    DEPTH.store(depth + 1, Ordering::Relaxed);
}

pub fn pop(plane: &Plane) {
    let depth = DEPTH.load(Ordering::Relaxed);
    if depth <= 0 {
        return;
    }
    write_stencil(plane, depth, StencilOp::DecrementClamp);
    set_mode(if depth - 1 == 0 { StencilMode::Off } else { StencilMode::Test((depth - 1) as u8) });
    DEPTH.store(depth - 1, Ordering::Relaxed);
}

fn set_mode(stencil: StencilMode) {
    gfx::set_pass_overrides(PassOverrides { stencil, ..gfx::pass_overrides() });
}

/// Draw `plane` into the stencil buffer only (no colour, no depth), changing
/// pixels whose current value equals `depth` with `op`. The op is applied on
/// depth-fail too, because `PlaneRenderer` always depth-tests and the clip
/// region must not depend on what was drawn before.
fn write_stencil(plane: &Plane, depth: i32, op: StencilOp) {
    let before = gfx::pass_overrides();
    set_mode(StencilMode::Write { reference: depth as u8, op });
    PlaneRenderer::render(plane);
    gfx::set_pass_overrides(before);
}
