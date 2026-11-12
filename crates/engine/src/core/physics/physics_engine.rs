use rapier3d::prelude::*;

pub struct PhysicsEngine {
    pub rigid_bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,

    physics_pipeline: PhysicsPipeline,

    gravity: Vector,
    integration_parameters: IntegrationParameters,
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    ccd_solver: CCDSolver,

    physics_hooks: (),
    physics_events: (),
}

impl PhysicsEngine {
    pub fn new() -> Self {
        let gravity = Vector::new(0.0, -9.81, 0.0);
        let integration_parameters = IntegrationParameters::default();
        let physics_pipeline = PhysicsPipeline::new();
        let island_manager = IslandManager::new();
        let broad_phase = DefaultBroadPhase::new();
        let narrow_phase = NarrowPhase::new();
        let rigid_bodies = RigidBodySet::new();
        let colliders = ColliderSet::new();
        let impulse_joints = ImpulseJointSet::new();
        let multibody_joints = MultibodyJointSet::new();
        let ccd_solver = CCDSolver::new();
        let physics_hooks = ();
        let physics_events = ();

        PhysicsEngine {
            rigid_bodies,
            colliders,
            impulse_joints,
            multibody_joints,

            physics_pipeline,
            gravity,
            integration_parameters,
            island_manager,
            broad_phase,
            narrow_phase,
            ccd_solver,

            physics_hooks,
            physics_events,
        }
    }

    pub fn update(&mut self, delta_time: f32) {
        self.integration_parameters.dt = delta_time;
        self.physics_pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &self.physics_hooks,
            &self.physics_events,
        );
    }

    pub fn add_rigid_body(&mut self, rigid_body: rapier3d::dynamics::RigidBody) -> RigidBodyHandle {
        self.rigid_bodies.insert(rigid_body)
    }

    /// Ray cast against every non-sensor collider attached to a body,
    /// returning the closest body and hit distance. Unlike rapier's query
    /// pipeline this does not rely on the broad phase, which is only
    /// refreshed by `update` — so it also works while the simulation is
    /// paused (e.g. picking in the editor).
    pub fn cast_ray_bodies(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_distance: f32,
    ) -> Option<(RigidBodyHandle, f32)> {
        use rapier3d::parry::query::RayCast;

        self.rigid_bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
        let ray = Ray::new(
            Vector::new(origin[0], origin[1], origin[2]),
            Vector::new(dir[0], dir[1], dir[2]),
        );
        let mut best: Option<(RigidBodyHandle, f32)> = None;
        for (_, collider) in self.colliders.iter() {
            if collider.is_sensor() {
                continue;
            }
            let Some(parent) = collider.parent() else {
                continue;
            };
            let limit = best.map_or(max_distance, |(_, t)| t);
            if collider
                .compute_aabb()
                .cast_local_ray(&ray, limit, true)
                .is_none()
            {
                continue;
            }
            if let Some(t) = collider
                .shape()
                .cast_ray(collider.position(), &ray, limit, true)
            {
                best = Some((parent, t));
            }
        }
        best
    }

    /// Remove a body together with every collider attached to it.
    pub fn remove_rigid_body(&mut self, handle: RigidBodyHandle) {
        self.rigid_bodies.remove(
            handle,
            &mut self.island_manager,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
    }

    pub fn add_collider(
        &mut self,
        collider: Collider,
        rigid_body_handle: Option<RigidBodyHandle>,
    ) -> ColliderHandle {
        if let Some(rigid_body_handle) = rigid_body_handle {
            self.colliders
                .insert_with_parent(collider, rigid_body_handle, &mut self.rigid_bodies)
        } else {
            self.colliders.insert(collider)
        }
    }
}
