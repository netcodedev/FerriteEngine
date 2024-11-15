use rapier3d::prelude::*;

pub struct PhysicsEngine {
    pub rigid_bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,

    physics_pipeline: PhysicsPipeline,

    gravity: Vector<Real>,
    integration_parameters: IntegrationParameters,
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,

    physics_hooks: (),
    physics_events: (),
}

impl PhysicsEngine {
    pub fn new() -> Self {
        let gravity = vector![0.0, -9.81, 0.0];
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
        let query_pipeline = QueryPipeline::new();
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
            query_pipeline,

            physics_hooks,
            physics_events,
        }
    }

    pub fn update(&mut self) {
        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &self.physics_hooks,
            &self.physics_events,
        );
    }

    pub fn add_rigid_body(&mut self, rigid_body: RigidBody) -> RigidBodyHandle {
        self.rigid_bodies.insert(rigid_body)
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
