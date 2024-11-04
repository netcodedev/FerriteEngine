use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4};
use glfw::{Action, MouseButton};

use crate::core::renderer::line::Line;

use super::camera::{Camera, Projection};

pub struct MousePicker {
    pub ray: Option<(Line, MouseButton)>,
    position: Point3<f32>,
    camera: Matrix4<f32>,
    projection: Matrix4<f32>,
}

impl MousePicker {
    pub fn new() -> Self {
        Self { ray: None, position: Point3::new(0.0,0.0,0.0), camera: Matrix4::identity(), projection: Matrix4::identity() }
    }

    pub fn update(&mut self, camera: &Camera, projection: &Projection) {
        self.position = camera.position;
        self.camera = camera.calc_matrix();
        self.projection = projection.calc_matrix();
    }

    fn calculate_ray(&mut self) -> Vector3<f32> {
        let ray_clip = Vector4::new(0.0, 0.0, -1.0, 1.0);
        let ray_eye = self.projection.invert().unwrap() * ray_clip;
        let ray_eye = Vector4::new(ray_eye.x, ray_eye.y, -1.0, 0.0);

        (self.camera.invert().unwrap() * ray_eye)
            .truncate()
            .normalize()
    }

    pub fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, event: &glfw::WindowEvent) -> Option<(Line, MouseButton)>{
        let line: Option<(Line, glfw::MouseButton)> = match event {
            glfw::WindowEvent::MouseButton(button, action, _) => {
                if *action == Action::Press {
                    let ray = self.calculate_ray();
                    let line = Line::new(self.position, ray, 20.0);
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
        if let Some(line) = line.clone() {
            self.ray = Some(line);
        }
        line
    }
}