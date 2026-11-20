use rapier3d::prelude::*;

use super::ray_index::RayIndex;

/// What a collider is, kept in its `user_data`. The engine tags what it
/// makes; games use values from `GAME_TAGS` up for their own.
pub mod collider_tag {
    /// Not tagged.
    pub const NONE: u128 = 0;
    /// Terrain chunks.
    pub const TERRAIN: u128 = 1;
    /// Things scattered over the terrain (trees, rocks).
    pub const SCATTER: u128 = 2;
    /// The first value games tag their own colliders with.
    pub const GAME_TAGS: u128 = 1000;
}

/// Where a ray hit: the collider, how far along the ray, and the surface
/// normal there.
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub collider: ColliderHandle,
    pub distance: f32,
    pub normal: [f32; 3],
}

pub struct PhysicsEngine {
    /// Moving a fixed body by setting its pose here: the queries find it
    /// at its new place from the next frame, or at once after `body_moved`.
    pub rigid_bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,
    pub soft_bodies: SoftBodySet,

    physics_pipeline: PhysicsPipeline,

    gravity: Vector,
    integration_parameters: IntegrationParameters,
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    ccd_solver: CCDSolver,

    physics_hooks: (),
    physics_events: (),

    /// Where the colliders are, for the queries (see `ray_index`).
    index: RayIndex,
    /// Whether the simulation is off (see `paused_frame`): the queries then
    /// bring the colliders to their bodies once a frame, not every time.
    paused: bool,
    /// Whether they are to do so before the next query.
    propagate_due: bool,
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
        let soft_bodies = SoftBodySet::new();
        let ccd_solver = CCDSolver::new();
        let physics_hooks = ();
        let physics_events = ();

        PhysicsEngine {
            rigid_bodies,
            colliders,
            impulse_joints,
            multibody_joints,
            soft_bodies,

            physics_pipeline,
            gravity,
            integration_parameters,
            island_manager,
            broad_phase,
            narrow_phase,
            ccd_solver,

            physics_hooks,
            physics_events,

            index: RayIndex::default(),
            paused: true,
            propagate_due: true,
        }
    }

    /// A new frame begins: fixed bodies moved since (by setting their pose)
    /// are found again by the queries (see `ray_index`). The scene calls
    /// this before it updates.
    pub fn begin_frame(&mut self) {
        self.index.poses_stale = true;
        self.propagate_due = true;
    }

    /// A frame in which nothing is simulated (the editor, a paused game).
    /// rapier keeps a list of the bodies changed since its last step, which
    /// only a step empties, and does not take removed bodies off it: in the
    /// editor it would grow with every chunk ever loaded, and the queries,
    /// which bring the colliders of every body on it to their body, with
    /// it. Here it is cut back to the bodies there are (all of which a
    /// first step then looks at, as it would have), and the queries bring
    /// the colliders to their bodies once a frame.
    pub fn paused_frame(&mut self) {
        self.rigid_bodies.iter_mut().for_each(drop);
        self.paused = true;
        self.begin_frame();
    }

    /// A body was moved by setting its pose: the queries find it where it
    /// is now in this frame already, not only from the next.
    pub fn body_moved(&mut self) {
        self.index.poses_stale = true;
        self.propagate_due = true;
    }

    pub fn update(&mut self, delta_time: f32) {
        self.paused = false;
        self.index.poses_stale = true;
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
            &mut self.soft_bodies,
            &mut self.ccd_solver,
            &self.physics_hooks,
            &self.physics_events,
        );
    }

    pub fn add_rigid_body(&mut self, rigid_body: rapier3d::dynamics::RigidBody) -> RigidBodyHandle {
        self.index.dirty = true;
        self.index.poses_stale = true;
        self.propagate_due = true;
        self.rigid_bodies.insert(rigid_body)
    }

    /// Bring the colliders' poses and the query index up to date.
    fn prepare_queries(&mut self) {
        // Running, only the bodies moved since the last step are on the
        // list: cheap, and exact for every query. Paused, every body is on
        // it: once a frame (or after `body_moved`).
        if !self.paused || std::mem::take(&mut self.propagate_due) {
            self.rigid_bodies
                .propagate_modified_body_positions_to_colliders(&mut self.colliders);
        }
        self.index.sync(&self.rigid_bodies, &self.colliders);
    }

    /// The nearest hit of `ray` within `max_distance` on a non-sensor
    /// collider `keep` accepts. Looks only at the colliders near the ray
    /// (see `ray_index`), so it is cheap among many, and it does not rely
    /// on the broad phase, which only `update` refreshes: it also works
    /// while the simulation is paused (picking in the editor).
    fn nearest_hit(
        &mut self,
        ray: &Ray,
        max_distance: f32,
        keep: impl Fn(ColliderHandle, &Collider) -> bool,
    ) -> Option<RayHit> {
        use rapier3d::parry::query::RayCast;

        self.prepare_queries();
        let colliders = &self.colliders;
        let mut best: Option<RayHit> = None;
        self.index.on_ray(ray, max_distance, |handle| {
            let limit = best.as_ref().map_or(max_distance, |hit| hit.distance);
            let Some(collider) = colliders.get(handle) else {
                return limit;
            };
            if collider.is_sensor() || !keep(handle, collider) {
                return limit;
            }
            if collider.compute_aabb().cast_local_ray(ray, limit, true).is_none() {
                return limit;
            }
            match collider.shape().cast_ray_and_get_normal(collider.position(), ray, limit, true) {
                Some(hit) => {
                    best = Some(RayHit {
                        collider: handle,
                        distance: hit.time_of_impact,
                        normal: [hit.normal.x, hit.normal.y, hit.normal.z],
                    });
                    hit.time_of_impact
                }
                None => limit,
            }
        });
        best
    }

    /// Ray cast against every non-sensor collider attached to a body,
    /// returning the closest body and hit distance (see `nearest_hit`).
    pub fn cast_ray_bodies(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_distance: f32,
    ) -> Option<(RigidBodyHandle, f32)> {
        let ray = Ray::new(
            Vector::new(origin[0], origin[1], origin[2]),
            Vector::new(dir[0], dir[1], dir[2]),
        );
        let hit = self.nearest_hit(&ray, max_distance, |_, collider| collider.parent().is_some())?;
        let parent = self.colliders.get(hit.collider)?.parent()?;
        Some((parent, hit.distance))
    }

    /// The nearest non-sensor collider the ray hits within `max_distance`,
    /// with the distance, leaving out colliders of the `ignore`d bodies.
    /// `dir` must be normalized.
    pub fn cast_ray_colliders(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_distance: f32,
        ignore: &[RigidBodyHandle],
    ) -> Option<(ColliderHandle, f32)> {
        self.cast_ray(origin, dir, max_distance, ignore)
            .map(|hit| (hit.collider, hit.distance))
    }

    /// Like `cast_ray_colliders`, with the surface normal where it hits.
    pub fn cast_ray(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_distance: f32,
        ignore: &[RigidBodyHandle],
    ) -> Option<RayHit> {
        self.cast_ray_where(origin, dir, max_distance, |_, collider| {
            !collider.parent().is_some_and(|p| ignore.contains(&p))
        })
    }

    /// The nearest non-sensor collider the ray hits within `max_distance`
    /// among those `keep` accepts. `dir` must be normalized.
    pub fn cast_ray_where(
        &mut self,
        origin: [f32; 3],
        dir: [f32; 3],
        max_distance: f32,
        keep: impl Fn(ColliderHandle, &Collider) -> bool,
    ) -> Option<RayHit> {
        let ray = Ray::new(
            Vector::new(origin[0], origin[1], origin[2]),
            Vector::new(dir[0], dir[1], dir[2]),
        );
        self.nearest_hit(&ray, max_distance, keep)
    }

    /// Every non-sensor collider `keep` accepts that `shape` at `pose`
    /// overlaps.
    pub fn intersecting(
        &mut self,
        shape: &dyn Shape,
        pose: &Pose,
        keep: impl Fn(ColliderHandle, &Collider) -> bool,
    ) -> Vec<ColliderHandle> {
        self.prepare_queries();
        let aabb = shape.compute_aabb(pose);
        let colliders = &self.colliders;
        self.index
            .in_box(&aabb)
            .into_iter()
            .filter_map(|handle| colliders.get(handle).map(|collider| (handle, collider)))
            .filter(|(handle, collider)| {
                !collider.is_sensor()
                    && collider.compute_aabb().intersects(&aabb)
                    && keep(*handle, collider)
                    && rapier3d::parry::query::intersection_test(
                        pose,
                        shape,
                        collider.position(),
                        collider.shape(),
                    )
                    .is_ok_and(|test| test.intersecting)
            })
            .map(|(handle, _)| handle)
            .collect()
    }

    /// The tag of a collider (see `collider_tag`); `NONE` if it is gone.
    pub fn collider_tag(&self, handle: ColliderHandle) -> u128 {
        self.colliders
            .get(handle)
            .map_or(collider_tag::NONE, |collider| collider.user_data)
    }

    /// Remove a body together with every collider attached to it.
    pub fn remove_rigid_body(&mut self, handle: RigidBodyHandle) {
        self.index.dirty = true;
        self.rigid_bodies.remove(
            handle,
            &mut self.island_manager,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            true,
        );
    }

    /// Remove a single collider, leaving its body in place. Does nothing if
    /// the collider no longer exists.
    pub fn remove_collider(&mut self, handle: ColliderHandle) {
        self.index.dirty = true;
        self.colliders.remove(
            handle,
            &mut self.island_manager,
            &mut self.rigid_bodies,
            &mut self.soft_bodies,
            true,
        );
    }

    pub fn add_collider(
        &mut self,
        collider: Collider,
        rigid_body_handle: Option<RigidBodyHandle>,
    ) -> ColliderHandle {
        self.index.dirty = true;
        self.propagate_due = true;
        if let Some(rigid_body_handle) = rigid_body_handle {
            self.colliders
                .insert_with_parent(collider, rigid_body_handle, &mut self.rigid_bodies)
        } else {
            self.colliders.insert(collider)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nearest hit by looking at every collider, as the queries did
    /// before the index.
    fn every_collider(physics: &mut PhysicsEngine, origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<(ColliderHandle, f32)> {
        physics.rigid_bodies.propagate_modified_body_positions_to_colliders(&mut physics.colliders);
        let ray = Ray::new(Vector::new(origin[0], origin[1], origin[2]), Vector::new(dir[0], dir[1], dir[2]));
        physics
            .colliders
            .iter()
            .filter(|(_, c)| !c.is_sensor())
            .filter_map(|(h, c)| c.shape().cast_ray(c.position(), &ray, max, true).map(|t| (h, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// A small deterministic random number generator, 0..1.
    fn numbers(seed: u64) -> impl FnMut() -> f32 {
        let mut state = seed;
        move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    /// A forest of fixed balls over 600 m, a wide fixed ground slab, a
    /// moving body and a collider without one.
    fn world() -> (PhysicsEngine, Vec<(RigidBodyHandle, ColliderHandle)>) {
        let mut physics = PhysicsEngine::new();
        let mut random = numbers(7);
        let mut trees = Vec::new();
        for _ in 0..400 {
            let at = Vector::new(random() * 600.0 - 300.0, random() * 20.0, random() * 600.0 - 300.0);
            let body = physics.add_rigid_body(RigidBodyBuilder::fixed().translation(at).build());
            let collider = physics.add_collider(ColliderBuilder::ball(1.0 + random() * 3.0).build(), Some(body));
            trees.push((body, collider));
        }
        let ground = physics.add_rigid_body(RigidBodyBuilder::fixed().translation(Vector::new(0.0, -2.0, 0.0)).build());
        physics.add_collider(ColliderBuilder::cuboid(5000.0, 0.5, 5000.0).build(), Some(ground));
        let moving = physics.add_rigid_body(RigidBodyBuilder::dynamic().translation(Vector::new(10.0, 5.0, 10.0)).build());
        physics.add_collider(ColliderBuilder::ball(2.0).build(), Some(moving));
        physics.add_collider(ColliderBuilder::ball(2.0).translation(Vector::new(-40.0, 5.0, 40.0)).build(), None);
        (physics, trees)
    }

    fn agree(physics: &mut PhysicsEngine, seed: u64) {
        let mut random = numbers(seed);
        for k in 0..600 {
            let origin = [random() * 700.0 - 350.0, random() * 60.0 + 1.0, random() * 700.0 - 350.0];
            // Straight down, level, and every way.
            let dir = match k % 3 {
                0 => [0.0, -1.0, 0.0],
                1 => {
                    let a = random() * std::f32::consts::TAU;
                    [a.cos(), 0.0, a.sin()]
                }
                _ => {
                    let v = [random() - 0.5, random() - 0.5, random() - 0.5];
                    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-3);
                    v.map(|c| c / l)
                }
            };
            let max = 50.0 + random() * 500.0;
            let expected = every_collider(physics, origin, dir, max);
            let got = physics.cast_ray_where(origin, dir, max, |_, _| true).map(|hit| (hit.collider, hit.distance));
            match (expected, got) {
                (None, None) => {}
                (Some(e), Some(g)) => assert!((e.1 - g.1).abs() < 1e-3, "ray {k}: {e:?} vs {g:?}"),
                (e, g) => panic!("ray {k} from {origin:?} along {dir:?}: {e:?} vs {g:?}"),
            }
        }
    }

    #[test]
    fn indexed_rays_hit_what_every_collider_would() {
        let (mut physics, _) = world();
        agree(&mut physics, 1);
    }

    #[test]
    fn rays_follow_fixed_bodies_moved_and_colliders_removed() {
        let (mut physics, trees) = world();
        agree(&mut physics, 2);
        // Moved past the index's back, as a door is swung, and some taken
        // out, one put in where another was.
        for &(body, _) in trees.iter().step_by(3) {
            let at = physics.rigid_bodies[body].translation();
            physics.rigid_bodies[body].set_translation(Vector::new(-at.x, at.y + 3.0, at.z + 120.0), true);
        }
        for &(_, collider) in trees.iter().skip(1).step_by(5) {
            physics.remove_collider(collider);
        }
        let body = trees[0].0;
        physics.add_collider(ColliderBuilder::ball(6.0).build(), Some(body));
        physics.begin_frame();
        agree(&mut physics, 3);
        // A falling body is where it is now.
        let moving = physics.rigid_bodies.iter().find(|(_, b)| b.is_dynamic()).unwrap().0;
        physics.rigid_bodies[moving].set_translation(Vector::new(100.0, 5.0, -100.0), true);
        // (Paused, as here, a moved body is found from the next frame or
        // when told.)
        physics.body_moved();
        // A fixed one moved within the frame is found at once when told.
        let (tree, _) = trees[2];
        physics.rigid_bodies[tree].set_translation(Vector::new(250.0, 40.0, 250.0), true);
        physics.body_moved();
        assert!(physics.cast_ray_where([250.0, 80.0, 250.0], [0.0, -1.0, 0.0], 100.0, |_, _| true).is_some());
        let hit = physics.cast_ray_where([100.0, 30.0, -100.0], [0.0, -1.0, 0.0], 100.0, |_, _| true).unwrap();
        assert!((hit.distance - 23.0).abs() < 1e-3, "{hit:?}");
        agree(&mut physics, 4);
    }

    #[test]
    fn what_changed_while_paused_still_collides_once_it_runs() {
        let mut physics = PhysicsEngine::new();
        // Chunks come and go in the editor, with no step between.
        for k in 0..200 {
            let body = physics.add_rigid_body(RigidBodyBuilder::fixed().translation(Vector::new(k as f32 * 10.0, 0.0, 0.0)).build());
            physics.add_collider(ColliderBuilder::cuboid(4.0, 0.5, 4.0).build(), Some(body));
            physics.paused_frame();
            if k % 2 == 0 {
                physics.remove_rigid_body(body);
                physics.paused_frame();
            }
        }
        let ground = physics.add_rigid_body(RigidBodyBuilder::fixed().build());
        physics.add_collider(ColliderBuilder::cuboid(50.0, 0.5, 50.0).build(), Some(ground));
        let ball = physics.add_rigid_body(RigidBodyBuilder::dynamic().translation(Vector::new(0.0, 3.0, 0.0)).build());
        physics.add_collider(ColliderBuilder::ball(0.5).build(), Some(ball));
        physics.paused_frame();
        // Paused: nothing falls.
        assert_eq!(physics.rigid_bodies[ball].translation().y, 3.0);
        // Running: it falls onto the ground and stays on it.
        for _ in 0..240 {
            physics.update(1.0 / 60.0);
        }
        let y = physics.rigid_bodies[ball].translation().y;
        assert!((y - 1.0).abs() < 0.05, "{y}");
    }

    #[test]
    fn overlaps_find_what_every_collider_would() {
        let (mut physics, _) = world();
        let mut random = numbers(9);
        for _ in 0..200 {
            let at = Vector::new(random() * 600.0 - 300.0, random() * 20.0, random() * 600.0 - 300.0);
            let ball = Ball::new(2.0 + random() * 20.0);
            let pose = Pose::from_translation(at);
            let mut got = physics.intersecting(&ball, &pose, |_, _| true);
            let mut expected: Vec<ColliderHandle> = physics
                .colliders
                .iter()
                .filter(|(_, c)| {
                    rapier3d::parry::query::intersection_test(&pose, &ball, c.position(), c.shape())
                        .is_ok_and(|t| t.intersecting)
                })
                .map(|(h, _)| h)
                .collect();
            got.sort_by_key(|h| h.into_raw_parts());
            expected.sort_by_key(|h| h.into_raw_parts());
            assert_eq!(got, expected);
        }
    }

    /// How long a ray takes among 4000 trees, after 20000 bodies came and
    /// went with the simulation off. Not a check; run it with `cargo test
    /// --release -p ferrite ray_cast_timings -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ray_cast_timings() {
        let mut physics = PhysicsEngine::new();
        let mut random = numbers(3);
        for _ in 0..4000 {
            let at = Vector::new(random() * 400.0 - 200.0, random() * 20.0, random() * 400.0 - 200.0);
            let body = physics.add_rigid_body(RigidBodyBuilder::fixed().translation(at).build());
            physics.add_collider(ColliderBuilder::capsule_y(4.0, 0.4).build(), Some(body));
        }
        physics.paused_frame();
        // Chunks that came and went in the editor.
        for _ in 0..20000 {
            let body = physics.add_rigid_body(RigidBodyBuilder::fixed().build());
            physics.add_collider(ColliderBuilder::ball(1.0).build(), Some(body));
            physics.remove_rigid_body(body);
        }
        physics.paused_frame();
        let rays: Vec<[f32; 3]> = (0..2000).map(|_| [random() * 400.0 - 200.0, 100.0, random() * 400.0 - 200.0]).collect();
        let t = std::time::Instant::now();
        for r in &rays { every_collider(&mut physics, *r, [0.0, -1.0, 0.0], 400.0); }
        let brute = t.elapsed().as_secs_f64() * 1e6 / rays.len() as f64;
        let t = std::time::Instant::now();
        for r in &rays { physics.cast_ray_where(*r, [0.0, -1.0, 0.0], 400.0, |_, _| true); }
        let indexed = t.elapsed().as_secs_f64() * 1e6 / rays.len() as f64;
        let t = std::time::Instant::now();
        for r in &rays { physics.cast_ray_where(*r, [std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2], 60.0, |_, _| true); }
        let level = t.elapsed().as_secs_f64() * 1e6 / rays.len() as f64;
        println!("per ray: every collider {brute:.1} us, indexed down {indexed:.2} us, indexed level 60 m {level:.2} us");
    }
}
