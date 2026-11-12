//! Wireframe line geometry for physics colliders, used by the debug overlay
//! and by per-component "show collider" toggles in the editor.

use std::f32::consts::PI;

use cgmath::{InnerSpace, Point3, Vector3};
use rapier3d::prelude::{Collider, RigidBodyHandle, TypedShape};

use crate::core::renderer::line::Line;

use super::physics_engine::PhysicsEngine;

const CIRCLE_SEGMENTS: usize = 16;


pub(crate) fn seg(a: Point3<f32>, b: Point3<f32>) -> Line {
    let dir = b - a;
    let len = dir.magnitude();
    Line {
        position: a,
        direction: if len > 0.001 {
            dir / len
        } else {
            Vector3::new(0.0, 1.0, 0.0)
        },
        length: len,
    }
}

pub(crate) fn apply_pose(
    pos: &rapier3d::math::Vector,
    rot: &rapier3d::math::Rotation,
    local: Vector3<f32>,
) -> Point3<f32> {
    use cgmath::Rotation as _;
    let q = cgmath::Quaternion::new(rot.w, rot.x, rot.y, rot.z);
    let r = q.rotate_vector(local);
    Point3::new(pos.x + r.x, pos.y + r.y, pos.z + r.z)
}

fn perp_basis(axis: Vector3<f32>) -> (Vector3<f32>, Vector3<f32>) {
    let n = axis.normalize();
    let right = if n.x.abs() < 0.9 {
        n.cross(Vector3::new(1.0, 0.0, 0.0)).normalize()
    } else {
        n.cross(Vector3::new(0.0, 1.0, 0.0)).normalize()
    };
    (right, n.cross(right).normalize())
}

fn circle_lines(
    center: Point3<f32>,
    right: Vector3<f32>,
    up: Vector3<f32>,
    radius: f32,
) -> Vec<Line> {
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let a = 2.0 * PI * i as f32 / CIRCLE_SEGMENTS as f32;
            let b = 2.0 * PI * (i + 1) as f32 / CIRCLE_SEGMENTS as f32;
            let pa = center + right * (radius * a.cos()) + up * (radius * a.sin());
            let pb = center + right * (radius * b.cos()) + up * (radius * b.sin());
            seg(pa, pb)
        })
        .collect()
}

pub(crate) fn ball_wires(pos: &rapier3d::math::Vector, radius: f32) -> Vec<Line> {
    let c = Point3::new(pos.x, pos.y, pos.z);
    let mut lines = Vec::new();
    lines.extend(circle_lines(
        c,
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
        radius,
    ));
    lines.extend(circle_lines(
        c,
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        radius,
    ));
    lines.extend(circle_lines(
        c,
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(0.0, 1.0, 0.0),
        radius,
    ));
    lines
}

pub(crate) fn capsule_wires(a: Point3<f32>, b: Point3<f32>, r: f32) -> Vec<Line> {
    let axis = (b - a).normalize();
    let (right, up) = perp_basis(axis);
    let mid = a + (b - a) * 0.5;

    let mut lines = Vec::new();
    lines.extend(circle_lines(a, right, up, r));
    lines.extend(circle_lines(b, right, up, r));
    lines.extend(circle_lines(mid, right, up, r));
    for i in 0..4 {
        let angle = PI * 0.5 * i as f32;
        let offset = right * (r * angle.cos()) + up * (r * angle.sin());
        lines.push(seg(a + offset, b + offset));
    }
    lines
}


fn cuboid_wires(
    pos: &rapier3d::math::Vector,
    rot: &rapier3d::math::Rotation,
    half: Vector3<f32>,
) -> Vec<Line> {
    let corner = |i: usize| {
        let sign = |bit: usize| if i & bit != 0 { 1.0 } else { -1.0 };
        apply_pose(
            pos,
            rot,
            Vector3::new(half.x * sign(1), half.y * sign(2), half.z * sign(4)),
        )
    };
    let mut lines = Vec::new();
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                lines.push(seg(corner(i), corner(i | bit)));
            }
        }
    }
    lines
}

fn cylinder_wires(
    pos: &rapier3d::math::Vector,
    rot: &rapier3d::math::Rotation,
    half_height: f32,
    radius: f32,
) -> Vec<Line> {
    let a = apply_pose(pos, rot, Vector3::new(0.0, -half_height, 0.0));
    let b = apply_pose(pos, rot, Vector3::new(0.0, half_height, 0.0));
    let axis = b - a;
    if axis.magnitude2() < 1e-8 {
        return Vec::new();
    }
    let (right, up) = perp_basis(axis);
    let mut lines = circle_lines(a, right, up, radius);
    lines.extend(circle_lines(b, right, up, radius));
    for i in 0..4 {
        let angle = PI * 0.5 * i as f32;
        let offset = right * (radius * angle.cos()) + up * (radius * angle.sin());
        lines.push(seg(a + offset, b + offset));
    }
    lines
}

/// Wireframe of one collider. Primitive shapes go to `shapes`; triangle
/// meshes go to `meshes`, limited to triangles within `mesh_distance` of
/// `camera` because terrain meshes are far too dense to draw entirely.
pub fn collider_wires(
    collider: &Collider,
    camera: Point3<f32>,
    mesh_distance: f32,
    shapes: &mut Vec<Line>,
    meshes: &mut Vec<Line>,
) {
    let pos = collider.translation();
    let rot = collider.rotation();
    match collider.shape().as_typed_shape() {
        TypedShape::Ball(ball) => shapes.extend(ball_wires(&pos, ball.radius)),
        TypedShape::Capsule(capsule) => {
            let sa = capsule.segment.a;
            let sb = capsule.segment.b;
            let a = apply_pose(&pos, &rot, Vector3::new(sa.x, sa.y, sa.z));
            let b = apply_pose(&pos, &rot, Vector3::new(sb.x, sb.y, sb.z));
            shapes.extend(capsule_wires(a, b, capsule.radius));
        }
        TypedShape::Cuboid(cuboid) => {
            let h = cuboid.half_extents;
            shapes.extend(cuboid_wires(&pos, &rot, Vector3::new(h.x, h.y, h.z)));
        }
        TypedShape::Cylinder(cylinder) => {
            shapes.extend(cylinder_wires(&pos, &rot, cylinder.half_height, cylinder.radius));
        }
        TypedShape::ConvexPolyhedron(poly) => {
            let pts = poly.points();
            for edge in poly.edges() {
                let p0 = pts[edge.vertices[0] as usize];
                let p1 = pts[edge.vertices[1] as usize];
                shapes.push(seg(
                    apply_pose(&pos, &rot, Vector3::new(p0.x, p0.y, p0.z)),
                    apply_pose(&pos, &rot, Vector3::new(p1.x, p1.y, p1.z)),
                ));
            }
        }
        TypedShape::TriMesh(mesh) => {
            let aabb = collider.compute_aabb();
            let nearest = Point3::new(
                camera.x.clamp(aabb.mins.x, aabb.maxs.x),
                camera.y.clamp(aabb.mins.y, aabb.maxs.y),
                camera.z.clamp(aabb.mins.z, aabb.maxs.z),
            );
            if (nearest - camera).magnitude() > mesh_distance {
                return;
            }
            for tri in mesh.triangles() {
                let va = apply_pose(&pos, &rot, Vector3::new(tri.a.x, tri.a.y, tri.a.z));
                let vb = apply_pose(&pos, &rot, Vector3::new(tri.b.x, tri.b.y, tri.b.z));
                let vc = apply_pose(&pos, &rot, Vector3::new(tri.c.x, tri.c.y, tri.c.z));
                let centroid = Point3::new(
                    (va.x + vb.x + vc.x) / 3.0,
                    (va.y + vb.y + vc.y) / 3.0,
                    (va.z + vb.z + vc.z) / 3.0,
                );
                if (centroid - camera).magnitude() > mesh_distance {
                    continue;
                }
                meshes.push(seg(va, vb));
                meshes.push(seg(vb, vc));
                meshes.push(seg(vc, va));
            }
        }
        _ => {}
    }
}

/// Wireframes of every collider attached to `bodies`.
pub fn body_wires(
    physics: &PhysicsEngine,
    bodies: &[RigidBodyHandle],
    camera: Point3<f32>,
    mesh_distance: f32,
    shapes: &mut Vec<Line>,
    meshes: &mut Vec<Line>,
) {
    for body in bodies {
        let Some(rb) = physics.rigid_bodies.get(*body) else {
            continue;
        };
        for handle in rb.colliders() {
            if let Some(collider) = physics.colliders.get(*handle) {
                collider_wires(collider, camera, mesh_distance, shapes, meshes);
            }
        }
    }
}
