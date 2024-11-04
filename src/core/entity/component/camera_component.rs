use crate::core::{camera::{Camera, CameraController, Projection}, scene::Scene};

use super::Component;

pub struct CameraComponent {
    camera: Camera,
    projection: Projection,
    camera_controller: CameraController,
}

#[allow(dead_code)]
impl CameraComponent {
    pub fn new(
        camera: Camera,
        projection: Projection,
        camera_controller: CameraController,
    ) -> Self {
        CameraComponent {
            camera,
            projection,
            camera_controller,
        }
    }

    pub fn get_camera(&self) -> &Camera {
        &self.camera
    }

    pub fn get_camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn get_projection(&self) -> &Projection {
        &self.projection
    }

    pub fn get_projection_mut(&mut self) -> &mut Projection {
        &mut self.projection
    }

    pub fn get_camera_controller(&self) -> &CameraController {
        &self.camera_controller
    }

    pub fn get_camera_controller_mut(&mut self) -> &mut CameraController {
        &mut self.camera_controller
    }
}

impl Component for CameraComponent {
    fn update(&mut self, _: &Scene, delta_time: f64) {
        self.camera_controller
            .update_camera(&mut self.camera, delta_time as f32);
    }

    fn handle_event(
        &mut self,
        _: &mut glfw::Glfw,
        window: &mut glfw::Window,
        event: &glfw::WindowEvent,
    ) {
        self.camera_controller.process_keyboard(window, event);
        self.camera_controller.process_mouse(window, event);
        self.projection.resize(&event);
    }
}
