use cgmath::{
    ortho, Angle, Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, Rad, SquareMatrix, Transform,
    Vector2, Vector3, Vector4,
};
use glfw::{Glfw, WindowEvent};

use crate::core::{
    camera::{Camera, Projection},
    entity::{
        component::{camera_component::CameraComponent, Component},
        Entity,
    },
    scene::Scene,
};

const OFFSET: f32 = 10.0;
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

    pub fn update_light_view(&mut self) {
        let light_direction = -self.position.to_vec().normalize();
        let center = -self.shadow_box.get_center().to_vec();
        let mut light_view: Matrix4<f32> = Matrix4::identity();
        let pitch = Vector2::new(light_direction.x, light_direction.z)
            .magnitude()
            .acos();
        light_view = light_view * Matrix4::from_angle_x(Rad(pitch));
        let mut yaw = Deg((light_direction.x / light_direction.z).atan());
        if light_direction.z > 0.0 {
            yaw = yaw - Deg(180.0)
        }
        light_view = light_view * Matrix4::from_angle_y(-yaw);
        self.light_view = light_view * Matrix4::from_translation(center);
    }

    pub fn get_position(&self) -> Point3<f32> {
        self.position
    }

    pub fn get_projection(&self) -> Matrix4<f32> {
        let projection = self.shadow_box.light_projection * self.light_view;
        projection
    }
}

impl Component for SkyLight {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, _: f64) {
        if let Some(camera_component) = scene.get_component::<CameraComponent>() {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();
            self.shadow_box
                .update(self.light_view, &camera, &projection);
            self.update_light_view();
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

    far_height: f32,
    far_width: f32,
    near_height: f32,
    near_width: f32,

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

            far_height: 0.0,
            far_width: 0.0,
            near_height: 0.0,
            near_width: 0.0,
            light_view,
            light_projection: Matrix4::identity(),
        }
    }

    fn update(&mut self, light_view: Matrix4<f32>, camera: &Camera, projection: &Projection) {
        self.light_view = light_view;

        let camera_rotation = camera.calc_rotation_matrix();
        let camera_position = camera.get_position();
        self.update_widths_and_heights(projection.fovy, projection.aspect);

        let forward_vector = camera_rotation.transform_vector(Vector3::unit_z());
        let to_far = forward_vector * SHADOW_DISTANCE;
        let to_near = forward_vector * projection.znear;
        let center_near = camera_position + to_near;
        let center_far = camera_position + to_far;

        let points = self.calculate_frustum_vertices(
            camera_rotation,
            forward_vector,
            center_near,
            center_far,
        );
        let mut first = true;
        for point in points {
            if first {
                self.min_x = point.x;
                self.max_x = point.x;
                self.min_y = point.y;
                self.max_y = point.y;
                self.min_z = point.z;
                self.max_z = point.z;
                first = false;
                continue;
            }
            if point.x > self.max_x {
                self.max_x = point.x;
            } else if point.x < self.min_x {
                self.min_x = point.x;
            }
            if point.y > self.max_y {
                self.max_y = point.y;
            } else if point.y < self.min_y {
                self.min_y = point.y;
            }
            if point.z > self.max_z {
                self.max_z = point.z;
            } else if point.z < self.min_z {
                self.min_z = point.z;
            }
        }
        self.max_z += OFFSET;

        self.update_projection();
    }

    fn get_center(&self) -> Point3<f32> {
        let center = Point3::new(
            (self.min_x + self.max_x) / 2.0,
            (self.min_y + self.max_y) / 2.0,
            (self.min_z + self.max_z) / 2.0,
        );
        self.light_view.invert().unwrap().transform_point(center)
    }

    fn update_projection(&mut self) {
        self.light_projection = ortho(
            self.min_x, self.max_x, self.min_y, self.max_y, self.min_z, self.max_z,
        );
    }

    fn update_widths_and_heights(&mut self, fov_y: Rad<f32>, aspect: f32) {
        self.far_width = SHADOW_DISTANCE * fov_y.tan();
        self.near_width = self.min_z * fov_y.tan();
        self.far_height = self.far_width / aspect;
        self.near_height = self.near_width / aspect;
    }

    fn calculate_frustum_vertices(
        &self,
        camera_rotation: Matrix4<f32>,
        forward_vector: Vector3<f32>,
        center_near: Point3<f32>,
        center_far: Point3<f32>,
    ) -> Vec<Vector4<f32>> {
        let up_vector = camera_rotation.transform_vector(Vector3::unit_y());
        let right_vector = forward_vector.cross(up_vector);
        let down_vector = -up_vector;
        let left_vector = -right_vector;

        let far_top = center_far + up_vector * self.far_height;
        let far_bottom = center_far + down_vector * self.far_height;
        let near_top = center_near + up_vector * self.near_height;
        let near_bottom = center_near + down_vector * self.near_height;

        vec![
            self.calculate_light_space_frustum_corner(far_top, right_vector, self.far_width),
            self.calculate_light_space_frustum_corner(far_top, left_vector, self.far_width),
            self.calculate_light_space_frustum_corner(far_bottom, right_vector, self.far_width),
            self.calculate_light_space_frustum_corner(far_bottom, left_vector, self.far_width),
            self.calculate_light_space_frustum_corner(near_top, right_vector, self.near_width),
            self.calculate_light_space_frustum_corner(near_top, left_vector, self.near_width),
            self.calculate_light_space_frustum_corner(near_bottom, right_vector, self.near_width),
            self.calculate_light_space_frustum_corner(near_bottom, left_vector, self.near_width),
        ]
    }

    fn calculate_light_space_frustum_corner(
        &self,
        point: Point3<f32>,
        direction: Vector3<f32>,
        width: f32,
    ) -> Vector4<f32> {
        let corner = point + direction * width;
        self.light_view * Vector4::new(corner.x, corner.y, corner.z, 1.0)
    }
}
