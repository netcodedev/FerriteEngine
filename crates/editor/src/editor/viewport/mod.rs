//! Editor camera, click selection and translate/rotate/scale gizmos.

use cgmath::{
    Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, Rad, SquareMatrix, Vector3, Vector4,
};
use ferrite::core::{
    camera::{Camera, CameraController, Projection},
    entity::{component::camera_component::CameraComponent, Entity, EntityHandle},
    renderer::{
        light::light::{srgb_to_linear, Light, LightKind},
        line::Line,
        particles::{EmitterShape, ParticleEmitter},
    },
    scene::Scene,
};
use glfw::{Action, CursorMode, Key, MouseButton, WindowEvent};

use super::{layout::Rect, GizmoMode};

mod gizmo;
mod shapes;

use gizmo::*;
use shapes::*;

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
const UNIFORM_COLOR: Vector3<f32> = Vector3::new(0.85, 0.85, 0.85);
/// Handle index of the scale gizmo's centre cube (scales all axes).
const UNIFORM_HANDLE: usize = 3;
/// Half the side of a scale handle's cube, relative to the gizmo size.
const SCALE_CUBE: f32 = 0.04;
/// Smallest scale a drag leaves on an axis.
const MIN_SCALE: f32 = 0.001;
const CIRCLE_SEGMENTS: usize = 48;
/// Translate arrows relative to the rotate rings.
const ARROW_SCALE: f32 = 0.5;
const GIZMO_LINE_WIDTH: i32 = 3;
/// Size of a light's marker, relative to the gizmos.
const LIGHT_MARKER_SCALE: f32 = 0.12;
/// How close (in pixels) a click must be to a marker to select its entity.
const MARKER_PICK_RADIUS: f32 = 8.0;

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

    /// Move or turn the camera; what is `None` stays.
    pub fn set_pose(&mut self, position: Option<Point3<f32>>, yaw: Option<Deg<f32>>, pitch: Option<Deg<f32>>) {
        if let Some(position) = position {
            self.position = position;
        }
        if let Some(yaw) = yaw {
            self.yaw = Rad::from(yaw);
        }
        if let Some(pitch) = pitch {
            self.pitch = Rad::from(Deg(pitch.0.clamp(-89.0, 89.0)));
        }
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

    /// Which gizmo handle the ray touches, if any: an axis (0..3) or, for
    /// the scale gizmo, `UNIFORM_HANDLE`.
    fn hit_gizmo(
        &self,
        ray: &Ray,
        center: Point3<f32>,
        axes: [Vector3<f32>; 3],
        mode: GizmoMode,
    ) -> Option<usize> {
        let size = self.gizmo_size(center);
        if mode == GizmoMode::Scale
            && distance_to_ray(ray, center) < size * SCALE_CUBE * 2.0
        {
            return Some(UNIFORM_HANDLE);
        }
        let mut best: Option<(usize, f32)> = None;
        for (i, axis) in axes.iter().enumerate() {
            let error = match mode {
                GizmoMode::Translate | GizmoMode::Scale => {
                    let length = size * ARROW_SCALE;
                    let Some((_, s)) = closest_params(ray, center, *axis) else {
                        continue;
                    };
                    let point = center + axis * s.clamp(0.0, length);
                    distance_to_ray(ray, point) / (size * 0.07)
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

    /// Entity under the cursor: the closest of a physics shape hit (player
    /// capsule, terrain, trees → terrain), a hit on drawn geometry (models
    /// without a collider) and an entity marker within a few pixels of the
    /// cursor (entities that draw nothing, such as a camera or light).
    fn pick(&self, scene: &mut Scene, ray: &Ray, rect: Rect) -> Option<EntityHandle> {
        let mut best = scene.raycast_entity(ray.origin, ray.dir, 5000.0);
        if let Some((id, t)) = scene.raycast_components(ray.origin, ray.dir) {
            if best.map_or(true, |(_, bt)| t < bt) {
                best = Some((id, t));
            }
        }
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
                    let axes = gizmo_axes(scene, &id, gizmo)?;
                    let ray = self.ray(scene, rect, self.cursor.0, self.cursor.1)?;
                    self.hit_gizmo(&ray, center, axes, gizmo)
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
                if let (Some(id), Some(handle)) = (selected, self.hover_axis) {
                    let size = world_position(scene, &id).map_or(1.0, |c| self.gizmo_size(c));
                    if let Some(drag) = begin_drag(scene, id, handle, gizmo, &ray, size) {
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

    /// Draw the gizmo of the selection. Must run while the GL viewport is
    /// still set to the editor viewport.
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
        self.render_lights(scene, &vp, rect, selected);
        self.render_emitters(scene, &vp, rect, selected);

        let Some(id) = selected else {
            return;
        };
        let Some(center) = world_position(scene, &id) else {
            return;
        };
        let Some(axes) = gizmo_axes(scene, &id, gizmo) else {
            return;
        };
        let size = self.gizmo_size(center);
        let active = match &self.drag {
            Some(drag) => drag.handle(),
            None => self.hover_axis,
        };
        let color_of = |handle: usize, normal: Vector3<f32>| {
            if active == Some(handle) {
                HIGHLIGHT
            } else {
                normal
            }
        };
        for (i, axis) in axes.iter().enumerate() {
            let lines = match gizmo {
                GizmoMode::Translate => arrow_lines(center, *axis, size * ARROW_SCALE),
                GizmoMode::Rotate => circle_lines(center, *axis, size),
                GizmoMode::Scale => {
                    let length = size * ARROW_SCALE;
                    let mut lines = vec![Line::new(center, *axis, length)];
                    lines.extend(cube_lines(center + axis * length, axes, size * SCALE_CUBE));
                    lines
                }
            };
            render_thick_lines(&vp, &lines, color_of(i, AXIS_COLORS[i]), rect, GIZMO_LINE_WIDTH);
        }
        if gizmo == GizmoMode::Scale {
            let lines = cube_lines(center, axes, size * SCALE_CUBE);
            let color = color_of(UNIFORM_HANDLE, UNIFORM_COLOR);
            render_thick_lines(&vp, &lines, color, rect, GIZMO_LINE_WIDTH);
        }
    }
}

impl Viewport {
    /// A small star in its colour for every light, so lights can be seen
    /// and clicked; for the selected one also where it reaches: its range
    /// (point), its cone (spot) or its direction (directional).
    fn render_lights(
        &self,
        scene: &Scene,
        vp: &Matrix4<f32>,
        rect: Rect,
        selected: Option<EntityHandle>,
    ) {
        for (id, transform, light) in collect_lights(scene) {
            let s = light.settings();
            let center = Point3::from_vec(transform.w.truncate());
            let down = (transform * Vector4::new(0.0, -1.0, 0.0, 0.0)).truncate();
            let dir = if down.magnitude2() > 1e-12 {
                down.normalize()
            } else {
                -Vector3::unit_y()
            };
            // What the colour looks like on screen, but never so dark the
            // marker disappears; switched off lights are grey.
            let color = if s.enabled {
                let c = Vector3::from(s.color.map(srgb_to_linear));
                c / c.x.max(c.y).max(c.z).max(1e-3)
            } else {
                Vector3::new(0.35, 0.35, 0.35)
            };
            let size = self.gizmo_size(center) * LIGHT_MARKER_SCALE;
            let mut lines = star_lines(center, size);
            if s.kind != LightKind::Point {
                lines.extend(arrow_lines(center, dir, size * 3.0));
            }
            let is_selected = selected == Some(id);
            if is_selected {
                lines.extend(match s.kind {
                    LightKind::Point => AXES
                        .iter()
                        .flat_map(|axis| circle_lines(center, *axis, s.range))
                        .collect(),
                    LightKind::Spot => cone_lines(center, dir, s.range, s.outer_angle),
                    LightKind::Directional => arrow_lines(center, dir, self.gizmo_size(center) * 2.5),
                });
            }
            let width = if is_selected { 2 } else { 1 };
            render_thick_lines(vp, &lines, color, rect, width);
        }
    }
}

impl Viewport {
    /// A small diamond for every particle emitter, in the colour of its
    /// particles; for the selected one also its shape and direction.
    fn render_emitters(
        &self,
        scene: &Scene,
        vp: &Matrix4<f32>,
        rect: Rect,
        selected: Option<EntityHandle>,
    ) {
        for (id, transform, emitter) in collect_components::<ParticleEmitter>(scene) {
            let e = emitter.effect();
            let center = Point3::from_vec(transform.w.truncate());
            let [r, g, b, _] = e.color_middle;
            let c = Vector3::new(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
            let color = c / c.x.max(c.y).max(c.z).max(1e-3);
            let size = self.gizmo_size(center) * LIGHT_MARKER_SCALE;
            let mut lines = diamond_lines(center, size);
            let is_selected = selected == Some(id);
            if is_selected {
                let axis = |v: Vector4<f32>| (transform * v).truncate();
                let (x, y, z) = (
                    axis(Vector4::unit_x()),
                    axis(Vector4::unit_y()),
                    axis(Vector4::unit_z()),
                );
                match e.shape {
                    EmitterShape::Point => {}
                    EmitterShape::Sphere(radius) => {
                        for a in [x, y, z] {
                            if a.magnitude2() > 1e-12 {
                                lines.extend(circle_lines(center, a.normalize(), radius * a.magnitude()));
                            }
                        }
                    }
                    EmitterShape::Disc(radius) => {
                        if y.magnitude2() > 1e-12 {
                            lines.extend(circle_lines(center, y.normalize(), radius * x.magnitude()));
                        }
                    }
                    EmitterShape::Box([hx, hy, hz]) => {
                        lines.extend(box_lines(center, [x * hx, y * hy, z * hz]));
                    }
                }
                let d = axis(Vector3::from(e.direction).extend(0.0));
                if d.magnitude2() > 1e-12 {
                    lines.extend(arrow_lines(center, d.normalize(), self.gizmo_size(center) * 1.5));
                }
            }
            let width = if is_selected { 2 } else { 1 };
            render_thick_lines(vp, &lines, color, rect, width);
        }
    }
}

/// Every component of type `T` in the scene with its entity and world
/// transform.
fn collect_components<T: ferrite::core::entity::component::Component>(
    scene: &Scene,
) -> Vec<(EntityHandle, Matrix4<f32>, &T)> {
    fn visit<'a, T: ferrite::core::entity::component::Component>(
        entity: &'a Entity,
        parent: Matrix4<f32>,
        out: &mut Vec<(EntityHandle, Matrix4<f32>, &'a T)>,
    ) {
        if entity.is_transient() {
            return;
        }
        let world = parent * entity.local_transform();
        for component in entity.components() {
            if let Some(found) = component.as_any().downcast_ref::<T>() {
                out.push((entity.id, world, found));
            }
        }
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

/// Every light in the scene with its entity and world transform.
fn collect_lights(scene: &Scene) -> Vec<(EntityHandle, Matrix4<f32>, &Light)> {
    fn visit<'a>(
        entity: &'a Entity,
        parent: Matrix4<f32>,
        out: &mut Vec<(EntityHandle, Matrix4<f32>, &'a Light)>,
    ) {
        let world = parent * entity.local_transform();
        for component in entity.components() {
            if let Some(light) = component.as_any().downcast_ref::<Light>() {
                out.push((entity.id, world, light));
            }
        }
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
