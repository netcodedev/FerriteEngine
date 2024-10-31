use cgmath::{InnerSpace, SquareMatrix, Vector3, Vector4};
use glfw::Action;

use crate::core::renderer::line::Line;

use super::camera::{Camera, Projection};

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
