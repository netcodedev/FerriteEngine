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

/// Default real-time length of one in-game day, in seconds.
const DEFAULT_DAY_LENGTH_SECONDS: f32 = 240.0;
/// Sun radius used to position the directional light away from the scene
/// origin. The exact value doesn't matter — only the direction is used for
/// lighting — but keeping it large avoids precision issues in the shadow
/// matrix when the light passes near the zenith.
const SUN_DISTANCE: f32 = 200.0;

pub struct SkyLight {
    position: Point3<f32>,
    light_view: Matrix4<f32>,
    shadow_box: ShadowBox,

    /// Current time of day, in [0, 1). 0.0 == midnight, 0.5 == solar noon.
    time_of_day: f32,
    /// Seconds of real time per one full simulated day.
    day_length_seconds: f32,
    /// When true the cycle is frozen — manual F5/F6 steps still apply.
    paused: bool,
}

impl SkyLight {
    /// Construct a sky light. The initial direction is derived from the
    /// supplied `position` vector (only its direction matters; magnitude is
    /// normalised). The time-of-day defaults to morning (~8 AM).
    pub fn new<P: Into<Point3<f32>>>(position: P) -> Self {
        let p: Point3<f32> = position.into();
        // Recover an initial time-of-day from the seed position so a caller
        // can still hint at "morning sun in the east" with `(30, 20, 10)`.
        let dir = p.to_vec();
        let initial_time = if dir.x.abs() + dir.y.abs() < 0.0001 {
            8.0 / 24.0
        } else {
            // angle = time*2π - π/2  →  time = (atan2(y, x) + π/2) / 2π
            let angle = dir.y.atan2(dir.x);
            let mut t = (angle + std::f32::consts::FRAC_PI_2) / std::f32::consts::TAU;
            if t < 0.0 {
                t += 1.0;
            }
            t
        };

        let light_view = Matrix4::identity();
        let mut sl = Self {
            position: p,
            light_view,
            shadow_box: ShadowBox::new(light_view),
            time_of_day: initial_time,
            day_length_seconds: DEFAULT_DAY_LENGTH_SECONDS,
            paused: false,
        };
        sl.position = Point3::from_vec(sl.compute_sun_direction() * SUN_DISTANCE);
        sl
    }

    /// Returns the world-space unit vector pointing from the origin toward
    /// the sun. At t=0.25 (sunrise) it points along +X; at t=0.5 (noon) it
    /// points along +Y; at t=0.75 (sunset) it points along -X.
    pub fn compute_sun_direction(&self) -> Vector3<f32> {
        let angle = self.time_of_day * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        Vector3::new(angle.cos(), angle.sin(), 0.0).normalize()
    }

    pub fn get_sun_direction(&self) -> Vector3<f32> {
        self.compute_sun_direction()
    }

    pub fn get_time_of_day(&self) -> f32 {
        self.time_of_day
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn toggle_paused(&mut self) {
        self.paused = !self.paused;
    }

    /// Advance (or rewind) the clock by `delta` of a day. `delta = 1/24`
    /// equals one hour. Wraps within [0, 1).
    pub fn advance_time(&mut self, delta: f32) {
        let mut t = self.time_of_day + delta;
        t = t.rem_euclid(1.0);
        self.time_of_day = t;
        self.position = Point3::from_vec(self.compute_sun_direction() * SUN_DISTANCE);
    }

    /// (hours, minutes) in 24-hour clock representation.
    pub fn get_clock(&self) -> (u32, u32) {
        let total_minutes = (self.time_of_day * 24.0 * 60.0) as u32;
        ((total_minutes / 60) % 24, total_minutes % 60)
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
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, delta_time: f64) {
        if !self.paused && self.day_length_seconds > 0.0 {
            let dt_day = (delta_time as f32) / self.day_length_seconds;
            self.advance_time(dt_day);
        }

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
