use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4};
use glfw::{Action, CursorMode, MouseButton};

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
        Self {
            ray: None,
            position: Point3::new(0.0, 0.0, 0.0),
            camera: Matrix4::identity(),
            projection: Matrix4::identity(),
        }
    }

    pub fn update(&mut self, camera: &Camera, projection: &Projection) {
        // Where the view is drawn from: the camera controller moves the
        // eye (`relative_position`), not the camera's base.
        self.position = camera.get_eye_position();
        self.camera = camera.get_matrix();
        self.projection = projection.get_matrix();
    }

    /// Direction of the ray through `ndc` (normalized device coordinates,
    /// −1..1 across the view, +y up).
    fn calculate_ray(&self, ndc: (f32, f32)) -> Vector3<f32> {
        let ray_clip = Vector4::new(ndc.0, ndc.1, -1.0, 1.0);
        let ray_eye = self.projection.invert().unwrap() * ray_clip;
        let ray_eye = Vector4::new(ray_eye.x, ray_eye.y, -1.0, 0.0);

        (self.camera.invert().unwrap() * ray_eye)
            .truncate()
            .normalize()
    }

    pub fn handle_event(
        &mut self,
        _: &mut glfw::Glfw,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) -> Option<(Line, MouseButton)> {
        let line: Option<(Line, glfw::MouseButton)> = match event {
            glfw::WindowEvent::MouseButton(button, action, _) => {
                if *action == Action::Press {
                    let ray = self.calculate_ray(cursor_ndc(window));
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

/// Where the cursor points, in normalized device coordinates over the
/// window: the centre while it is captured (the crosshair of a first person
/// view), where it is otherwise.
fn cursor_ndc(window: &glfw::Window) -> (f32, f32) {
    let (width, height) = window.get_size();
    if window.get_cursor_mode() == CursorMode::Disabled || width <= 0 || height <= 0 {
        return (0.0, 0.0);
    }
    let (x, y) = window.get_cursor_pos();
    (
        (2.0 * x / width as f64 - 1.0) as f32,
        (1.0 - 2.0 * y / height as f64) as f32,
    )
}

#[cfg(test)]
mod tests {
    use cgmath::{Deg, Rad};

    use super::*;

    #[test]
    fn rays_start_at_the_eye_and_go_through_the_point_picked() {
        // Yaw 0 looks along +x, so +z is to the right.
        let mut camera = Camera::new((0.0, 0.0, 0.0), Rad(0.0), Rad(0.0));
        camera.set_relative_position((5.0, 1.0, 0.0));
        let projection = Projection::new(800, 800, Deg(90.0), 0.1, 100.0);
        let mut picker = MousePicker::new();
        picker.update(&camera, &projection);
        assert_eq!(picker.position, Point3::new(5.0, 1.0, 0.0));

        let centre = picker.calculate_ray((0.0, 0.0));
        assert!((centre - Vector3::unit_x()).magnitude() < 1e-4, "{centre:?}");
        // The right edge of a 90° view is 45° off to the right.
        let right = picker.calculate_ray((1.0, 0.0));
        let expected = Vector3::new(1.0, 0.0, 1.0).normalize();
        assert!((right - expected).magnitude() < 1e-4, "{right:?}");
    }
}
