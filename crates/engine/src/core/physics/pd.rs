//! Driving a rigid body towards a pose with a spring and a damper (a PD
//! controller): what holds a carried object in front of a player, and
//! could hold lashed cargo. The push is a force with a limit, so a heavy
//! body sags under its weight, lags behind and swings; one too heavy for
//! the limit is not lifted at all, and two drives on one body add up (two
//! players carry together).
//!
//! Call once per frame before the physics step: the push goes in as an
//! impulse for the frame's step.

use cgmath::{InnerSpace, Point3, Quaternion, Rotation, Vector3};
use rapier3d::prelude::RigidBodyHandle;

use super::physics_engine::PhysicsEngine;

/// How a drive pulls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdGains {
    /// Pull per metre off the target (N/m): the same for every body, so a
    /// heavier one hangs lower (its weight over this) and follows slower.
    pub stiffness: f32,
    /// Damping of the speed relative to the target's, per kilogram (1/s).
    pub damping: f32,
    /// Turn per radian off the target's rotation, per kg m² (1/s²).
    pub angular_stiffness: f32,
    pub angular_damping: f32,
    /// The most force (N) the drive gives, its weight's share included.
    pub max_force: f32,
    /// The most torque (N m).
    pub max_torque: f32,
}

impl Default for PdGains {
    fn default() -> Self {
        Self {
            stiffness: 2000.0,
            damping: 28.0,
            angular_stiffness: 160.0,
            angular_damping: 22.0,
            max_force: 600.0,
            max_torque: 120.0,
        }
    }
}

/// What a drive did in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PdResult {
    /// The force it gave (N), at most `max_force`.
    pub force: f32,
    /// Whether it wanted more than `max_force` (the body is too heavy, or
    /// pulled away).
    pub strained: bool,
}

/// Push `body` towards `position` and `rotation` (`rotation`: `None`
/// leaves it free to turn) for a step of `dt` seconds. `velocity` and
/// `spin` (angular velocity, radians per second about each axis): the
/// target's own (it moves and turns with the carrier), so a body keeps up
/// with a moving target instead of being pulled after it. The weight hangs
/// on the spring: a body comes to rest `mass × g / stiffness` below the
/// target.
pub fn drive(
    physics: &mut PhysicsEngine,
    body: RigidBodyHandle,
    position: Point3<f32>,
    rotation: Option<Quaternion<f32>>,
    velocity: Vector3<f32>,
    spin: Vector3<f32>,
    gains: &PdGains,
    dt: f32,
) -> PdResult {
    let Some(rb) = physics.rigid_bodies.get_mut(body) else {
        return PdResult::default();
    };
    let mass = rb.mass().max(1e-3);
    let t = rb.translation();
    let v = rb.linvel();
    let (here, speed) = (Point3::new(t.x, t.y, t.z), Vector3::new(v.x, v.y, v.z));
    let wanted = (position - here) * gains.stiffness + (velocity - speed) * (gains.damping * mass);
    let (force, strained) = clamp(wanted, gains.max_force);
    if dt > 0.0 {
        rb.apply_impulse(rapier3d::math::Vector::new(force.x * dt, force.y * dt, force.z * dt), true);
    }

    if let Some(target) = rotation {
        let r = rb.rotation();
        let now = Quaternion::new(r.w, r.x, r.y, r.z);
        let w = rb.angvel();
        let now_spin = Vector3::new(w.x, w.y, w.z);
        // The turn from now to the target, the short way, as axis × angle.
        let mut delta = target * now.invert();
        if delta.s < 0.0 {
            delta = -delta;
        }
        let sin = delta.v.magnitude();
        let error = if sin > 1e-6 { delta.v / sin * (2.0 * sin.atan2(delta.s)) } else { Vector3::new(0.0, 0.0, 0.0) };
        let inertia = rb.mass_properties().local_mprops.principal_inertia();
        let inertia = ((inertia.x + inertia.y + inertia.z) / 3.0).max(1e-4);
        let torque = (error * gains.angular_stiffness + (spin - now_spin) * gains.angular_damping) * inertia;
        let (torque, _) = clamp(torque, gains.max_torque);
        if dt > 0.0 {
            rb.apply_torque_impulse(rapier3d::math::Vector::new(torque.x * dt, torque.y * dt, torque.z * dt), true);
        }
    }
    PdResult { force: force.magnitude(), strained }
}

fn clamp(v: Vector3<f32>, max: f32) -> (Vector3<f32>, bool) {
    let length = v.magnitude();
    if length > max && length > 0.0 {
        (v * (max / length), true)
    } else {
        (v, false)
    }
}

/// `v` turned by `q` (for callers working in cgmath).
pub fn turn(q: Quaternion<f32>, v: Vector3<f32>) -> Vector3<f32> {
    q.rotate_vector(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};

    fn crate_of(physics: &mut PhysicsEngine, kg: f32) -> RigidBodyHandle {
        let body = physics.add_rigid_body(RigidBodyBuilder::dynamic().translation(rapier3d::math::Vector::new(0.0, 1.0, 0.0)).build());
        // A 0.5 m cube of this mass.
        let collider = ColliderBuilder::cuboid(0.25, 0.25, 0.25).density(kg / 0.125).build();
        physics.add_collider(collider, Some(body));
        body
    }

    fn height_after(kg: f32, drives: usize, seconds: f32) -> f32 {
        let mut physics = PhysicsEngine::new();
        let body = crate_of(&mut physics, kg);
        let dt = 1.0 / 60.0;
        for _ in 0..(seconds / dt) as usize {
            for _ in 0..drives {
                drive(&mut physics, body, Point3::new(0.0, 1.5, 0.0), None, Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0), &PdGains::default(), dt);
            }
            physics.update(dt);
        }
        physics.rigid_bodies[body].translation().y
    }

    #[test]
    fn a_light_body_settles_just_below_the_target() {
        // 10 kg on 2000 N/m: about 5 cm below.
        let y = height_after(10.0, 1, 3.0);
        assert!((y - (1.5 - 98.1 / 2000.0)).abs() < 0.01, "{y}");
    }

    #[test]
    fn a_heavy_body_sags_and_one_over_the_limit_falls() {
        let light = height_after(10.0, 1, 3.0);
        let heavy = height_after(45.0, 1, 3.0);
        assert!(heavy < light - 0.02, "light {light}, heavy {heavy}");
        // 80 kg weigh ~785 N, more than one drive gives: it is not held.
        let too_heavy = height_after(80.0, 1, 3.0);
        assert!(too_heavy < 1.0, "{too_heavy}");
        // Two hold it (about 20 cm below the target: 785 N on 4000 N/m).
        let together = height_after(80.0, 2, 3.0);
        assert!(together > 1.25, "{together}");
    }

    #[test]
    fn it_turns_the_body_to_the_target_rotation() {
        let mut physics = PhysicsEngine::new();
        physics.gravity = rapier3d::math::Vector::new(0.0, 0.0, 0.0);
        let body = crate_of(&mut physics, 10.0);
        let target = Quaternion::from(cgmath::Euler::new(cgmath::Rad(0.0), cgmath::Rad(1.2), cgmath::Rad(0.0)));
        let dt = 1.0 / 60.0;
        for _ in 0..240 {
            drive(&mut physics, body, Point3::new(0.0, 1.0, 0.0), Some(target), Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0), &PdGains::default(), dt);
            physics.update(dt);
        }
        let r = physics.rigid_bodies[body].rotation();
        let now = Quaternion::new(r.w, r.x, r.y, r.z);
        assert!(now.dot(target).abs() > 0.995, "{now:?}");
    }

    /// A target swung round in a circle (a carrier turning on the spot)
    /// with its velocity and spin given: the body keeps up, close to it
    /// and turned as it is.
    #[test]
    fn a_body_keeps_up_with_a_target_turning_round() {
        let mut physics = PhysicsEngine::new();
        let body = crate_of(&mut physics, 15.0);
        let (radius, rate, dt) = (0.6_f32, 4.0_f32, 1.0 / 60.0);
        let at = |t: f32| Point3::new(radius * t.cos(), 1.0, radius * t.sin());
        let turn = |t: f32| Quaternion::from(cgmath::Euler::new(cgmath::Rad(0.0), cgmath::Rad(-t), cgmath::Rad(0.0)));
        // Start where the target starts.
        physics.rigid_bodies[body].set_translation(rapier3d::math::Vector::new(radius, 1.0, 0.0), true);
        let mut worst = (0.0_f32, 0.0_f32);
        for step in 0..240 {
            let angle = rate * dt * step as f32;
            let velocity = Vector3::new(-angle.sin(), 0.0, angle.cos()) * (radius * rate);
            let spin = Vector3::new(0.0, -rate, 0.0);
            drive(&mut physics, body, at(angle), Some(turn(angle)), velocity, spin, &PdGains::default(), dt);
            physics.update(dt);
            if step > 60 {
                let t = physics.rigid_bodies[body].translation();
                let r = physics.rigid_bodies[body].rotation();
                let now = Quaternion::new(r.w, r.x, r.y, r.z);
                let after = at(angle + rate * dt);
                let off = (Point3::new(t.x, t.y, t.z) - after).magnitude();
                let twist = 2.0 * now.dot(turn(angle + rate * dt)).abs().min(1.0).acos();
                worst = (worst.0.max(off), worst.1.max(twist));
            }
        }
        // Its weight hangs it ~7 cm low; turning adds little to that.
        assert!(worst.0 < 0.12, "{} m off", worst.0);
        assert!(worst.1.to_degrees() < 8.0, "{}° off", worst.1.to_degrees());
    }
}
