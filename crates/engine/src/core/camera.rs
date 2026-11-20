use cgmath::{
    perspective, EuclideanSpace, Euler, InnerSpace, Matrix4, Point3, Rad, SquareMatrix, Vector3,
};
use glfw::{Action, Key};

use super::utils::DataSource;

// Previously this multiplied the perspective by a wgpu-style depth remap
// matrix (z: [-1,1] → [0,1]). That matrix was authored row-major but cgmath's
// Matrix4::new takes column-major args, so the 0.5s ended up in the wrong
// cells and the resulting projection had a non-standard, distance-dependent
// FOV. OpenGL natively maps NDC z [-1,1] to depth [0,1] via glDepthRange, so
// the wgpu adjustment isn't needed here — using a plain perspective matrix
// gives a correct, standards-compliant projection.


#[derive(Debug)]
pub struct Camera {
    relative_position: Point3<f32>,
    position: Point3<f32>,
    yaw: Rad<f32>,
    pitch: Rad<f32>,

    matrix: Matrix4<f32>,
}

impl Camera {
    pub fn new<V: Into<Point3<f32>>, Y: Into<Rad<f32>>, P: Into<Rad<f32>>>(
        position: V,
        yaw: Y,
        pitch: P,
    ) -> Self {
        Self {
            relative_position: Point3::origin(),
            position: position.into(),
            yaw: yaw.into(),
            pitch: pitch.into(),
            matrix: Matrix4::identity(),
        }
    }

    fn calc_matrix(&mut self) {
        let (sin_pitch, cos_pitch) = self.pitch.0.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.0.sin_cos();

        self.matrix = Matrix4::look_to_rh(
            self.position + self.relative_position.to_vec(),
            Vector3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize(),
            Vector3::unit_y(),
        );
    }

    pub fn calc_rotation_matrix(&self) -> Matrix4<f32> {
        Matrix4::from(Euler::new(-self.yaw, -self.pitch, Rad(0.0)))
    }

    pub fn update(&mut self, position: Point3<f32>, yaw: Rad<f32>, pitch: Rad<f32>) {
        self.relative_position = position;
        self.yaw = yaw;
        self.pitch = pitch;
        self.calc_matrix();
    }

    pub fn set_position<P: Into<Point3<f32>>>(&mut self, position: P) {
        self.position = position.into();
        self.calc_matrix();
    }

    pub fn set_yaw_pitch(&mut self, yaw: Rad<f32>, pitch: Rad<f32>) {
        self.yaw = yaw;
        self.pitch = pitch;
        self.calc_matrix();
    }

    pub fn set_relative_position<P: Into<Point3<f32>>>(&mut self, position: P) {
        self.relative_position = position.into();
        self.calc_matrix();
    }

    pub fn get_position(&self) -> Point3<f32> {
        self.position
    }

    pub fn get_relative_position(&self) -> Point3<f32> {
        self.relative_position
    }

    pub fn get_eye_position(&self) -> Point3<f32> {
        self.position + self.relative_position.to_vec()
    }

    pub fn get_yaw(&self) -> Rad<f32> {
        self.yaw
    }

    pub fn get_pitch(&self) -> Rad<f32> {
        self.pitch
    }

    pub fn get_matrix(&self) -> Matrix4<f32> {
        self.matrix
    }
}

pub struct Projection {
    pub aspect: f32,
    pub fovy: Rad<f32>,
    pub znear: f32,
    zfar: f32,

    matrix: Matrix4<f32>,
}

impl Projection {
    pub fn new<F: Into<Rad<f32>>>(width: u32, height: u32, fovy: F, znear: f32, zfar: f32) -> Self {
        let mut projection = Self {
            aspect: width as f32 / height as f32,
            fovy: fovy.into(),
            znear,
            zfar,
            matrix: Matrix4::identity(),
        };
        projection.calc_matrix();
        projection
    }

    pub fn resize(&mut self, event: &glfw::WindowEvent) {
        if let glfw::WindowEvent::FramebufferSize(width, height) = event {
            // A minimized window's framebuffer is 0 by 0: no aspect at all,
            // so keep the last one.
            if *width <= 0 || *height <= 0 {
                return;
            }
            self.aspect = *width as f32 / *height as f32;
            self.calc_matrix();
            crate::core::gfx::set_viewport(crate::core::gfx::Rect::sized(*width as u32, *height as u32));
        }
    }

    fn calc_matrix(&mut self) {
        self.matrix = perspective(self.fovy, self.aspect, self.znear, self.zfar);
    }

    pub fn get_matrix(&self) -> Matrix4<f32> {
        self.matrix
    }

    /// Use when rendering into a sub-rectangle of the window (e.g. the
    /// editor viewport), whose aspect differs from the framebuffer's.
    pub fn set_aspect(&mut self, aspect: f32) {
        if aspect.is_finite() && aspect > 0.0 && (aspect - self.aspect).abs() > f32::EPSILON {
            self.aspect = aspect;
            self.calc_matrix();
        }
    }

    pub fn set_fovy<F: Into<Rad<f32>>>(&mut self, fovy: F) {
        self.fovy = fovy.into();
        self.calc_matrix();
    }

    pub fn get_zfar(&self) -> f32 {
        self.zfar
    }

    pub fn set_clip(&mut self, znear: f32, zfar: f32) {
        self.znear = znear.max(0.001);
        self.zfar = zfar.max(self.znear + 0.001);
        self.calc_matrix();
    }
}

/// Degrees the camera turns per pixel the mouse moves, by default.
pub const DEFAULT_MOUSE_SENSITIVITY: f32 = 0.12;

#[derive(Debug)]
pub struct CameraController {
    amount_left: f32,
    amount_right: f32,
    amount_forward: f32,
    amount_backward: f32,
    amount_up: f32,
    amount_down: f32,
    speed: DataSource<f32>,
    /// Degrees per pixel of mouse movement while flying.
    sensitivity: f32,
    /// Mouse movement not yet applied, in pixels.
    pending_look: (f32, f32),
    last_cursor: Option<(f64, f64)>,
    /// Whether this controller flies the camera (WASD or arrow keys, Space
    /// and Shift for up and down, the mouse to look while the cursor is
    /// captured). A player controller turns it off while it drives the
    /// camera itself.
    pub is_free: bool,
}

impl CameraController {
    pub fn new(speed: f32, sensitivity: f32) -> Self {
        Self {
            amount_left: 0.0,
            amount_right: 0.0,
            amount_forward: 0.0,
            amount_backward: 0.0,
            amount_up: 0.0,
            amount_down: 0.0,
            speed: DataSource::new(speed),
            sensitivity,
            pending_look: (0.0, 0.0),
            last_cursor: None,
            is_free: false,
        }
    }

    pub fn get_speed(&self) -> f32 {
        self.speed.read()
    }

    pub fn get_speed_ref(&self) -> DataSource<f32> {
        self.speed.clone()
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed.write(speed);
        if self.speed.read() < 0.0 {
            self.speed.write(0.0);
        }
    }

    pub fn get_sensitivity(&self) -> f32 {
        self.sensitivity
    }

    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        self.sensitivity = sensitivity.max(0.0);
    }

    /// Track a key; returns whether it is one of the flying keys.
    pub fn process_key(&mut self, key: Key, action: Action) -> bool {
        if !self.is_free {
            return false;
        }
        let value = match action {
            Action::Press => 1.0,
            Action::Release => 0.0,
            Action::Repeat => return true,
        };
        let amount = match key {
            Key::W | Key::Up => &mut self.amount_forward,
            Key::S | Key::Down => &mut self.amount_backward,
            Key::A | Key::Left => &mut self.amount_left,
            Key::D | Key::Right => &mut self.amount_right,
            Key::Space | Key::E => &mut self.amount_up,
            Key::LeftShift | Key::Q => &mut self.amount_down,
            _ => return false,
        };
        *amount = value;
        true
    }

    pub fn process_keyboard(
        &mut self,
        _window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) -> bool {
        match event {
            glfw::WindowEvent::Key(key, _, action, _) => self.process_key(*key, *action),
            _ => false,
        }
    }

    /// Track the cursor; `captured` is whether the game holds the mouse
    /// (cursor disabled). Only a captured cursor turns the camera.
    pub fn process_cursor(&mut self, x: f64, y: f64, captured: bool) {
        if let (true, true, Some((last_x, last_y))) = (self.is_free, captured, self.last_cursor) {
            self.pending_look.0 += (x - last_x) as f32;
            self.pending_look.1 += (y - last_y) as f32;
        }
        self.last_cursor = Some((x, y));
    }

    pub fn process_mouse(&mut self, window: &mut glfw::Window, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::Scroll(_, y) => {
                self.set_speed(self.speed.read() + (*y as f32 * 10.0));
            }
            glfw::WindowEvent::CursorPos(x, y) => {
                let captured = window.get_cursor_mode() == glfw::CursorMode::Disabled;
                self.process_cursor(*x, *y, captured);
            }
            _ => {}
        }
    }

    pub fn update_camera(&mut self, camera: &mut Camera, delta_time: f32) {
        if !self.is_free {
            self.pending_look = (0.0, 0.0);
            return;
        }

        let (dx, dy) = std::mem::take(&mut self.pending_look);
        let turn = self.sensitivity.to_radians();
        let yaw = Rad(camera.yaw.0 + dx * turn);
        let limit = 89f32.to_radians();
        let pitch = Rad((camera.pitch.0 - dy * turn).clamp(-limit, limit));

        let (yaw_sin, yaw_cos) = yaw.0.sin_cos();
        let forward = Vector3::new(yaw_cos, 0.0, yaw_sin).normalize();
        let right = Vector3::new(-yaw_sin, 0.0, yaw_cos).normalize();

        let mut position = camera.relative_position;
        let speed = self.speed.read();

        position += forward * (self.amount_forward - self.amount_backward) * speed * delta_time;
        position += right * (self.amount_right - self.amount_left) * speed * delta_time;
        position.y += (self.amount_up - self.amount_down) * speed * delta_time;

        camera.update(position, yaw, pitch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, SquareMatrix};

    fn projection() -> Projection {
        Projection::new(1600, 900, Deg(60.0), 0.1, 1000.0)
    }

    #[test]
    fn a_new_projection_has_the_frame_aspect() {
        let p = projection();
        assert!((p.aspect - (1600.0 / 900.0)).abs() < 1e-5);
    }

    #[test]
    fn set_aspect_only_rebuilds_when_the_value_changes() {
        let mut p = projection();
        let before = p.get_matrix();
        p.set_aspect(p.aspect);
        assert_eq!(before, p.get_matrix());
        p.set_aspect(1.0);
        assert_ne!(before, p.get_matrix());
    }

    #[test]
    fn a_zero_sized_framebuffer_keeps_the_projection() {
        let mut p = projection();
        let before = p.get_matrix();
        for (w, h) in [(0, 0), (0, 900), (1600, 0)] {
            p.resize(&glfw::WindowEvent::FramebufferSize(w, h));
            assert_eq!(before, p.get_matrix());
        }
    }

    #[test]
    fn set_aspect_rejects_zero_and_non_finite() {
        let mut p = projection();
        let before = p.get_matrix();
        p.set_aspect(0.0);
        p.set_aspect(f32::NAN);
        p.set_aspect(-1.0);
        assert_eq!(before, p.get_matrix());
    }

    #[test]
    fn set_clip_keeps_znear_positive_and_zfar_above_znear() {
        let mut p = projection();
        p.set_clip(-5.0, -10.0);
        assert!(p.znear > 0.0);
        assert!(p.get_zfar() > p.znear);
    }

    #[test]
    fn a_camera_matrix_at_the_origin_is_not_identity() {
        // Looking down +x from the origin: the view matrix is a rotation,
        // not the identity.
        let mut cam = Camera::new((0.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        cam.update(Point3::new(0.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        assert_ne!(cam.get_matrix(), Matrix4::identity());
    }

    #[test]
    fn a_controller_that_is_not_free_ignores_input() {
        let mut controller = CameraController::new(10.0, DEFAULT_MOUSE_SENSITIVITY);
        let mut cam = Camera::new((0.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        assert!(!controller.process_key(Key::W, Action::Press));
        controller.process_cursor(0.0, 0.0, true);
        controller.process_cursor(100.0, 0.0, true);
        controller.update_camera(&mut cam, 1.0);
        assert_eq!(cam.get_eye_position(), Point3::new(0.0, 0.0, 0.0));
        assert_eq!(cam.get_yaw(), Rad(0.0));
    }

    #[test]
    fn a_free_controller_flies_with_wasd_and_the_arrow_keys() {
        let mut controller = CameraController::new(10.0, DEFAULT_MOUSE_SENSITIVITY);
        controller.is_free = true;
        let mut cam = Camera::new((0.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        // Yaw 0 looks along +x.
        assert!(controller.process_key(Key::W, Action::Press));
        controller.update_camera(&mut cam, 0.5);
        assert!((cam.get_eye_position().x - 5.0).abs() < 1e-4);
        controller.process_key(Key::W, Action::Release);
        controller.process_key(Key::Up, Action::Press);
        controller.process_key(Key::Space, Action::Press);
        controller.update_camera(&mut cam, 0.5);
        let eye = cam.get_eye_position();
        assert!((eye.x - 10.0).abs() < 1e-4);
        assert!((eye.y - 5.0).abs() < 1e-4);
    }

    #[test]
    fn a_free_controller_looks_around_only_with_a_captured_cursor() {
        let mut controller = CameraController::new(10.0, 1.0);
        controller.is_free = true;
        let mut cam = Camera::new((0.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        // A free cursor (over the UI) does not turn the camera.
        controller.process_cursor(0.0, 0.0, false);
        controller.process_cursor(50.0, 0.0, false);
        controller.update_camera(&mut cam, 0.0);
        assert_eq!(cam.get_yaw(), Rad(0.0));
        // Captured: 30 px right and 10 px up at 1°/px.
        controller.process_cursor(80.0, -10.0, true);
        controller.update_camera(&mut cam, 0.0);
        assert!((Deg::from(cam.get_yaw()).0 - 30.0).abs() < 1e-3);
        assert!((Deg::from(cam.get_pitch()).0 - 10.0).abs() < 1e-3);
        // Pitch stops short of straight up.
        controller.process_cursor(80.0, -1000.0, true);
        controller.update_camera(&mut cam, 0.0);
        assert!(Deg::from(cam.get_pitch()).0 <= 89.0 + 1e-3);
    }

    #[test]
    fn get_eye_position_follows_position_and_relative() {
        let mut cam = Camera::new((10.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        cam.set_relative_position((1.0, 2.0, 3.0));
        let eye = cam.get_eye_position();
        assert!((eye.x - 11.0).abs() < 1e-5);
        assert!((eye.y - 2.0).abs() < 1e-5);
        assert!((eye.z - 3.0).abs() < 1e-5);
    }
}
