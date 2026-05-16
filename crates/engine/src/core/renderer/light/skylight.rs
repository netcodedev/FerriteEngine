use cgmath::{ortho, EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    camera::{Camera, Projection},
    entity::{
        component::{camera_component::CameraComponent, Component},
        Entity,
    },
    scene::Scene,
};

const OFFSET: f32 = 150.0;
const SHADOW_DISTANCE: f32 = 50.0;

pub struct SkyLight {
    position: Point3<f32>,
    light_view: Matrix4<f32>,
    shadow_box: ShadowBox,
}

impl SkyLight {
    pub fn new<P: Into<Point3<f32>>>(position: P) -> Self {
        let position = position.into();
        let light_view = Matrix4::identity();
        Self {
            position: position.clone(),
            light_view,
            shadow_box: ShadowBox::new(light_view),
        }
    }

    pub fn update_light_view(&mut self, camera: &Camera, projection: &Projection) {
        let camera_pos = camera.get_eye_position();
        let inv_view = camera.get_matrix().invert().unwrap();

        let forward = (inv_view * Vector4::new(0.0, 0.0, -1.0, 0.0))
            .truncate()
            .normalize();
        let up_cam = (inv_view * Vector4::new(0.0, 1.0, 0.0, 0.0))
            .truncate()
            .normalize();
        let right_cam = (inv_view * Vector4::new(1.0, 0.0, 0.0, 0.0))
            .truncate()
            .normalize();

        let fov = projection.fovy.0;
        let aspect = projection.aspect;
        let near = projection.znear;
        let far = SHADOW_DISTANCE;

        let nc = camera_pos + forward * near;
        let fc = camera_pos + forward * far;

        let near_height = 2.0 * (fov / 2.0).tan() * near;
        let near_width = near_height * aspect;
        let far_height = 2.0 * (fov / 2.0).tan() * far;
        let far_width = far_height * aspect;

        let frustum_corners = [
            nc + up_cam * (near_height / 2.0) - right_cam * (near_width / 2.0),
            nc + up_cam * (near_height / 2.0) + right_cam * (near_width / 2.0),
            nc - up_cam * (near_height / 2.0) - right_cam * (near_width / 2.0),
            nc - up_cam * (near_height / 2.0) + right_cam * (near_width / 2.0),
            fc + up_cam * (far_height / 2.0) - right_cam * (far_width / 2.0),
            fc + up_cam * (far_height / 2.0) + right_cam * (far_width / 2.0),
            fc - up_cam * (far_height / 2.0) - right_cam * (far_width / 2.0),
            fc - up_cam * (far_height / 2.0) + right_cam * (far_width / 2.0),
        ];

        let mut center = Point3::new(0.0, 0.0, 0.0);
        for corner in &frustum_corners {
            center += corner.to_vec();
        }
        center /= 8.0;

        let light_direction = -self.position.to_vec().normalize();
        let distance = SHADOW_DISTANCE;
        let light_pos = center - light_direction * distance;

        let light_up = if light_direction.x.abs() < 0.001 && light_direction.z.abs() < 0.001 {
            Vector3::unit_z()
        } else {
            Vector3::unit_y()
        };

        self.light_view = Matrix4::look_at_rh(light_pos, center, light_up);

        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;

        for corner in &frustum_corners {
            let trf = self.light_view * Vector4::new(corner.x, corner.y, corner.z, 1.0);
            min_x = min_x.min(trf.x);
            max_x = max_x.max(trf.x);
            min_y = min_y.min(trf.y);
            max_y = max_y.max(trf.y);
            min_z = min_z.min(trf.z);
            max_z = max_z.max(trf.z);
        }

        self.shadow_box
            .update_tight(self.light_view, min_x, max_x, min_y, max_y, min_z, max_z);
    }

    pub fn get_position(&self) -> Point3<f32> {
        self.position
    }

    pub fn get_projection(&self) -> Matrix4<f32> {
        self.shadow_box.light_projection * self.light_view
    }
}

impl Component for SkyLight {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, _: f64) {
        if let Some(camera_component) = scene.get_component::<CameraComponent>() {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();
            self.update_light_view(&camera, &projection);
        }
    }

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}

#[derive(Debug)]
struct ShadowBox {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
    min_z: f32,
    max_z: f32,

    light_view: Matrix4<f32>,
    light_projection: Matrix4<f32>,
}

impl ShadowBox {
    fn new(light_view: Matrix4<f32>) -> Self {
        Self {
            min_x: 0.0,
            max_x: 0.0,
            min_y: 0.0,
            max_y: 0.0,
            min_z: 0.0,
            max_z: 0.0,
            light_view,
            light_projection: Matrix4::identity(),
        }
    }

    fn update_tight(
        &mut self,
        light_view: Matrix4<f32>,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
        min_z: f32,
        max_z: f32,
    ) {
        self.light_view = light_view;

        // Use exact bounds of the view frustum in light space
        self.min_x = min_x;
        self.max_x = max_x;
        self.min_y = min_y;
        self.max_y = max_y;
        self.min_z = min_z;
        self.max_z = max_z;

        self.update_projection();
    }

    fn update_projection(&mut self) {
        // Extend the near plane significantly backwards to catch shadow casters in front of the camera
        let near = -self.max_z - OFFSET;
        // Extend the far plane slightly
        let far = -self.min_z + OFFSET;
        self.light_projection = ortho(self.min_x, self.max_x, self.min_y, self.max_y, near, far);
    }
}
