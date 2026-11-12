use cgmath::{Deg, EuclideanSpace, Matrix4, Point3, Rad};

use crate::core::{
    camera::{Camera, CameraController, Projection},
    entity::Entity,
    scene::{scene_file::ComponentData, Scene},
};

use super::{Component, Property, PropertyValue};

pub struct CameraComponent {
    camera: Camera,
    projection: Projection,
    camera_controller: CameraController,
}

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

    pub fn get_view_projection(&self) -> Matrix4<f32> {
        self.projection.get_matrix() * self.camera.get_matrix()
    }
}

impl Component for CameraComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, delta_time: f64) {
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

    fn display_name(&self) -> String {
        "Camera".into()
    }

    fn to_data(&self) -> Option<ComponentData> {
        let eye = self.camera.get_eye_position();
        Some(ComponentData::Camera {
            position: [eye.x, eye.y, eye.z],
            yaw_deg: Deg::from(self.camera.get_yaw()).0,
            pitch_deg: Deg::from(self.camera.get_pitch()).0,
            fov_deg: Deg::from(self.projection.fovy).0,
            near: self.projection.znear,
            far: self.projection.get_zfar(),
            speed: self.camera_controller.get_speed(),
        })
    }

    fn properties(&self) -> Vec<Property> {
        let eye = self.camera.get_eye_position();
        vec![
            Property::float("position x", eye.x),
            Property::float("position y", eye.y),
            Property::float("position z", eye.z),
            Property::float("yaw", Deg::from(self.camera.get_yaw()).0),
            Property::ranged("pitch", Deg::from(self.camera.get_pitch()).0, -89.0, 89.0),
            Property::ranged("fov", Deg::from(self.projection.fovy).0, 10.0, 120.0),
            Property::float("near", self.projection.znear),
            Property::float("far", self.projection.get_zfar()),
            Property::float("speed", self.camera_controller.get_speed()),
        ]
    }

    fn set_property(&mut self, name: &str, value: PropertyValue) {
        let Some(v) = value.as_f32() else {
            return;
        };
        let mut eye = self.camera.get_eye_position();
        match name {
            "position x" | "position y" | "position z" => {
                match name {
                    "position x" => eye.x = v,
                    "position y" => eye.y = v,
                    _ => eye.z = v,
                }
                // Fold any controller offset into the base position so the
                // edited value is exactly what is shown.
                self.camera.set_relative_position(Point3::origin());
                self.camera.set_position(eye);
            }
            "yaw" => {
                let pitch = self.camera.get_pitch();
                self.camera.set_yaw_pitch(Rad::from(Deg(v)), pitch);
            }
            "pitch" => {
                let yaw = self.camera.get_yaw();
                self.camera
                    .set_yaw_pitch(yaw, Rad::from(Deg(v.clamp(-89.0, 89.0))));
            }
            "fov" => self.projection.set_fovy(Deg(v.clamp(10.0, 120.0))),
            "near" => self.projection.set_clip(v, self.projection.get_zfar()),
            "far" => self.projection.set_clip(self.projection.znear, v),
            "speed" => self.camera_controller.set_speed(v),
            _ => {}
        }
    }
}
