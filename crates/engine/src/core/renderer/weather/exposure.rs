//! The scene's depth seen from straight above, around the camera: whatever
//! is the first thing rain or snow falling straight down hits. A surface
//! below that depth is under a roof, a tree or an overhang and stays dry.
//! Drawn a second time with the terrain alone (`ground`), it tells how high
//! anything is above the ground, so snow can bury what lies on it.

use std::sync::atomic::{AtomicBool, Ordering};

use cgmath::{ortho, Matrix4, Point3, Vector3};
use gl::types::*;

use super::{EXPOSURE_UNIT, GROUND_UNIT};

/// Texels per side.
const SIZE: i32 = 1024;
/// Metres the map spans per side (about 16 cm per texel).
const EXTENT: f32 = 160.0;
/// Metres between the map's top and bottom.
const RANGE: f32 = 600.0;
/// How far above the camera the map starts: anything higher counts as sky.
const ABOVE_CAMERA: f32 = 250.0;
/// The map moves in steps this big, so it is not redrawn every frame: a
/// whole number of texels (64), so its texels stay where they were and
/// what is read from it does not jump when it moves.
const SNAP: f32 = EXTENT / SIZE as f32 * 64.0;
/// Frames a map is kept while the camera stays put, before it is drawn
/// again for things that moved.
const REDRAW_FRAMES: u32 = 30;

/// Set while the exposure map is drawn: shaders see a framebuffer of their
/// own then, as in the shadow pass, and characters (whose feet would keep
/// the ground under them dry) stay out of it.
static EXPOSURE_PASS: AtomicBool = AtomicBool::new(false);
/// Set while the ground map is drawn (during an exposure pass): only the
/// terrain draws itself then.
static GROUND_PASS: AtomicBool = AtomicBool::new(false);

/// Whether the exposure map is being drawn right now.
pub fn in_exposure_pass() -> bool {
    EXPOSURE_PASS.load(Ordering::Relaxed)
}

/// Whether the ground map is being drawn right now: everything but the
/// terrain stays out of it.
pub fn in_ground_pass() -> bool {
    GROUND_PASS.load(Ordering::Relaxed)
}

/// A depth map of its own: framebuffer and texture.
fn depth_target() -> (GLuint, GLuint) {
    let (mut framebuffer, mut depth) = (0, 0);
    unsafe {
        gl::GenTextures(1, &mut depth);
        gl::BindTexture(gl::TEXTURE_2D, depth);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::DEPTH_COMPONENT32F as GLint,
            SIZE,
            SIZE,
            0,
            gl::DEPTH_COMPONENT,
            gl::FLOAT,
            std::ptr::null(),
        );
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as GLint);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as GLint);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as GLint);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as GLint);
        gl::BindTexture(gl::TEXTURE_2D, 0);

        gl::GenFramebuffers(1, &mut framebuffer);
        gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
        gl::FramebufferTexture2D(
            gl::FRAMEBUFFER,
            gl::DEPTH_ATTACHMENT,
            gl::TEXTURE_2D,
            depth,
            0,
        );
        gl::DrawBuffer(gl::NONE);
        gl::ReadBuffer(gl::NONE);
        gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
    }
    (framebuffer, depth)
}

pub struct SkyExposure {
    framebuffer: GLuint,
    depth: GLuint,
    /// The terrain alone, from the same view.
    ground_framebuffer: GLuint,
    ground: GLuint,
    /// Corner (x, z) and top the map was drawn for, and frames since.
    drawn: Option<[f32; 3]>,
    age: u32,
    view_projection: Matrix4<f32>,
}

impl SkyExposure {
    pub fn new() -> Self {
        let (framebuffer, depth) = depth_target();
        let (ground_framebuffer, ground) = depth_target();
        Self {
            framebuffer,
            depth,
            ground_framebuffer,
            ground,
            drawn: None,
            age: 0,
            view_projection: Matrix4::from_scale(1.0),
        }
    }

    /// Start drawing the map for a camera at `eye`, if it is due: then the
    /// map's framebuffer is bound and cleared and `Some(view_projection)`
    /// comes back; draw the opaque scene with it and call `end`.
    pub fn begin(&mut self, eye: Point3<f32>) -> Option<Matrix4<f32>> {
        let snap = |v: f32| (v / SNAP).round() * SNAP;
        let area = [snap(eye.x), snap(eye.z), snap(eye.y + ABOVE_CAMERA)];
        if self.drawn == Some(area) && self.age < REDRAW_FRAMES {
            self.age += 1;
            return None;
        }
        self.drawn = Some(area);
        self.age = 0;

        let [x, z, top] = area;
        // Looking straight down with -z up the image: x runs right, z down,
        // a turn (not a mirror), so faces keep their winding.
        let view = Matrix4::look_at_rh(
            Point3::new(x, top, z),
            Point3::new(x, top - 1.0, z),
            -Vector3::unit_z(),
        );
        let half = EXTENT * 0.5;
        self.view_projection = ortho(-half, half, -half, half, 0.0, RANGE) * view;

        unsafe {
            // Not sampled while drawn into.
            gl::ActiveTexture(gl::TEXTURE0 + EXPOSURE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.framebuffer);
            gl::Viewport(0, 0, SIZE, SIZE);
            gl::DepthMask(gl::TRUE);
            gl::Clear(gl::DEPTH_BUFFER_BIT);
        }
        EXPOSURE_PASS.store(true, Ordering::Relaxed);
        Some(self.view_projection)
    }

    /// After `end`, when `begin` drew the map: start drawing the ground
    /// map from the same view. Draw the opaque scene again (only the
    /// terrain draws itself, see `in_ground_pass`) and call `end`.
    pub fn begin_ground(&self) -> Matrix4<f32> {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + GROUND_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.ground_framebuffer);
            gl::Viewport(0, 0, SIZE, SIZE);
            gl::DepthMask(gl::TRUE);
            gl::Clear(gl::DEPTH_BUFFER_BIT);
        }
        EXPOSURE_PASS.store(true, Ordering::Relaxed);
        GROUND_PASS.store(true, Ordering::Relaxed);
        self.view_projection
    }

    /// Done drawing the map; the default framebuffer is bound again.
    pub fn end(&self) {
        EXPOSURE_PASS.store(false, Ordering::Relaxed);
        GROUND_PASS.store(false, Ordering::Relaxed);
        unsafe { gl::BindFramebuffer(gl::FRAMEBUFFER, 0) };
    }

    /// World to the map's clip space, and the metres its depth spans.
    pub fn projection(&self) -> Option<(Matrix4<f32>, f32)> {
        self.drawn.map(|_| (self.view_projection, RANGE))
    }

    pub fn bind(&self) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0 + EXPOSURE_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, self.depth);
            gl::ActiveTexture(gl::TEXTURE0 + GROUND_UNIT);
            gl::BindTexture(gl::TEXTURE_2D, self.ground);
            gl::ActiveTexture(gl::TEXTURE0);
        }
    }
}

impl Drop for SkyExposure {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteFramebuffers(1, &self.framebuffer);
            gl::DeleteTextures(1, &self.depth);
            gl::DeleteFramebuffers(1, &self.ground_framebuffer);
            gl::DeleteTextures(1, &self.ground);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::Vector4;

    fn projected(m: &Matrix4<f32>, p: Point3<f32>) -> Vector4<f32> {
        let clip = m * p.to_homogeneous();
        clip / clip.w
    }

    #[test]
    fn the_map_looks_straight_down_without_mirroring() {
        let view = Matrix4::look_at_rh(
            Point3::new(0.0, 100.0, 0.0),
            Point3::new(0.0, 99.0, 0.0),
            -Vector3::unit_z(),
        );
        let m = ortho(-80.0, 80.0, -80.0, 80.0, 0.0, RANGE) * view;
        // +x goes right, +z goes down the image.
        let east = projected(&m, Point3::new(40.0, 0.0, 0.0));
        let south = projected(&m, Point3::new(0.0, 0.0, 40.0));
        assert!((east.x - 0.5).abs() < 1e-4 && east.y.abs() < 1e-4);
        assert!((south.y + 0.5).abs() < 1e-4 && south.x.abs() < 1e-4);
        // Higher is nearer (smaller depth).
        let high = projected(&m, Point3::new(0.0, 50.0, 0.0));
        let low = projected(&m, Point3::new(0.0, 0.0, 0.0));
        assert!(high.z < low.z);
    }
}
