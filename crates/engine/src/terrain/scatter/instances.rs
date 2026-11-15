use std::cell::RefCell;

use cgmath::{EuclideanSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Vector3, Zero};
use gl::types::{GLsizei, GLsizeiptr, GLuint, GLvoid};
use glfw::WindowEvent;
use rapier3d::prelude::{ColliderBuilder, ColliderHandle, Pose, Rotation, Vector};

use crate::core::{
    entity::{component::Component, Entity},
    physics::physics_engine::{collider_tag, PhysicsEngine},
    renderer::pbr::{self, Lighting},
    scene::Scene,
    view_frustum::ViewFrustum,
};

use super::super::ChunkBounds;

use super::*;

/// An instance taken out of a `ScatterInstances`.
pub struct TakenInstance {
    pub spec: ScatterSpec,
    pub variant: usize,
    /// Where it stood (its base).
    pub position: Point3<f32>,
    pub yaw: f32,
}

/// One layer's instances on one chunk.
pub struct ScatterInstances {
    spec: ScatterSpec,
    /// `instances_by_variant[k]` — placements assigned to variant `k`. Empty slots
    /// are preserved so indices align with `ScatterModel::variants`.
    instances_by_variant: Vec<Vec<Matrix4<f32>>>,
    center: Point3<f32>,
    /// Box around every instance, for frustum culling.
    bounds: ChunkBounds,
    /// `(x, y, z, yaw)` of each instance, in the order of `colliders`.
    placements: Vec<(f32, f32, f32, f32)>,
    /// Colliders these instances added, so they can be taken out again
    /// without removing the chunk they stand on.
    colliders: Vec<ColliderHandle>,
    gpu: RefCell<Option<ScatterGpu>>,
}

struct ScatterGpu {
    variants: Vec<Option<VariantGpu>>,
}

struct VariantGpu {
    instance_vbo: GLuint,
    /// `vaos[lod_idx][submesh_idx]`
    vaos: Vec<Vec<GLuint>>,
    instance_count: GLsizei,
}

impl Drop for ScatterGpu {
    fn drop(&mut self) {
        if crate::core::headless::is_headless() {
            return;
        }
        unsafe {
            for kind in &self.variants {
                let Some(kind) = kind else { continue };
                gl::DeleteBuffers(1, &kind.instance_vbo);
                for lod_vaos in &kind.vaos {
                    if !lod_vaos.is_empty() {
                        gl::DeleteVertexArrays(lod_vaos.len() as GLsizei, lod_vaos.as_ptr());
                    }
                }
            }
        }
    }
}

impl ScatterInstances {
    pub fn new(spec: &ScatterSpec, placements: &[(f32, f32, f32, f32)]) -> Self {
        let spec = spec.clone();
        if placements.is_empty() {
            return Self {
                spec,
                instances_by_variant: Vec::new(),
                center: Point3::new(0.0, 0.0, 0.0),
                bounds: ChunkBounds {
                    min: (0, 0, 0),
                    max: (0, 0, 0),
                },
                placements: Vec::new(),
                colliders: Vec::new(),
                gpu: RefCell::new(None),
            };
        }
        let variant_count = spec.model.variants.len();
        let mut instances_by_variant: Vec<Vec<Matrix4<f32>>> =
            (0..variant_count).map(|_| Vec::new()).collect();

        for &(x, y, z, yaw) in placements {
            let variant_idx = pick_variant_index(x, z, variant_count);
            let translation = Matrix4::from_translation(Vector3::new(x, y, z));
            let yaw_rot: Matrix4<f32> = Quaternion::from_angle_y(Rad(yaw)).into();
            instances_by_variant[variant_idx].push(translation * yaw_rot);
        }

        let sum: Vector3<f32> = placements
            .iter()
            .fold(Vector3::zero(), |acc, &(x, y, z, _)| {
                acc + Vector3::new(x, y, z)
            });
        let center = Point3::from_vec(sum / placements.len() as f32);

        // Every variant's box, turned any way around its upright axis.
        let (mut reach, mut low, mut high) = (0.0f32, 0.0f32, 0.0f32);
        for variant in &spec.model.variants {
            let (min, max) = variant.bounds;
            reach = reach.max(min[0].abs()).max(max[0].abs()).max(min[2].abs()).max(max[2].abs());
            low = low.min(min[1]);
            high = high.max(max[1]);
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for &(x, y, z, _) in placements {
            min = [min[0].min(x - reach), min[1].min(y + low), min[2].min(z - reach)];
            max = [max[0].max(x + reach), max[1].max(y + high), max[2].max(z + reach)];
        }
        let bounds = ChunkBounds {
            min: (min[0].floor() as i32, min[1].floor() as i32, min[2].floor() as i32),
            max: (max[0].ceil() as i32, max[1].ceil() as i32, max[2].ceil() as i32),
        };

        Self {
            spec,
            instances_by_variant,
            center,
            bounds,
            placements: placements.to_vec(),
            colliders: Vec::new(),
            gpu: RefCell::new(None),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.instances_by_variant.iter().all(|v| v.is_empty())
    }

    /// One instance of `variant` at the origin, turned by `yaw`; place it
    /// with the parent transform when drawing.
    pub fn single(spec: &ScatterSpec, variant: usize, yaw: f32) -> Self {
        let mut instances_by_variant: Vec<Vec<Matrix4<f32>>> =
            (0..spec.model.variants.len()).map(|_| Vec::new()).collect();
        if let Some(slot) = instances_by_variant.get_mut(variant) {
            slot.push(Quaternion::from_angle_y(Rad(yaw)).into());
        }
        Self {
            spec: spec.clone(),
            instances_by_variant,
            center: Point3::new(0.0, 0.0, 0.0),
            bounds: ChunkBounds {
                min: (0, 0, 0),
                max: (0, 0, 0),
            },
            placements: Vec::new(),
            colliders: Vec::new(),
            gpu: RefCell::new(None),
        }
    }

    /// Take out the instance whose collider is `collider` (and the
    /// collider), e.g. a tree that was felled. `None` if it is not one of
    /// these instances.
    pub fn take_instance(
        &mut self,
        collider: ColliderHandle,
        physics: &mut PhysicsEngine,
    ) -> Option<TakenInstance> {
        let index = self.colliders.iter().position(|c| *c == collider)?;
        physics.remove_collider(self.colliders.remove(index));
        let (x, y, z, yaw) = self.placements.remove(index);
        let variant_count = self.spec.model.variants.len();
        let variant = pick_variant_index(x, z, variant_count);
        // Rebuilt from what is left; the GPU buffers follow on the next draw.
        let rest = Self::new(&self.spec, &self.placements);
        self.instances_by_variant = rest.instances_by_variant;
        *self.gpu.borrow_mut() = None;
        Some(TakenInstance {
            spec: self.spec.clone(),
            variant,
            position: Point3::new(x, y, z),
            yaw,
        })
    }

    /// The collider of the instance of the layer with `salt` standing at
    /// `position` (within a few centimetres), if it is one of these.
    pub fn collider_at(&self, salt: u64, position: Point3<f32>) -> Option<ColliderHandle> {
        if self.spec.salt != salt {
            return None;
        }
        let index = self.placements.iter().position(|&(x, y, z, _)| {
            (x - position.x).abs() < 0.05
                && (y - position.y).abs() < 0.05
                && (z - position.z).abs() < 0.05
        })?;
        self.colliders.get(index).copied()
    }

    /// The layer and base position of the instance whose collider is
    /// `collider`, if it is one of these.
    pub fn instance_of(&self, collider: ColliderHandle) -> Option<(u64, Point3<f32>)> {
        let index = self.colliders.iter().position(|c| *c == collider)?;
        let (x, y, z, _) = self.placements[index];
        Some((self.spec.salt, Point3::new(x, y, z)))
    }

    /// Height of the tallest part of `variant`, from its base.
    pub fn variant_height(spec: &ScatterSpec, variant: usize) -> f32 {
        spec.model
            .variants
            .get(variant)
            .map_or(1.0, |v| v.bounds.1[1].max(0.1))
    }

    /// Whether these instances are exactly what `spec` asks for.
    pub fn matches(&self, spec: &ScatterSpec) -> bool {
        self.spec.same_as(spec)
    }

    /// `parent` is the chunk's fixed body; parenting the colliders to it lets
    /// them be removed together with the chunk.
    pub fn add_colliders(
        &mut self,
        placements: &[(f32, f32, f32, f32)],
        scene: &mut Scene,
        parent: Option<rapier3d::prelude::RigidBodyHandle>,
    ) {
        let variant_count = self.spec.model.variants.len();
        for &(x, y, z, yaw) in placements {
            let variant_idx = pick_variant_index(x, z, variant_count);
            let translation = Vector::new(x, y, z);
            let rotation = Rotation::from_rotation_y(yaw);
            let pose = Pose::from_parts(translation, rotation);
            let collider = ColliderBuilder::new(self.spec.model.variants[variant_idx].collider_shape.clone())
                .position(pose)
                .user_data(collider_tag::SCATTER)
                .build();
            self.colliders
                .push(scene.physics_engine.add_collider(collider, parent));
        }
    }

    /// Take these instances' colliders out of the physics world. Harmless
    /// if they are already gone with the chunk body.
    pub fn remove_colliders(&mut self, physics: &mut PhysicsEngine) {
        for handle in self.colliders.drain(..) {
            physics.remove_collider(handle);
        }
    }

    fn ensure_gpu(&self) {
        if self.gpu.borrow().is_some() || crate::core::headless::is_headless() {
            return;
        }
        let gpu = build_instances_gpu(&self.instances_by_variant, &self.spec.model);
        *self.gpu.borrow_mut() = Some(gpu);
    }
}

fn build_instances_gpu(
    instances_by_variant: &[Vec<Matrix4<f32>>],
    r: &ScatterModel,
) -> ScatterGpu {
    let mut kinds: Vec<Option<VariantGpu>> = Vec::with_capacity(instances_by_variant.len());
    for (variant_idx, instances) in instances_by_variant.iter().enumerate() {
        if instances.is_empty() {
            kinds.push(None);
            continue;
        }
        let kind = &r.variants[variant_idx];

        // Per-variant instance VBO.
        let mut buf: Vec<f32> = Vec::with_capacity(instances.len() * 16);
        for m in instances {
            let cols: [[f32; 4]; 4] = (*m).into();
            for c in &cols {
                buf.extend_from_slice(c);
            }
        }
        let instance_vbo = unsafe {
            let mut vbo = 0;
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (buf.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                buf.as_ptr() as *const GLvoid,
                gl::STATIC_DRAW,
            );
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            vbo
        };

        // One VAO per (LOD, submesh) within this variant.
        let mut vaos_per_lod: Vec<Vec<GLuint>> = Vec::with_capacity(kind.lods.len());
        for lod in &kind.lods {
            let mut lod_vaos: Vec<GLuint> = vec![0; lod.submeshes.len()];
            if !lod_vaos.is_empty() {
                unsafe {
                    gl::GenVertexArrays(lod_vaos.len() as GLsizei, lod_vaos.as_mut_ptr());
                    for (vao, sub) in lod_vaos.iter().zip(lod.submeshes.iter()) {
                        gl::BindVertexArray(*vao);

                        gl::BindBuffer(gl::ARRAY_BUFFER, sub.vbo);
                        gl::EnableVertexAttribArray(0);
                        gl::VertexAttribPointer(
                            0,
                            3,
                            gl::FLOAT,
                            gl::FALSE,
                            VERTEX_STRIDE,
                            POS_OFFSET as *const GLvoid,
                        );
                        gl::EnableVertexAttribArray(1);
                        gl::VertexAttribPointer(
                            1,
                            3,
                            gl::FLOAT,
                            gl::FALSE,
                            VERTEX_STRIDE,
                            NORMAL_OFFSET as *const GLvoid,
                        );
                        gl::EnableVertexAttribArray(2);
                        gl::VertexAttribPointer(
                            2,
                            2,
                            gl::FLOAT,
                            gl::FALSE,
                            VERTEX_STRIDE,
                            UV_OFFSET as *const GLvoid,
                        );

                        gl::BindBuffer(gl::ARRAY_BUFFER, instance_vbo);
                        for col in 0..4 {
                            let loc = 3 + col as GLuint;
                            let offset = (col * 16) as usize;
                            gl::EnableVertexAttribArray(loc);
                            gl::VertexAttribPointer(
                                loc,
                                4,
                                gl::FLOAT,
                                gl::FALSE,
                                INSTANCE_STRIDE,
                                offset as *const GLvoid,
                            );
                            gl::VertexAttribDivisor(loc, 1);
                        }
                        gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, sub.ibo);
                    }
                }
            }
            vaos_per_lod.push(lod_vaos);
        }
        unsafe {
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);
        }

        kinds.push(Some(VariantGpu {
            instance_vbo,
            vaos: vaos_per_lod,
            instance_count: instances.len() as GLsizei,
        }));
    }
    ScatterGpu { variants: kinds }
}

fn pick_lod_index(distance: f32, lod_count: usize) -> usize {
    if lod_count == 0 {
        return 0;
    }
    let max_lod = lod_count - 1;
    if distance < LOD_DIST_NEAR {
        return 0;
    }
    if distance < LOD_DIST_MID {
        return 1.min(max_lod);
    }
    if distance < LOD_DIST_FAR {
        return 2.min(max_lod);
    }
    3.min(max_lod)
}

impl Component for ScatterInstances {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        // The ground map holds the terrain alone.
        if crate::core::renderer::weather::exposure::in_ground_pass() {
            return;
        }
        if !ViewFrustum::is_bounds_in_frustum_matrix(view_projection, self.bounds) {
            return;
        }
        let Some(lighting) = Lighting::from_scene(scene) else {
            return;
        };
        let d = self.center - lighting.camera_position;
        let distance = (d.x * d.x + d.y * d.y + d.z * d.z).sqrt();
        self.draw(
            view_projection,
            parent_transform,
            &lighting,
            LodChoice::Distance(distance),
        );
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn on_remove(&mut self, physics: &mut PhysicsEngine) {
        self.remove_colliders(physics);
    }
}

impl ScatterInstances {
    /// Draw the instances. Needs no scene, so previews can use it too.
    pub fn draw(
        &self,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
        lighting: &Lighting,
        lod: LodChoice,
    ) {
        if self.is_empty() {
            return;
        }
        self.ensure_gpu();
        let gpu_borrow = self.gpu.borrow();
        let gpu = gpu_borrow.as_ref().expect("gpu just initialized");

        {
            let r = &*self.spec.model;
            r.shader.bind();
            r.shader.set_uniform_mat4("viewProjection", view_projection);
            r.shader.set_uniform_mat4("parentTransform", parent_transform);
            lighting.apply(&r.shader);

            unsafe {
                gl::Enable(gl::DEPTH_TEST);
                gl::DepthMask(gl::TRUE);
                gl::DepthFunc(gl::LESS);
                gl::Disable(gl::CULL_FACE);
                for (variant_idx, variant_gpu_opt) in gpu.variants.iter().enumerate() {
                    let Some(variant_gpu) = variant_gpu_opt else { continue };
                    let kind = &r.variants[variant_idx];
                    let lod_idx = match lod {
                        LodChoice::Distance(distance) => pick_lod_index(distance, kind.lods.len()),
                        LodChoice::Fixed(tier) => tier.min(kind.lods.len().saturating_sub(1)),
                    };
                    let lod = &kind.lods[lod_idx];
                    let vaos = &variant_gpu.vaos[lod_idx];
                    for (sub_idx, sub) in lod.submeshes.iter().enumerate() {
                        pbr::bind_material(
                            &r.shader,
                            kind.model.material_textures(sub.material_index),
                        );
                        gl::BindVertexArray(vaos[sub_idx]);
                        gl::DrawElementsInstanced(
                            gl::TRIANGLES,
                            sub.index_count,
                            gl::UNSIGNED_INT,
                            std::ptr::null(),
                            variant_gpu.instance_count,
                        );
                    }
                }
                gl::BindVertexArray(0);
                gl::Enable(gl::CULL_FACE);
                gl::Disable(gl::DEPTH_TEST);
            }
        }
    }
}
