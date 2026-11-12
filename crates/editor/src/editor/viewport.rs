//! Editor camera, click selection and translate/rotate gizmos.

use cgmath::{
    Deg, EuclideanSpace, InnerSpace, Matrix3, Matrix4, Point3, Quaternion, Rad, Rotation,
    Rotation3, SquareMatrix, Vector3, Vector4,
};
use ferrite::core::{
    camera::{Camera, CameraController, Projection},
    entity::{component::camera_component::CameraComponent, Entity, EntityHandle},
    renderer::line::{Line, LineRenderer},
    scene::Scene,
};
use glfw::{Action, CursorMode, Key, MouseButton, WindowEvent};

use super::{layout::Rect, outline, GizmoMode};

const AXES: [Vector3<f32>; 3] = [
    Vector3::new(1.0, 0.0, 0.0),
    Vector3::new(0.0, 1.0, 0.0),
    Vector3::new(0.0, 0.0, 1.0),
];
const AXIS_COLORS: [Vector3<f32>; 3] = [
    Vector3::new(0.95, 0.25, 0.25),
    Vector3::new(0.3, 0.9, 0.3),
    Vector3::new(0.3, 0.5, 1.0),
];
const HIGHLIGHT: Vector3<f32> = Vector3::new(1.0, 0.85, 0.2);
const CIRCLE_SEGMENTS: usize = 48;
/// Translate arrows relative to the rotate rings.
const ARROW_SCALE: f32 = 0.5;
const GIZMO_LINE_WIDTH: i32 = 3;
/// How close (in pixels) a click must be to a marker to select its entity.
const MARKER_PICK_RADIUS: f32 = 8.0;

struct Ray {
    origin: Point3<f32>,
    dir: Vector3<f32>,
}

enum Drag {
    Translate {
        id: EntityHandle,
        axis: Vector3<f32>,
        start_world: Point3<f32>,
        start_param: f32,
        parent_inverse: Matrix4<f32>,
    },
    Rotate {
        id: EntityHandle,
        axis: Vector3<f32>,
        center: Point3<f32>,
        start_angle: f32,
        start_local: Quaternion<f32>,
        parent_rotation: Quaternion<f32>,
    },
}

/// Result of a viewport event, applied by the editor layer.
pub enum ViewportAction {
    None,
    /// The event was used (camera, gizmo hover) but changes nothing else.
    Consumed,
    Select(Option<EntityHandle>),
    /// A gizmo drag modified the scene.
    Edited,
}

pub struct Viewport {
    position: Point3<f32>,
    yaw: Rad<f32>,
    pitch: Rad<f32>,
    fov: Deg<f32>,
    speed: f32,

    flying: bool,
    last_cursor: Option<(f64, f64)>,
    /// forward, back, left, right, up, down
    keys: [bool; 6],
    fast: bool,

    cursor: (f32, f32),
    hover_axis: Option<usize>,
    drag: Option<Drag>,
}

impl Viewport {
    pub fn new() -> Self {
        Self {
            position: Point3::new(-30.0, 75.0, -30.0),
            yaw: Rad::from(Deg(45.0)),
            pitch: Rad::from(Deg(-30.0)),
            fov: Deg(64.0),
            speed: 20.0,
            flying: false,
            last_cursor: None,
            keys: [false; 6],
            fast: false,
            cursor: (0.0, 0.0),
            hover_axis: None,
            drag: None,
        }
    }

    pub fn is_flying(&self) -> bool {
        self.flying
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn forward(&self) -> Vector3<f32> {
        let (sp, cp) = self.pitch.0.sin_cos();
        let (sy, cy) = self.yaw.0.sin_cos();
        Vector3::new(cp * cy, sp, cp * sy).normalize()
    }

    /// Move the fly camera and push it into the scene as the editor camera.
    pub fn update(&mut self, scene: &mut Scene, rect: Rect, delta_time: f64) {
        if self.flying {
            let forward = self.forward();
            let right = forward.cross(Vector3::unit_y()).normalize();
            let mut motion = Vector3::new(0.0, 0.0, 0.0);
            let axis = |on: bool| if on { 1.0 } else { 0.0 };
            motion += forward * (axis(self.keys[0]) - axis(self.keys[1]));
            motion += right * (axis(self.keys[3]) - axis(self.keys[2]));
            motion += Vector3::unit_y() * (axis(self.keys[4]) - axis(self.keys[5]));
            if motion.magnitude2() > 0.0 {
                let speed = self.speed * if self.fast { 4.0 } else { 1.0 };
                self.position += motion.normalize() * speed * delta_time as f32;
            }
        }

        let aspect = rect.w / rect.h.max(1.0);
        if scene.get_editor_camera_mut().is_none() {
            let camera = Camera::new(self.position, self.yaw, self.pitch);
            let projection = Projection::new(
                rect.w.max(1.0) as u32,
                rect.h.max(1.0) as u32,
                self.fov,
                0.1,
                2000.0,
            );
            scene.set_editor_camera(Some(CameraComponent::new(
                camera,
                projection,
                CameraController::new(self.speed, 1.0),
            )));
        }
        if let Some(editor_camera) = scene.get_editor_camera_mut() {
            let camera = editor_camera.get_camera_mut();
            camera.set_relative_position(Point3::origin());
            camera.set_position(self.position);
            camera.set_yaw_pitch(self.yaw, self.pitch);
            editor_camera.get_projection_mut().set_aspect(aspect);
        }
    }

    /// Put the camera a few metres in front of `id`, looking at it.
    pub fn focus(&mut self, scene: &Scene, id: EntityHandle) {
        let Some(target) = world_position(scene, &id) else {
            return;
        };
        let distance = 12.0;
        self.position = target - self.forward() * distance;
    }

    /// Point a little in front of the camera, used to place new entities.
    pub fn spawn_point(&self) -> Point3<f32> {
        self.position + self.forward() * 15.0
    }

    pub fn camera_pose(&self) -> (Point3<f32>, Deg<f32>, Deg<f32>) {
        (self.position, Deg::from(self.yaw), Deg::from(self.pitch))
    }

    /// Cancel camera flight and gizmo drags (e.g. when entering play mode).
    pub fn reset_interaction(&mut self, window: &mut glfw::Window) {
        if self.flying {
            window.set_cursor_mode(CursorMode::Normal);
        }
        self.flying = false;
        self.keys = [false; 6];
        self.drag = None;
        self.hover_axis = None;
        self.last_cursor = None;
    }

    fn ray(&self, scene: &Scene, rect: Rect, x: f32, y: f32) -> Option<Ray> {
        let vp = scene.get_active_camera()?.get_view_projection();
        let inverse = vp.invert()?;
        let nx = (x - rect.x) / rect.w * 2.0 - 1.0;
        let ny = 1.0 - (y - rect.y) / rect.h * 2.0;
        let near = inverse * Vector4::new(nx, ny, -1.0, 1.0);
        let far = inverse * Vector4::new(nx, ny, 1.0, 1.0);
        let near = Point3::from_homogeneous(near);
        let far = Point3::from_homogeneous(far);
        Some(Ray {
            origin: near,
            dir: (far - near).normalize(),
        })
    }

    fn gizmo_size(&self, center: Point3<f32>) -> f32 {
        (center - self.position).magnitude().max(1.0) * 0.16
    }

    /// Which gizmo axis the ray touches, if any.
    fn hit_gizmo(&self, ray: &Ray, center: Point3<f32>, mode: GizmoMode) -> Option<usize> {
        let size = self.gizmo_size(center);
        let mut best: Option<(usize, f32)> = None;
        for (i, axis) in AXES.iter().enumerate() {
            let error = match mode {
                GizmoMode::Translate => {
                    let length = size * ARROW_SCALE;
                    let (_, s) = closest_params(ray, center, *axis)?;
                    let s = s.clamp(0.0, length);
                    let point = center + axis * s;
                    let t = (point - ray.origin).dot(ray.dir).max(0.0);
                    let on_ray = ray.origin + ray.dir * t;
                    (on_ray - point).magnitude() / (size * 0.07)
                }
                GizmoMode::Rotate => {
                    let Some(hit) = ray_plane(ray, center, *axis) else {
                        continue;
                    };
                    ((hit - center).magnitude() - size).abs() / (size * 0.1)
                }
            };
            if error < 1.0 && best.map_or(true, |(_, e)| error < e) {
                best = Some((i, error));
            }
        }
        best.map(|(i, _)| i)
    }

    /// Entity under the cursor: the closest physics shape hit (player
    /// capsule, terrain, trees → terrain), or an entity marker within a few
    /// pixels of the cursor for entities without a shape (camera, light).
    fn pick(&self, scene: &mut Scene, ray: &Ray, rect: Rect) -> Option<EntityHandle> {
        let mut best = scene.raycast_entity(ray.origin, ray.dir, 5000.0);
        let vp = scene.get_active_camera()?.get_view_projection();
        for (id, position) in collect_world_positions(scene) {
            let Some((sx, sy)) = project(&vp, rect, position) else {
                continue;
            };
            let (dx, dy) = (sx - self.cursor.0, sy - self.cursor.1);
            if dx * dx + dy * dy > MARKER_PICK_RADIUS * MARKER_PICK_RADIUS {
                continue;
            }
            let t = (position - ray.origin).dot(ray.dir);
            if t > 0.0 && best.map_or(true, |(_, bt)| t < bt) {
                best = Some((id, t));
            }
        }
        best.map(|(id, _)| id)
    }

    pub fn handle_event(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        event: &WindowEvent,
        rect: Rect,
        selected: Option<EntityHandle>,
        gizmo: GizmoMode,
    ) -> ViewportAction {
        match event {
            WindowEvent::MouseButton(MouseButton::Button2, Action::Press, _) => {
                if rect.contains(self.cursor.0, self.cursor.1) && self.drag.is_none() {
                    self.flying = true;
                    self.last_cursor = None;
                    window.set_cursor_mode(CursorMode::Disabled);
                    return ViewportAction::Consumed;
                }
                ViewportAction::None
            }
            WindowEvent::MouseButton(MouseButton::Button2, Action::Release, _) => {
                if self.flying {
                    self.flying = false;
                    self.keys = [false; 6];
                    // No set_cursor_pos here: Wayland only allows it while the
                    // cursor is disabled, and glfw 0.62 aborts on the
                    // resulting FEATURE_UNAVAILABLE error code.
                    window.set_cursor_mode(CursorMode::Normal);
                    return ViewportAction::Consumed;
                }
                ViewportAction::None
            }
            WindowEvent::CursorPos(x, y) => {
                if self.flying {
                    if let Some((lx, ly)) = self.last_cursor {
                        let sensitivity = 0.003;
                        self.yaw += Rad((*x - lx) as f32 * sensitivity);
                        let pitch = self.pitch.0 - (*y - ly) as f32 * sensitivity;
                        self.pitch = Rad(pitch.clamp(-1.55, 1.55));
                    }
                    self.last_cursor = Some((*x, *y));
                    return ViewportAction::Consumed;
                }
                self.cursor = (*x as f32, *y as f32);
                if let Some(drag) = &self.drag {
                    let Some(ray) = self.ray(scene, rect, self.cursor.0, self.cursor.1) else {
                        return ViewportAction::Consumed;
                    };
                    apply_drag(scene, drag, &ray);
                    return ViewportAction::Edited;
                }
                self.hover_axis = selected.and_then(|id| {
                    let center = world_position(scene, &id)?;
                    let ray = self.ray(scene, rect, self.cursor.0, self.cursor.1)?;
                    self.hit_gizmo(&ray, center, gizmo)
                });
                ViewportAction::None
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                if !rect.contains(self.cursor.0, self.cursor.1) || self.flying {
                    return ViewportAction::None;
                }
                let Some(ray) = self.ray(scene, rect, self.cursor.0, self.cursor.1) else {
                    return ViewportAction::Consumed;
                };
                if let (Some(id), Some(axis)) = (selected, self.hover_axis) {
                    if let Some(drag) = begin_drag(scene, id, axis, gizmo, &ray) {
                        self.drag = Some(drag);
                        return ViewportAction::Consumed;
                    }
                }
                ViewportAction::Select(self.pick(scene, &ray, rect))
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                if self.drag.take().is_some() {
                    return ViewportAction::Edited;
                }
                ViewportAction::None
            }
            WindowEvent::Scroll(_, dy) => {
                if !rect.contains(self.cursor.0, self.cursor.1) {
                    return ViewportAction::None;
                }
                if self.flying {
                    self.speed = (self.speed * (1.0 + *dy as f32 * 0.15)).clamp(1.0, 500.0);
                } else {
                    let step = self.speed * 0.25 * *dy as f32;
                    self.position += self.forward() * step;
                }
                ViewportAction::Consumed
            }
            WindowEvent::Key(key, _, action, modifiers) if self.flying => {
                let pressed = *action != Action::Release;
                self.fast = modifiers.contains(glfw::Modifiers::Shift);
                let slot = match key {
                    Key::W => 0,
                    Key::S => 1,
                    Key::A => 2,
                    Key::D => 3,
                    Key::E => 4,
                    Key::Q => 5,
                    Key::LeftShift | Key::RightShift => {
                        self.fast = pressed;
                        return ViewportAction::Consumed;
                    }
                    _ => return ViewportAction::None,
                };
                self.keys[slot] = pressed;
                ViewportAction::Consumed
            }
            _ => ViewportAction::None,
        }
    }

    /// Draw entity markers and the gizmo of the selection. Must run while the
    /// GL viewport is still set to the editor viewport.
    pub fn render(
        &self,
        scene: &Scene,
        rect: Rect,
        selected: Option<EntityHandle>,
        gizmo: GizmoMode,
    ) {
        let Some(camera) = scene.get_active_camera() else {
            return;
        };
        let vp = camera.get_view_projection();

        // The selected entity's marker is highlighted: entities without
        // geometry (camera, light) have no outline to show the selection.
        let mut markers = Vec::new();
        let mut selected_marker = Vec::new();
        for (id, position) in collect_world_positions(scene) {
            let size = (position - self.position).magnitude() * 0.012;
            let target = if Some(id) == selected {
                &mut selected_marker
            } else {
                &mut markers
            };
            for axis in AXES {
                target.push(Line::new(position - axis * size, axis, size * 2.0));
            }
        }
        if !markers.is_empty() {
            LineRenderer::render_lines(&vp, &markers, Vector3::new(0.75, 0.75, 0.75), true);
        }
        if !selected_marker.is_empty() {
            LineRenderer::render_lines(&vp, &selected_marker, outline::OUTLINE_COLOR, true);
        }

        let Some(id) = selected else {
            return;
        };
        let Some(center) = world_position(scene, &id) else {
            return;
        };
        let size = self.gizmo_size(center);
        let active_axis = match &self.drag {
            Some(Drag::Translate { axis, .. }) | Some(Drag::Rotate { axis, .. }) => {
                AXES.iter().position(|a| a == axis)
            }
            None => self.hover_axis,
        };
        for (i, axis) in AXES.iter().enumerate() {
            let color = if active_axis == Some(i) {
                HIGHLIGHT
            } else {
                AXIS_COLORS[i]
            };
            let lines = match gizmo {
                GizmoMode::Translate => arrow_lines(center, *axis, size * ARROW_SCALE),
                GizmoMode::Rotate => circle_lines(center, *axis, size),
            };
            render_thick_lines(&vp, &lines, color, rect, GIZMO_LINE_WIDTH);
        }
    }
}

/// Draw `lines` `width` pixels thick by repeating them shifted by whole
/// pixels in screen space. Core-profile GL only guarantees 1 px wide lines,
/// so `glLineWidth` is not reliable.
fn render_thick_lines(
    vp: &Matrix4<f32>,
    lines: &Vec<Line>,
    color: Vector3<f32>,
    rect: Rect,
    width: i32,
) {
    let half = width / 2;
    for dy in -half..=half {
        for dx in -half..=half {
            // A translation applied after the projection adds `t * w` to
            // clip space, i.e. shifts NDC by exactly `t`.
            let shift = Matrix4::from_translation(Vector3::new(
                dx as f32 * 2.0 / rect.w.max(1.0),
                dy as f32 * 2.0 / rect.h.max(1.0),
                0.0,
            ));
            LineRenderer::render_lines(&(shift * vp), lines, color, true);
        }
    }
}

fn arrow_lines(center: Point3<f32>, axis: Vector3<f32>, size: f32) -> Vec<Line> {
    let tip = center + axis * size;
    let (u, v) = perpendicular_basis(axis);
    let head = size * 0.15;
    let mut lines = vec![Line::new(center, axis, size)];
    for side in [u, -u, v, -v] {
        let start = tip - axis * head + side * head * 0.4;
        let dir = tip - start;
        lines.push(Line::new(start, dir.normalize(), dir.magnitude()));
    }
    lines
}

fn circle_lines(center: Point3<f32>, axis: Vector3<f32>, radius: f32) -> Vec<Line> {
    let (u, v) = perpendicular_basis(axis);
    let point = |i: usize| {
        let a = i as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
        center + (u * a.cos() + v * a.sin()) * radius
    };
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let (a, b) = (point(i), point(i + 1));
            Line::new(a, (b - a).normalize(), (b - a).magnitude())
        })
        .collect()
}

/// Orthonormal `(u, v)` with `u × v = axis`.
fn perpendicular_basis(axis: Vector3<f32>) -> (Vector3<f32>, Vector3<f32>) {
    let helper = if axis.x.abs() < 0.9 {
        Vector3::unit_x()
    } else {
        Vector3::unit_y()
    };
    let u = axis.cross(helper).normalize();
    let v = axis.cross(u).normalize();
    (u, v)
}

/// Closest-point parameters `(t on ray, s on axis line)`; `None` when the
/// ray is (nearly) parallel to the axis.
fn closest_params(ray: &Ray, origin: Point3<f32>, axis: Vector3<f32>) -> Option<(f32, f32)> {
    let w0 = ray.origin - origin;
    let b = ray.dir.dot(axis);
    let d = ray.dir.dot(w0);
    let e = axis.dot(w0);
    let denom: f32 = 1.0 - b * b;
    if denom.abs() < 1e-5 {
        return None;
    }
    Some(((b * e - d) / denom, (e - b * d) / denom))
}

fn ray_plane(ray: &Ray, center: Point3<f32>, normal: Vector3<f32>) -> Option<Point3<f32>> {
    let denom = ray.dir.dot(normal);
    if denom.abs() < 1e-4 {
        return None;
    }
    let t = (center - ray.origin).dot(normal) / denom;
    (t > 0.0).then(|| ray.origin + ray.dir * t)
}

fn angle_around(axis: Vector3<f32>, center: Point3<f32>, point: Point3<f32>) -> f32 {
    let (u, v) = perpendicular_basis(axis);
    let d = point - center;
    d.dot(v).atan2(d.dot(u))
}

fn rotation_of(matrix: &Matrix4<f32>) -> Quaternion<f32> {
    let m = Matrix3::from_cols(
        matrix.x.truncate().normalize(),
        matrix.y.truncate().normalize(),
        matrix.z.truncate().normalize(),
    );
    Quaternion::from(m)
}

fn begin_drag(
    scene: &Scene,
    id: EntityHandle,
    axis_index: usize,
    gizmo: GizmoMode,
    ray: &Ray,
) -> Option<Drag> {
    let axis = AXES[axis_index];
    let world = scene.get_world_transform(&id)?;
    let parent = scene.get_parent_world_transform(&id)?;
    let center = Point3::from_vec(world.w.truncate());
    match gizmo {
        GizmoMode::Translate => {
            let (_, s) = closest_params(ray, center, axis)?;
            Some(Drag::Translate {
                id,
                axis,
                start_world: center,
                start_param: s,
                parent_inverse: parent.invert()?,
            })
        }
        GizmoMode::Rotate => {
            let hit = ray_plane(ray, center, axis)?;
            Some(Drag::Rotate {
                id,
                axis,
                center,
                start_angle: angle_around(axis, center, hit),
                start_local: scene.get_entity(&id)?.get_rotation(),
                parent_rotation: rotation_of(&parent),
            })
        }
    }
}

fn apply_drag(scene: &mut Scene, drag: &Drag, ray: &Ray) {
    match drag {
        Drag::Translate {
            id,
            axis,
            start_world,
            start_param,
            parent_inverse,
        } => {
            let Some((_, s)) = closest_params(ray, *start_world, *axis) else {
                return;
            };
            // `s` is measured from the start position along the axis; the
            // difference to where the axis was grabbed is the movement.
            let world = start_world + axis * (s - start_param);
            let local = parent_inverse * world.to_homogeneous();
            scene.set_entity_position(id, Point3::from_homogeneous(local));
        }
        Drag::Rotate {
            id,
            axis,
            center,
            start_angle,
            start_local,
            parent_rotation,
        } => {
            let Some(hit) = ray_plane(ray, *center, *axis) else {
                return;
            };
            let delta = angle_around(*axis, *center, hit) - start_angle;
            let world_delta = Quaternion::from_axis_angle(*axis, Rad(delta));
            let local = parent_rotation.invert() * world_delta * parent_rotation * start_local;
            scene.set_entity_rotation(id, local.normalize());
        }
    }
}

/// Window-pixel position of a world point inside the viewport rect, or
/// `None` when it is behind the camera.
fn project(vp: &Matrix4<f32>, rect: Rect, point: Point3<f32>) -> Option<(f32, f32)> {
    let clip = vp * point.to_homogeneous();
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some((
        rect.x + (ndc.x + 1.0) * 0.5 * rect.w,
        rect.y + (1.0 - ndc.y) * 0.5 * rect.h,
    ))
}

fn world_position(scene: &Scene, id: &EntityHandle) -> Option<Point3<f32>> {
    let world = scene.get_world_transform(id)?;
    Some(Point3::from_vec(world.w.truncate()))
}

/// World position of every persistent entity, computed in one pass.
fn collect_world_positions(scene: &Scene) -> Vec<(EntityHandle, Point3<f32>)> {
    fn visit(entity: &Entity, parent: Matrix4<f32>, out: &mut Vec<(EntityHandle, Point3<f32>)>) {
        if entity.is_transient() {
            return;
        }
        let world = parent * entity.local_transform();
        out.push((entity.id, Point3::from_vec(world.w.truncate())));
        for child in entity.get_children() {
            visit(child, world, out);
        }
    }
    let mut out = Vec::new();
    for entity in scene.get_entities() {
        visit(entity, Matrix4::identity(), &mut out);
    }
    out
}
