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

    pub fn update_light_view(&mut self, camera: &Camera, projection: &Projection) {
        // Calculate the frustum center in world space
        let camera_pos = camera.get_position();
        let camera_rot = camera.calc_rotation_matrix();
        let fov = projection.fovy.0;
        let aspect = projection.aspect;
        let near = projection.znear;
        let far = SHADOW_DISTANCE;

        let forward = camera_rot.transform_vector(Vector3::unit_z()).normalize();
        let up = camera_rot.transform_vector(Vector3::unit_y()).normalize();
        let right = forward.cross(up).normalize();

        let nc = camera_pos + forward * near;
        let fc = camera_pos + forward * far;

        let far_height = 2.0 * (fov / 2.0).tan() * far;
        let far_width = far_height * aspect;

        // Frustum center is the center of the far plane
        let center = fc;

        // Light direction
        let light_direction = -self.position.to_vec().normalize();
        let distance = SHADOW_DISTANCE;
        let light_pos = center - light_direction * distance;
        self.light_view = Matrix4::look_at_rh(light_pos, center, up);
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
            self.update_light_view(&camera, &projection);
            self.shadow_box.update(self.light_view, &camera, &projection);
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

        // Camera parameters
        let camera_pos = camera.get_position();
        let camera_rot = camera.calc_rotation_matrix();
        let fov = projection.fovy.0;
        let aspect = projection.aspect;
        let near = projection.znear;
        let far = SHADOW_DISTANCE;

        // Calculate up, right, forward
        let forward = camera_rot.transform_vector(Vector3::unit_z()).normalize();
        let up = camera_rot.transform_vector(Vector3::unit_y()).normalize();
        let right = forward.cross(up).normalize();

        // Near and far plane centers
        let nc = camera_pos + forward * near;
        let fc = camera_pos + forward * far;

        // Near and far plane sizes
        let near_height = 2.0 * (fov / 2.0).tan() * near;
        let near_width = near_height * aspect;
        let far_height = 2.0 * (fov / 2.0).tan() * far;
        let far_width = far_height * aspect;

        // 8 frustum corners in world space
        let frustum_corners = vec![
            // Near plane
            nc + (up * (near_height / 2.0)) - (right * (near_width / 2.0)), // ntl
            nc + (up * (near_height / 2.0)) + (right * (near_width / 2.0)), // ntr
            nc - (up * (near_height / 2.0)) - (right * (near_width / 2.0)), // nbl
            nc - (up * (near_height / 2.0)) + (right * (near_width / 2.0)), // nbr
            // Far plane
            fc + (up * (far_height / 2.0)) - (right * (far_width / 2.0)), // ftl
            fc + (up * (far_height / 2.0)) + (right * (far_width / 2.0)), // ftr
            fc - (up * (far_height / 2.0)) - (right * (far_width / 2.0)), // fbl
            fc - (up * (far_height / 2.0)) + (right * (far_width / 2.0)), // fbr
        ];

        // Transform all corners to light space
        let light_space_corners: Vec<_> = frustum_corners
            .iter()
            .map(|corner| {
                let v = light_view * Vector4::new(corner.x, corner.y, corner.z, 1.0);
                v.truncate()
            })
            .collect();

        // Find min/max bounds
        let mut min = light_space_corners[0];
        let mut max = light_space_corners[0];
        for v in &light_space_corners[1..] {
            min.x = min.x.min(v.x);
            min.y = min.y.min(v.y);
            min.z = min.z.min(v.z);
            max.x = max.x.max(v.x);
            max.y = max.y.max(v.y);
            max.z = max.z.max(v.z);
        }

        self.min_x = min.x;
        self.max_x = max.x;
        self.min_y = min.y;
        self.max_y = max.y;
        self.min_z = min.z;
        self.max_z = max.z + OFFSET;

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
