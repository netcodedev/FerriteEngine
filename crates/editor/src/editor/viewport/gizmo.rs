use cgmath::{
    EuclideanSpace, InnerSpace, Matrix3, Matrix4, Point3, Quaternion, Rad, Rotation, Rotation3,
    SquareMatrix, Vector3,
};
use ferrite::core::{entity::EntityHandle, scene::Scene};

use crate::editor::GizmoMode;

use super::*;

pub(super) struct Ray {
    pub(super) origin: Point3<f32>,
    pub(super) dir: Vector3<f32>,
}

pub(super) enum Drag {
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
    Scale {
        id: EntityHandle,
        /// 0..3: a local axis, `UNIFORM_HANDLE`: all of them.
        handle: usize,
        /// World direction the drag is measured along.
        axis: Vector3<f32>,
        center: Point3<f32>,
        /// Where the drag started: the parameter along `axis` for an axis
        /// handle, the hit on the view plane for the uniform one.
        start_param: f32,
        start_hit: Point3<f32>,
        /// Normal of the plane the uniform handle is dragged on.
        plane_normal: Vector3<f32>,
        start_scale: Vector3<f32>,
        /// Dragging this far doubles the scale.
        length: f32,
    },
}

impl Drag {
    /// Index of the handle being dragged, for highlighting.
    pub(super) fn handle(&self) -> Option<usize> {
        match self {
            Drag::Translate { axis, .. } | Drag::Rotate { axis, .. } => {
                AXES.iter().position(|a| a == axis)
            }
            Drag::Scale { handle, .. } => Some(*handle),
        }
    }
}

/// Orthonormal `(u, v)` with `u × v = axis`.
pub(super) fn perpendicular_basis(axis: Vector3<f32>) -> (Vector3<f32>, Vector3<f32>) {
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
pub(super) fn closest_params(ray: &Ray, origin: Point3<f32>, axis: Vector3<f32>) -> Option<(f32, f32)> {
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

pub(super) fn distance_to_ray(ray: &Ray, point: Point3<f32>) -> f32 {
    let t = (point - ray.origin).dot(ray.dir).max(0.0);
    (ray.origin + ray.dir * t - point).magnitude()
}

/// Directions of the gizmo's axes: the world axes for moving and rotating,
/// the entity's own (rotated) axes for scaling, which acts on those.
pub(super) fn gizmo_axes(scene: &Scene, id: &EntityHandle, mode: GizmoMode) -> Option<[Vector3<f32>; 3]> {
    if mode != GizmoMode::Scale {
        return Some(AXES);
    }
    let rotation = rotation_of(&scene.get_world_transform(id)?);
    Some(AXES.map(|axis| rotation.rotate_vector(axis)))
}

pub(super) fn ray_plane(ray: &Ray, center: Point3<f32>, normal: Vector3<f32>) -> Option<Point3<f32>> {
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

pub(super) fn begin_drag(
    scene: &Scene,
    id: EntityHandle,
    handle: usize,
    gizmo: GizmoMode,
    ray: &Ray,
    size: f32,
) -> Option<Drag> {
    if gizmo == GizmoMode::Scale {
        return begin_scale(scene, id, handle, ray, size);
    }
    let axis = AXES[handle];
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
        GizmoMode::Scale => None,
    }
}

fn begin_scale(
    scene: &Scene,
    id: EntityHandle,
    handle: usize,
    ray: &Ray,
    size: f32,
) -> Option<Drag> {
    let center = world_position(scene, &id)?;
    let start_scale = scene.get_entity(&id)?.get_scale();
    let length = size * ARROW_SCALE;
    if handle == UNIFORM_HANDLE {
        // Measured on the plane facing the camera, along its up-right
        // diagonal: dragging up or right grows, down or left shrinks.
        let normal = -ray.dir;
        let start_hit = ray_plane(ray, center, normal)?;
        let right = Vector3::unit_y().cross(normal);
        let right = if right.magnitude2() > 1e-6 {
            right.normalize()
        } else {
            Vector3::unit_x()
        };
        let up = normal.cross(right).normalize();
        return Some(Drag::Scale {
            id,
            handle,
            axis: (right + up).normalize(),
            center,
            start_param: 0.0,
            start_hit,
            plane_normal: normal,
            start_scale,
            length,
        });
    }
    let axis = gizmo_axes(scene, &id, GizmoMode::Scale)?[handle];
    let (_, s) = closest_params(ray, center, axis)?;
    Some(Drag::Scale {
        id,
        handle,
        axis,
        center,
        start_param: s,
        start_hit: center,
        plane_normal: axis,
        start_scale,
        length,
    })
}

pub(super) fn apply_drag(scene: &mut Scene, drag: &Drag, ray: &Ray) {
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
        Drag::Scale {
            id,
            handle,
            axis,
            center,
            start_param,
            start_hit,
            plane_normal,
            start_scale,
            length,
        } => {
            let moved = if *handle == UNIFORM_HANDLE {
                let Some(hit) = ray_plane(ray, *center, *plane_normal) else {
                    return;
                };
                (hit - start_hit).dot(*axis)
            } else {
                let Some((_, s)) = closest_params(ray, *center, *axis) else {
                    return;
                };
                s - start_param
            };
            let factor = ((length + moved) / length).max(MIN_SCALE);
            let mut scale = *start_scale;
            if *handle == UNIFORM_HANDLE {
                scale *= factor;
            } else {
                scale[*handle] *= factor;
            }
            scale = scale.map(|c| c.max(MIN_SCALE));
            scene.set_entity_scale(id, scale);
        }
    }
}
