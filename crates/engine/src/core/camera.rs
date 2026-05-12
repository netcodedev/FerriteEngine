use cgmath::{
    perspective, EuclideanSpace, Euler, InnerSpace, Matrix4, Point3, Rad, SquareMatrix, Vector3,
};
use glfw::{Action, Key};

use super::utils::DataSource;

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);


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
            self.aspect = *width as f32 / *height as f32;
            self.calc_matrix();
            unsafe {
                gl::Viewport(0, 0, *width, *height);
            }
        }
    }

    fn calc_matrix(&mut self) {
        self.matrix =
            OPENGL_TO_WGPU_MATRIX * perspective(self.fovy, self.aspect, self.znear, self.zfar);
    }

    pub fn get_matrix(&self) -> Matrix4<f32> {
        self.matrix
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
    speed: DataSource<f32>,
    /// Set by PlayerController each frame; when false this controller is a no-op.
    pub is_free: bool,
}

impl CameraController {
    pub fn new(speed: f32, _sensitivity: f32) -> Self {
        Self {
            amount_left: 0.0,
            amount_right: 0.0,
            amount_forward: 0.0,
            amount_backward: 0.0,
            amount_up: 0.0,
            amount_down: 0.0,
            speed: DataSource::new(speed),
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

    pub fn process_keyboard(
        &mut self,
        _window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) -> bool {
        if !self.is_free {
            return false;
        }
        match event {
            glfw::WindowEvent::Key(Key::Up, _, action, _) => {
                self.amount_forward = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                true
            }
            glfw::WindowEvent::Key(Key::Down, _, action, _) => {
                self.amount_backward = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                true
            }
            glfw::WindowEvent::Key(Key::Left, _, action, _) => {
                self.amount_left = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                true
            }
            glfw::WindowEvent::Key(Key::Right, _, action, _) => {
                self.amount_right = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                true
            }
            glfw::WindowEvent::Key(Key::Space, _, action, _) => {
                self.amount_up = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                true
            }
            glfw::WindowEvent::Key(Key::LeftShift, _, action, _) => {
                self.amount_down = match action {
                    Action::Press => 1.0,
                    Action::Release => 0.0,
                    _ => return false,
                };
                true
            }
            _ => false,
        }
    }

    pub fn process_mouse(&mut self, _window: &mut glfw::Window, event: &glfw::WindowEvent) {
        if let glfw::WindowEvent::Scroll(_, y) = event {
            self.set_speed(self.speed.read() + (*y as f32 * 10.0));
        }
    }

    pub fn update_camera(&mut self, camera: &mut Camera, delta_time: f32) {
        if !self.is_free {
            return;
        }

        let (yaw_sin, yaw_cos) = camera.yaw.0.sin_cos();
        let forward = Vector3::new(yaw_cos, 0.0, yaw_sin).normalize();
        let right = Vector3::new(-yaw_sin, 0.0, yaw_cos).normalize();

        let mut position = camera.relative_position;
        let yaw = camera.yaw;
        let pitch = camera.pitch;
        let speed = self.speed.read();

        position += forward * (self.amount_forward - self.amount_backward) * speed * delta_time;
        position += right * (self.amount_right - self.amount_left) * speed * delta_time;
        position.y += (self.amount_up - self.amount_down) * speed * delta_time;

        camera.update(position, yaw, pitch);
    }
}
