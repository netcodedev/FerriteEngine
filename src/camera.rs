use cgmath::*;
use glfw::{Action, CursorMode, Key};
use std::f32::consts::FRAC_PI_2;

use crate::{
    line::Line,
    terrain::{ChunkBounds, CHUNK_SIZE},
};

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);

const SAFE_FRAC_PI_2: f32 = FRAC_PI_2 - 0.0001;

#[derive(Debug)]
pub struct Camera {
    pub position: Point3<f32>,
    pub yaw: Rad<f32>,
    pub pitch: Rad<f32>,
}

impl Camera {
    pub fn new<V: Into<Point3<f32>>, Y: Into<Rad<f32>>, P: Into<Rad<f32>>>(
        position: V,
        yaw: Y,
        pitch: P,
    ) -> Self {
        Self {
            position: position.into(),
            yaw: yaw.into(),
            pitch: pitch.into(),
        }
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        let (sin_pitch, cos_pitch) = self.pitch.0.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.0.sin_cos();

        Matrix4::look_to_rh(
            self.position,
            Vector3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize(),
            Vector3::unit_y(),
        )
    }
}

pub struct Projection {
    aspect: f32,
    fovy: Rad<f32>,
    znear: f32,
    zfar: f32,
}

impl Projection {
    pub fn new<F: Into<Rad<f32>>>(width: u32, height: u32, fovy: F, znear: f32, zfar: f32) -> Self {
        Self {
            aspect: width as f32 / height as f32,
            fovy: fovy.into(),
            znear,
            zfar,
        }
    }

    pub fn resize(&mut self, event: &glfw::WindowEvent) {
        if let glfw::WindowEvent::FramebufferSize(width, height) = event {
            self.aspect = *width as f32 / *height as f32;
            unsafe {
                gl::Viewport(0, 0, *width, *height);
            }
        }
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        OPENGL_TO_WGPU_MATRIX * perspective(self.fovy, self.aspect, self.znear, self.zfar)
    }
}

pub struct ViewFrustum {}

impl ViewFrustum {
    pub fn is_bounds_in_frustum(
        projection: &Projection,
        camera: &Camera,
        bounds: ChunkBounds,
    ) -> bool {
        let mut result = false;

        // check if bounds are close to camera
        let distance = (camera.position - bounds.center()).magnitude();
        if distance < CHUNK_SIZE as f32 * 0.75 {
            return true;
        }

        let view_projection = projection.calc_matrix() * camera.calc_matrix();
        let clip: [Vector4<f32>; 8] = [
            Vector4::new(
                bounds.min.0 as f32,
                bounds.min.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.min.0 as f32,
                bounds.min.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.min.0 as f32,
                bounds.max.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.min.0 as f32,
                bounds.max.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.min.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.min.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.max.1 as f32,
                bounds.min.2 as f32,
                1.0,
            ),
            Vector4::new(
                bounds.max.0 as f32,
                bounds.max.1 as f32,
                bounds.max.2 as f32,
                1.0,
            ),
        ];

        for point in clip {
            let point = view_projection * point;
            if point.x <= point.w
                && point.x >= -point.w
                && point.y <= point.w
                && point.y >= -point.w
                && point.z <= point.w
                && point.z >= -point.w
            {
                result = true;
                break;
            }
        }

        result
    }
}

#[derive(Debug)]
pub struct CameraController {
    amount_left: f32,
    amount_right: f32,
    amount_forward: f32,
    amount_backward: f32,
    amount_up: f32,
    amount_down: f32,
    rotate_horizontal: f32,
    rotate_vertical: f32,
    speed: f32,
    sensitivity: f32,
    is_active: bool,
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
            rotate_horizontal: 0.0,
            rotate_vertical: 0.0,
            speed,
            sensitivity,
            is_active: false,
        }
    }

    pub fn get_speed(&self) -> f32 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
        if self.speed < 0.0 {
            self.speed = 0.0;
        }
    }

    pub fn process_keyboard(
        &mut self,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) -> bool {
        match event {
            glfw::WindowEvent::Key(Key::W | Key::Up, _, action, _) => {
                let amount = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                self.amount_forward = amount;
                true
            }
            glfw::WindowEvent::Key(Key::S | Key::Down, _, action, _) => {
                let amount = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                self.amount_backward = amount;
                true
            }
            glfw::WindowEvent::Key(Key::A | Key::Left, _, action, _) => {
                let amount = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                self.amount_left = amount;
                true
            }
            glfw::WindowEvent::Key(Key::D | Key::Right, _, action, _) => {
                let amount = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                self.amount_right = amount;
                true
            }
            glfw::WindowEvent::Key(Key::Space, _, action, _) => {
                let amount = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                self.amount_up = amount;
                true
            }
            glfw::WindowEvent::Key(Key::LeftShift, _, action, _) => {
                let amount = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                self.amount_down = amount;
                true
            }
            glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                match window.get_cursor_mode() {
                    CursorMode::Disabled => window.set_cursor_mode(CursorMode::Normal),
                    CursorMode::Normal => window.set_cursor_mode(CursorMode::Disabled),
                    _ => {}
                }
                self.is_active = !self.is_active;
                true
            }
            _ => false,
        }
    }

    pub fn process_mouse(&mut self, window: &mut glfw::Window, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::CursorPos(xpos, ypos) => match window.get_cursor_mode() {
                CursorMode::Disabled => {
                    if self.is_active {
                        self.rotate_horizontal = *xpos as f32;
                        self.rotate_vertical = *ypos as f32;

                        if self.rotate_horizontal.abs() > 250.0 {
                            self.rotate_horizontal = 0.0;
                        }
                        if self.rotate_vertical.abs() > 250.0 {
                            self.rotate_vertical = 0.0;
                        }

                        window.set_cursor_pos(0.0, 0.0);
                    }
                }
                _ => {}
            },
            glfw::WindowEvent::Scroll(_, y) => {
                self.set_speed(self.speed + (*y as f32 * 10.0));
            }
            _ => {}
        }
    }

    pub fn update_camera(&mut self, camera: &mut Camera, delta_time: f32) {
        // Move forward/backward and left/right
        let (yaw_sin, yaw_cos) = camera.yaw.0.sin_cos();
        let forward = Vector3::new(yaw_cos, 0.0, yaw_sin).normalize();
        let right = Vector3::new(-yaw_sin, 0.0, yaw_cos).normalize();
        camera.position +=
            forward * (self.amount_forward - self.amount_backward) * self.speed * delta_time;
        camera.position += right * (self.amount_right - self.amount_left) * self.speed * delta_time;

        // Move up/down. Since we don't use roll, we can just
        // modify the y coordinate directly.
        camera.position.y += (self.amount_up - self.amount_down) * self.speed * delta_time;

        // Rotate
        camera.yaw += Rad(self.rotate_horizontal) * self.sensitivity * delta_time;
        camera.pitch += Rad(-self.rotate_vertical) * self.sensitivity * delta_time;

        self.rotate_horizontal = 0.0;
        self.rotate_vertical = 0.0;

        // Keep the camera's angle from going too high/low.
        if camera.pitch < -Rad(SAFE_FRAC_PI_2) {
            camera.pitch = -Rad(SAFE_FRAC_PI_2);
        } else if camera.pitch > Rad(SAFE_FRAC_PI_2) {
            camera.pitch = Rad(SAFE_FRAC_PI_2);
        }
    }
}

pub struct MousePicker {
    pub ray: Option<Line>,
}

impl MousePicker {
    pub fn new() -> Self {
        Self { ray: None }
    }

    pub fn process_mouse(
        &mut self,
        event: &glfw::WindowEvent,
        camera: &Camera,
        projection: &Projection,
    ) -> Option<(Line, glfw::MouseButton)> {
        let line: Option<(Line, glfw::MouseButton)> = match event {
            glfw::WindowEvent::MouseButton(button, action, _) => {
                if *action == Action::Press {
                    let ray = self.calculate_ray(camera, projection);
                    let line = Line::new(camera.position, ray, 20.0);
                    self.ray = Some(line.clone());
                    match button {
                        glfw::MouseButton::Button1 => Some((line, glfw::MouseButton::Button1)),
                        glfw::MouseButton::Button2 => Some((line, glfw::MouseButton::Button2)),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        line
    }

    pub fn calculate_ray(&mut self, camera: &Camera, projection: &Projection) -> Vector3<f32> {
        let ray_clip = Vector4::new(0.0, 0.0, -1.0, 1.0);
        let ray_eye = projection.calc_matrix().invert().unwrap() * ray_clip;
        let ray_eye = Vector4::new(ray_eye.x, ray_eye.y, -1.0, 0.0);

        (camera.calc_matrix().invert().unwrap() * ray_eye)
            .truncate()
            .normalize()
    }
}
