use cgmath::*;
use glfw::{Action, Key, CursorMode};
use std::f32::consts::FRAC_PI_2;

use crate::line::Line;

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
    yaw: Rad<f32>,
    pitch: Rad<f32>,
}

impl Camera {
    pub fn new<
        V: Into<Point3<f32>>,
        Y: Into<Rad<f32>>,
        P: Into<Rad<f32>>,
    >(
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
            Vector3::new(
                cos_pitch * cos_yaw,
                sin_pitch,
                cos_pitch * sin_yaw
            ).normalize(),
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
    pub fn new<F: Into<Rad<f32>>>(
        width: u32,
        height: u32,
        fovy: F,
        znear: f32,
        zfar: f32,
    ) -> Self {
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
        }
    }

    pub fn process_keyboard(&mut self, event: &glfw::WindowEvent) -> bool{
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
            _ => false,
        }
    }

    pub fn process_mouse(&mut self, window: &mut glfw::Window, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::CursorPos(xpos, ypos) => {
                match window.get_cursor_mode() {
                    CursorMode::Disabled => {
                        self.rotate_horizontal = *xpos as f32;
                        self.rotate_vertical = *ypos as f32;

                        window.set_cursor_pos(0.0, 0.0);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn update_camera(&mut self, camera: &mut Camera, delta_time: f32) {
        // Move forward/backward and left/right
        let (yaw_sin, yaw_cos) = camera.yaw.0.sin_cos();
        let forward = Vector3::new(yaw_cos, 0.0, yaw_sin).normalize();
        let right = Vector3::new(-yaw_sin, 0.0, yaw_cos).normalize();
        camera.position += forward * (self.amount_forward - self.amount_backward) * self.speed * delta_time;
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
    pub rays: Vec<Line>,
}

impl MousePicker {
    pub fn new() -> Self {
        Self {
            rays: Vec::<Line>::new(),
        }
    }

    pub fn process_mouse(&mut self, event: &glfw::WindowEvent, camera: &Camera, projection: &Projection) -> Option<Line> {
        let line: Option<Line> = match event {
            glfw::WindowEvent::MouseButton(glfw::MouseButton::Button1, action, _) => {
                if *action == Action::Press {
                    let ray = self.calculate_ray(camera, projection);
                    let line = Line::new(camera.position, ray, 1000.0);
                    self.rays.push(line.clone());
                    Some(line)
                } else {
                    None
                }
            }
            _ => {
                None
            }
        };
        line
    }

    pub fn calculate_ray(&mut self, camera: &Camera, projection: &Projection) -> Vector3<f32>{
        let ray_clip = Vector4::new(0.0, 0.0, -1.0, 1.0);
        let ray_eye = projection.calc_matrix().invert().unwrap() * ray_clip;
        let ray_eye = Vector4::new(ray_eye.x, ray_eye.y, -1.0, 0.0);

        (camera.calc_matrix().invert().unwrap() * ray_eye).truncate().normalize()
    }
}