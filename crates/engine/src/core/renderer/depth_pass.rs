//! Passes that only fill a depth buffer: the sun's shadow cascades, the
//! lights' shadow faces, the depth prepass (ambient occlusion) and the
//! weather's maps. While one runs, the scene's renderers draw with
//! depth-only programs: the position and, where a colour map's alpha cuts
//! texels out, that test; no shading at all.
//!
//! A depth-only program must put its vertices exactly where the shading
//! program does (the shading pass tests `LEQUAL` against the prepass'
//! depth), so both declare `invariant gl_Position` and compute it with the
//! same expression.

use std::sync::atomic::{AtomicU32, Ordering};

static DEPTH_PASSES: AtomicU32 = AtomicU32::new(0);

/// Whether a depth-only pass is being drawn.
pub fn is_active() -> bool {
    DEPTH_PASSES.load(Ordering::Relaxed) > 0
}

/// Marks a depth-only pass while it lives.
pub struct DepthPass(());

impl DepthPass {
    pub fn begin() -> Self {
        DEPTH_PASSES.fetch_add(1, Ordering::Relaxed);
        DepthPass(())
    }
}

impl Drop for DepthPass {
    fn drop(&mut self) {
        DEPTH_PASSES.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The fragment shader of a depth-only program without an alpha test.
pub const EMPTY_FRAGMENT: &str = "#version 460 core\nvoid main() {}\n";

/// The fragment shader of a depth-only program that cuts out texels whose
/// colour map's alpha is below a half (foliage cards), as `shadePbr` does.
/// Reads `TexCoords`.
pub const ALPHA_TEST_FRAGMENT: &str = "#version 460 core
in vec2 TexCoords;
uniform sampler2D baseColorMap;
uniform int hasBaseColorMap;
void main() {
    if (hasBaseColorMap == 1 && texture(baseColorMap, TexCoords).a < 0.5) {
        discard;
    }
}
";
