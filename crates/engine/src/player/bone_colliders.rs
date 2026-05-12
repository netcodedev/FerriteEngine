use cgmath::{EuclideanSpace, InnerSpace, Matrix4, SquareMatrix, Vector3};
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder, RigidBodyHandle};

use crate::core::{
    entity::{
        component::{model_component::ModelComponent, Component},
        Entity,
    },
    scene::Scene,
};

const MIN_BONE_LENGTH: f32 = 0.04;
const BONE_RADIUS_FACTOR: f32 = 0.12;
const MAX_BONE_RADIUS: f32 = 0.08;

pub struct BoneColliders {
    handles: Vec<(RigidBodyHandle, String)>,
}

impl BoneColliders {
    pub fn build(scene: &mut Scene, entity: &Entity) -> Self {
        let segments = entity
            .get_component::<ModelComponent>()
            .map(|mc| mc.get_model().get_bone_segments(&Matrix4::identity()))
            .unwrap_or_default();

        let mut handles = Vec::new();
        for (name, parent_pos, bone_pos) in &segments {
            let dir = bone_pos - parent_pos;
            let len = dir.magnitude();
            if len < MIN_BONE_LENGTH {
                continue;
            }
            let radius = (len * BONE_RADIUS_FACTOR).min(MAX_BONE_RADIUS);
            let half_height = (len / 2.0 - radius).max(0.0);
            let mid = parent_pos + dir * 0.5;

            let rb = RigidBodyBuilder::kinematic_position_based()
                .translation(rapier3d::prelude::Vector::new(mid.x, mid.y, mid.z))
                .build();
            let rb_handle = scene.physics_engine.add_rigid_body(rb);

            let collider = ColliderBuilder::capsule_y(half_height, radius)
                .sensor(true)
                .build();
            scene.physics_engine.add_collider(collider, Some(rb_handle));

            handles.push((rb_handle, name.clone()));
        }

        Self { handles }
    }
}

impl Component for BoneColliders {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, _dt: f64) {
        let entity_transform = Matrix4::from_translation(entity.get_position().to_vec().into())
            * Matrix4::from(entity.get_rotation());

        let segments = entity
            .get_component::<ModelComponent>()
            .map(|mc| mc.get_model().get_bone_segments(&entity_transform))
            .unwrap_or_default();

        for (rb_handle, bone_name) in &self.handles {
            let Some((_, parent_pos, bone_pos)) =
                segments.iter().find(|(n, _, _)| n == bone_name)
            else {
                continue;
            };

            let dir = bone_pos - parent_pos;
            let len = dir.magnitude();
            if len < 0.0001 {
                continue;
            }

            let mid = parent_pos + dir * 0.5;
            let rot = rotation_y_to(dir / len);

            let rb = &mut scene.physics_engine.rigid_bodies[*rb_handle];
            rb.set_next_kinematic_translation(rapier3d::prelude::Vector::new(mid.x, mid.y, mid.z));
            rb.set_next_kinematic_rotation(rot);
        }
    }

    fn handle_event(
        &mut self,
        _: &mut glfw::Glfw,
        _: &mut glfw::Window,
        _: &glfw::WindowEvent,
    ) {
    }
}

/// Glamx quaternion rotating the +Y axis to align with `dir` (already normalized).
fn rotation_y_to(dir: Vector3<f32>) -> rapier3d::math::Rotation {
    use rapier3d::glamx::{Quat, Vec3};
    let dot = dir.y; // dot with (0,1,0)
    if dot > 0.9999 {
        return Quat::IDENTITY;
    }
    if dot < -0.9999 {
        return Quat::from_axis_angle(Vec3::X, std::f32::consts::PI);
    }
    let axis = Vector3::new(0.0, 1.0, 0.0).cross(dir).normalize();
    let angle = dot.acos();
    Quat::from_axis_angle(Vec3::new(axis.x, axis.y, axis.z), angle)
}
