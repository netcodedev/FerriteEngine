//! Nested rectangular clipping via the stencil buffer.
//!
//! Each `push` increments the stencil value inside the given plane, so the
//! active clip region is the intersection of every pushed plane. `pop` must
//! be called with the same plane to undo it. `UI::render` resets the stack
//! once per frame.

use std::sync::atomic::{AtomicI32, Ordering};

use ferrite::core::renderer::plane::{Plane, PlaneRenderer};

static DEPTH: AtomicI32 = AtomicI32::new(0);

pub fn reset() {
    DEPTH.store(0, Ordering::Relaxed);
    unsafe {
        gl::StencilMask(0xFF);
        gl::ClearStencil(0);
        gl::Clear(gl::STENCIL_BUFFER_BIT);
        gl::Disable(gl::STENCIL_TEST);
    }
}

pub fn push(plane: &Plane) {
    let depth = DEPTH.load(Ordering::Relaxed);
    unsafe {
        gl::Enable(gl::STENCIL_TEST);
        write_stencil(plane, depth, gl::INCR);
        gl::StencilFunc(gl::EQUAL, depth + 1, 0xFF);
    }
    DEPTH.store(depth + 1, Ordering::Relaxed);
}

pub fn pop(plane: &Plane) {
    let depth = DEPTH.load(Ordering::Relaxed);
    if depth <= 0 {
        return;
    }
    unsafe {
        write_stencil(plane, depth, gl::DECR);
        if depth - 1 == 0 {
            gl::Disable(gl::STENCIL_TEST);
        } else {
            gl::StencilFunc(gl::EQUAL, depth - 1, 0xFF);
        }
    }
    DEPTH.store(depth - 1, Ordering::Relaxed);
}

/// Draw `plane` into the stencil buffer only (no colour, no depth), changing
/// pixels whose current value equals `depth` with `op`. The op is applied on
/// depth-fail too, because `PlaneRenderer` always depth-tests and the clip
/// region must not depend on what was drawn before.
unsafe fn write_stencil(plane: &Plane, depth: i32, op: gl::types::GLenum) {
    gl::ColorMask(gl::FALSE, gl::FALSE, gl::FALSE, gl::FALSE);
    gl::DepthMask(gl::FALSE);
    gl::StencilMask(0xFF);
    gl::StencilFunc(gl::EQUAL, depth, 0xFF);
    gl::StencilOp(gl::KEEP, op, op);
    PlaneRenderer::render(plane);
    gl::StencilOp(gl::KEEP, gl::KEEP, gl::KEEP);
    gl::StencilMask(0x00);
    gl::ColorMask(gl::TRUE, gl::TRUE, gl::TRUE, gl::TRUE);
    gl::DepthMask(gl::TRUE);
}
