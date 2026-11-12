//! Per-chunk forest backed by a pool of multiple tree variants, each with
//! discrete LODs. Trees are loaded once into a thread-local resource pool;
//! each chunk picks a kind per placement (deterministically hashed from the
//! tree's XZ position) and drives one `glDrawElementsInstanced` call per
//! (kind, LOD-submesh) bucket it actually contains.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use cgmath::{EuclideanSpace, Matrix4, Point3, Quaternion, Rad, Rotation3, Vector3, Zero};
use gl::types::{GLsizei, GLsizeiptr, GLuint, GLvoid};
use glfw::WindowEvent;
use rapier3d::prelude::{ColliderBuilder, Pose, Rotation, SharedShape, TriMeshFlags, Vector};
use russimp_ng::material::TextureType;

use crate::core::{
    entity::{
        component::Component,
        Entity,
    },
    model::{Model, ModelBuilder},
    renderer::{light::skylight::SkyLight, shader::Shader, texture::Texture},
    scene::Scene,
};

use super::dual_contouring::WATER_LEVEL;

pub const TREE_MIN_Y: f32 = WATER_LEVEL + 4.0;
pub const TREE_MAX_Y: f32 = 78.0;
pub const TREE_ATTEMPTS_PER_CHUNK: u32 = 70;

const TREE_SCALE: f32 = 1.0;

/// Distance bands for LOD selection. With 4 LODs available (Acer pack), each
/// instance picks tier 0/1/2/3 by ascending distance; trees with fewer LODs
/// (Paine pine has 3) clamp to their last tier.
const LOD_DIST_NEAR: f32 = 60.0;
const LOD_DIST_MID: f32 = 130.0;
const LOD_DIST_FAR: f32 = 250.0;

/// Tree sources loaded into the pool. Each source can contribute one or many
/// `TreeKind`s depending on its scene structure.
const TREE_SOURCES: &[&str] = &[
    "Trees/Trees.gltf",
    "Trees/Acer tree pack.fbx",
];

const POS_OFFSET: usize = 0;
const NORMAL_OFFSET: usize = 12;
const UV_OFFSET: usize = 24;
const VERTEX_STRIDE: i32 = 32;

const INSTANCE_STRIDE: i32 = 64;

thread_local! {
    static TREE_RESOURCES: RefCell<Option<TreeResources>> = const { RefCell::new(None) };
}

struct TreeResources {
    shader: Shader,
    kinds: Vec<TreeKind>,
}

struct TreeKind {
    /// Shared with sibling kinds loaded from the same multi-tree asset, so
    /// material textures live in exactly one place.
    model: Rc<Model>,
    /// Detail tiers sorted from highest quality (index 0) to lowest.
    lods: Vec<TreeLod>,
    collider_shape: SharedShape,
}

struct TreeLod {
    submeshes: Vec<TreeSubmeshGpu>,
}

struct TreeSubmeshGpu {
    vbo: GLuint,
    ibo: GLuint,
    index_count: GLsizei,
    material_index: usize,
}

impl Drop for TreeSubmeshGpu {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteBuffers(1, &self.vbo);
            gl::DeleteBuffers(1, &self.ibo);
        }
    }
}

/// Free the shared tree models and textures. Must run on the GL thread while
/// the context is still alive; otherwise they would be dropped with the
/// thread-local at process exit, after the context is gone.
pub fn release_gl_resources() {
    let _ = TREE_RESOURCES.try_with(|cell| cell.borrow_mut().take());
}

fn with_tree_resources<R>(f: impl FnOnce(&TreeResources) -> R) -> R {
    TREE_RESOURCES.with(|cell| {
        {
            let mut borrow = cell.borrow_mut();
            if borrow.is_none() {
                *borrow = Some(load_tree_resources());
            }
        }
        let borrow = cell.borrow();
        f(borrow.as_ref().expect("tree resources just initialized"))
    })
}

/// Sort key for an LOD's name. Lower numbers are higher detail.
fn lod_priority(name: &str) -> u32 {
    let lc = name.to_lowercase();
    if lc.contains("lod0") {
        0
    } else if lc.contains("lod1") {
        1
    } else if lc.contains("lod2") {
        2
    } else if lc.contains("lod3") || lc.contains("billboard") {
        3
    } else if lc.contains("high") {
        0
    } else if lc.contains("last") {
        2
    } else if lc.contains("low") {
        1
    } else {
        99
    }
}

fn load_tree_resources() -> TreeResources {
    let shader = Shader::new(
        include_str!("tree_instanced_vertex.glsl"),
        include_str!("tree_instanced_fragment.glsl"),
    );
    let mut kinds: Vec<TreeKind> = Vec::new();
    for path in TREE_SOURCES {
        match load_kinds_from(path) {
            Ok(mut variants) => kinds.append(&mut variants),
            Err(e) => eprintln!("tree_forest: failed to load '{path}': {e}"),
        }
    }
    if kinds.is_empty() {
        panic!("tree_forest: no tree kinds loaded (check TREE_SOURCES)");
    }
    TreeResources { shader, kinds }
}

fn load_kinds_from(path: &str) -> Result<Vec<TreeKind>, Box<dyn std::error::Error>> {
    let mut model = ModelBuilder::new(path)?.with_scale(TREE_SCALE).build();
    model.init();
    let mut groups = model.extract_tree_groups();
    if groups.is_empty() {
        return Err(format!("no tree variants found in '{path}'").into());
    }

    let shared_model = Rc::new(model);

    let mut out = Vec::with_capacity(groups.len());
    for group in groups.iter_mut() {
        group.lods.sort_by_key(|(name, _)| lod_priority(name));

        // Build GPU LODs.
        let mut gpu_lods: Vec<TreeLod> = Vec::with_capacity(group.lods.len());
        for (_, submeshes) in &group.lods {
            let mut submesh_gpu = Vec::with_capacity(submeshes.len());
            for sd in submeshes {
                let mut vertex_data: Vec<f32> = Vec::with_capacity(sd.positions.len() * 8);
                for i in 0..sd.positions.len() {
                    let p = sd.positions[i];
                    let n = sd.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
                    let uv = sd.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
                    vertex_data.extend_from_slice(&[
                        p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1],
                    ]);
                }
                let (vbo, ibo) = unsafe {
                    let mut vbo = 0;
                    let mut ibo = 0;
                    gl::GenBuffers(1, &mut vbo);
                    gl::GenBuffers(1, &mut ibo);
                    gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
                    gl::BufferData(
                        gl::ARRAY_BUFFER,
                        (vertex_data.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                        vertex_data.as_ptr() as *const GLvoid,
                        gl::STATIC_DRAW,
                    );
                    gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ibo);
                    gl::BufferData(
                        gl::ELEMENT_ARRAY_BUFFER,
                        (sd.indices.len() * std::mem::size_of::<u32>()) as GLsizeiptr,
                        sd.indices.as_ptr() as *const GLvoid,
                        gl::STATIC_DRAW,
                    );
                    gl::BindBuffer(gl::ARRAY_BUFFER, 0);
                    gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, 0);
                    (vbo, ibo)
                };
                submesh_gpu.push(TreeSubmeshGpu {
                    vbo,
                    ibo,
                    index_count: sd.indices.len() as GLsizei,
                    material_index: sd.material_index,
                });
            }
            gpu_lods.push(TreeLod { submeshes: submesh_gpu });
        }

        // Pick an LOD to base the collider on: prefer "low" (still 3D, much
        // cheaper than the high-poly tier), fall back to whatever has 3D
        // geometry that isn't the billboard.
        let collider_idx = group
            .lods
            .iter()
            .position(|(name, _)| {
                let lc = name.to_lowercase();
                lc.contains("lod1") || lc.contains("low")
            })
            .or_else(|| {
                group.lods.iter().position(|(name, _)| {
                    let lc = name.to_lowercase();
                    !lc.contains("lod3") && !lc.contains("billboard") && !lc.contains("last")
                })
            })
            .unwrap_or(0);
        let collider_shape = build_collider(&shared_model, &group.lods[collider_idx].1)
            .unwrap_or_else(|| panic!("'{}' failed to build collider trimesh", group.name));

        out.push(TreeKind {
            model: shared_model.clone(),
            lods: gpu_lods,
            collider_shape,
        });
    }
    Ok(out)
}

/// Build a shared trimesh collider from a single LOD's submeshes, skipping
/// any submesh whose material name reads as foliage. The player should bump
/// into the trunk/branches but walk through the canopy.
fn build_collider(
    model: &Model,
    submeshes: &[crate::core::model::SubmeshData],
) -> Option<SharedShape> {
    if submeshes.is_empty() {
        return None;
    }
    let mut verts: Vec<Vector> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();
    let mut offset: u32 = 0;
    for sd in submeshes {
        if is_foliage_material(model, sd.material_index) {
            continue;
        }
        for p in &sd.positions {
            verts.push(Vector::new(p[0], p[1], p[2]));
        }
        for tri in sd.indices.chunks_exact(3) {
            indices.push([tri[0] + offset, tri[1] + offset, tri[2] + offset]);
        }
        offset += sd.positions.len() as u32;
    }
    // Some LODs (typically the billboard tier) only have foliage geometry,
    // which would leave us with nothing. Fall back to *all* submeshes in
    // that case so we still get a sensible collision shape.
    if verts.is_empty() {
        offset = 0;
        for sd in submeshes {
            for p in &sd.positions {
                verts.push(Vector::new(p[0], p[1], p[2]));
            }
            for tri in sd.indices.chunks_exact(3) {
                indices.push([tri[0] + offset, tri[1] + offset, tri[2] + offset]);
            }
            offset += sd.positions.len() as u32;
        }
    }
    if verts.is_empty() || indices.is_empty() {
        return None;
    }
    SharedShape::trimesh_with_flags(verts, indices, TriMeshFlags::FIX_INTERNAL_EDGES).ok()
}

fn is_foliage_material(model: &Model, material_index: usize) -> bool {
    let Some(name) = model.material_name_at(material_index) else {
        return false;
    };
    let lc = name.to_lowercase();
    lc.contains("cluster")
        || lc.contains("leaf")
        || lc.contains("leaves")
        || lc.contains("leav")    // Paine pine names its foliage material "leav"
        || lc.contains("foliage")
        || lc.contains("billboard")
}

/// Stable hash of (x, z) → kind index. Same input always picks the same kind,
/// so a chunk's forest doesn't reshuffle on reload.
fn pick_kind_index(x: f32, z: f32, kind_count: usize) -> usize {
    if kind_count == 0 {
        return 0;
    }
    let mut h = (x.to_bits() as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h ^= (z.to_bits() as u64).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    (h as usize) % kind_count
}

pub struct TreeForest {
    /// `instances_by_kind[k]` — placements assigned to kind `k`. Empty slots
    /// are preserved so indices align with `TreeResources::kinds`.
    instances_by_kind: Vec<Vec<Matrix4<f32>>>,
    center: Point3<f32>,
    gpu: RefCell<Option<TreeForestGpu>>,
}

struct TreeForestGpu {
    kinds: Vec<Option<TreeKindGpu>>,
}

struct TreeKindGpu {
    instance_vbo: GLuint,
    /// `vaos[lod_idx][submesh_idx]`
    vaos: Vec<Vec<GLuint>>,
    instance_count: GLsizei,
}

impl Drop for TreeForestGpu {
    fn drop(&mut self) {
        unsafe {
            for kind in &self.kinds {
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

impl TreeForest {
    pub fn from_placements(placements: &[(f32, f32, f32, f32)]) -> Self {
        if placements.is_empty() {
            return Self {
                instances_by_kind: Vec::new(),
                center: Point3::new(0.0, 0.0, 0.0),
                gpu: RefCell::new(None),
            };
        }
        let kind_count = with_tree_resources(|r| r.kinds.len());
        let mut instances_by_kind: Vec<Vec<Matrix4<f32>>> =
            (0..kind_count).map(|_| Vec::new()).collect();

        for &(x, y, z, yaw) in placements {
            let kind_idx = pick_kind_index(x, z, kind_count);
            let translation = Matrix4::from_translation(Vector3::new(x, y, z));
            let yaw_rot: Matrix4<f32> = Quaternion::from_angle_y(Rad(yaw)).into();
            instances_by_kind[kind_idx].push(translation * yaw_rot);
        }

        let sum: Vector3<f32> = placements
            .iter()
            .fold(Vector3::zero(), |acc, &(x, y, z, _)| {
                acc + Vector3::new(x, y, z)
            });
        let center = Point3::from_vec(sum / placements.len() as f32);

        Self {
            instances_by_kind,
            center,
            gpu: RefCell::new(None),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.instances_by_kind.iter().all(|v| v.is_empty())
    }

    /// `parent` is the chunk's fixed body; parenting the colliders to it lets
    /// them be removed together with the chunk.
    pub fn add_colliders(
        &self,
        placements: &[(f32, f32, f32, f32)],
        scene: &mut Scene,
        parent: Option<rapier3d::prelude::RigidBodyHandle>,
    ) {
        if placements.is_empty() {
            return;
        }
        with_tree_resources(|r| {
            let kind_count = r.kinds.len();
            for &(x, y, z, yaw) in placements {
                let kind_idx = pick_kind_index(x, z, kind_count);
                let translation = Vector::new(x, y, z);
                let rotation = Rotation::from_rotation_y(yaw);
                let pose = Pose::from_parts(translation, rotation);
                let collider = ColliderBuilder::new(r.kinds[kind_idx].collider_shape.clone())
                    .position(pose)
                    .build();
                scene.physics_engine.add_collider(collider, parent);
            }
        });
    }

    fn ensure_gpu(&self) {
        if self.gpu.borrow().is_some() {
            return;
        }
        with_tree_resources(|r| {
            let gpu = build_forest_gpu(&self.instances_by_kind, r);
            *self.gpu.borrow_mut() = Some(gpu);
        });
    }
}

fn build_forest_gpu(
    instances_by_kind: &[Vec<Matrix4<f32>>],
    r: &TreeResources,
) -> TreeForestGpu {
    let mut kinds: Vec<Option<TreeKindGpu>> = Vec::with_capacity(instances_by_kind.len());
    for (kind_idx, instances) in instances_by_kind.iter().enumerate() {
        if instances.is_empty() {
            kinds.push(None);
            continue;
        }
        let kind = &r.kinds[kind_idx];

        // Per-kind instance VBO.
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

        // One VAO per (LOD, submesh) within this kind.
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

        kinds.push(Some(TreeKindGpu {
            instance_vbo,
            vaos: vaos_per_lod,
            instance_count: instances.len() as GLsizei,
        }));
    }
    TreeForestGpu { kinds }
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

impl Component for TreeForest {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn render(
        &self,
        scene: &Scene,
        _: &Entity,
        view_projection: &Matrix4<f32>,
        parent_transform: &Matrix4<f32>,
    ) {
        if self.is_empty() {
            return;
        }
        let Some(skylight) = scene.get_component::<SkyLight>() else {
            return;
        };
        let light_pos = skylight.get_position();

        let camera_pos = scene
            .get_active_camera()
            .map(|c| c.get_camera().get_eye_position());
        let distance = camera_pos
            .map(|p| {
                let d = self.center - p;
                (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
            })
            .unwrap_or(0.0);

        self.ensure_gpu();
        let gpu_borrow = self.gpu.borrow();
        let gpu = gpu_borrow.as_ref().expect("gpu just initialized");

        with_tree_resources(|r| {
            r.shader.bind();
            r.shader.set_uniform_mat4("viewProjection", view_projection);
            r.shader.set_uniform_mat4("parentTransform", parent_transform);
            r.shader
                .set_uniform_3f("lightPosition", light_pos.x, light_pos.y, light_pos.z);

            unsafe {
                gl::Enable(gl::DEPTH_TEST);
                gl::DepthMask(gl::TRUE);
                gl::DepthFunc(gl::LESS);
                gl::Disable(gl::CULL_FACE);
                for (kind_idx, kind_gpu_opt) in gpu.kinds.iter().enumerate() {
                    let Some(kind_gpu) = kind_gpu_opt else { continue };
                    let kind = &r.kinds[kind_idx];
                    let lod_idx = pick_lod_index(distance, kind.lods.len());
                    let lod = &kind.lods[lod_idx];
                    let vaos = &kind_gpu.vaos[lod_idx];
                    for (sub_idx, sub) in lod.submeshes.iter().enumerate() {
                        bind_material_textures(
                            &r.shader,
                            kind.model.material_textures(sub.material_index),
                        );
                        gl::BindVertexArray(vaos[sub_idx]);
                        gl::DrawElementsInstanced(
                            gl::TRIANGLES,
                            sub.index_count,
                            gl::UNSIGNED_INT,
                            std::ptr::null(),
                            kind_gpu.instance_count,
                        );
                    }
                }
                gl::BindVertexArray(0);
                gl::Enable(gl::CULL_FACE);
                gl::Disable(gl::DEPTH_TEST);
            }
        });
    }

    fn handle_event(&mut self, _: &mut glfw::Glfw, _: &mut glfw::Window, _: &WindowEvent) {}
}

fn bind_material_textures(
    shader: &Shader,
    textures: Option<&HashMap<TextureType, Texture>>,
) {
    let Some(textures) = textures else {
        return;
    };
    // PBR-authored materials (Acer pack) expose albedo as `BaseColor`; legacy
    // ones (Paine pine, the older Tree.fbx) use `Diffuse`. Pick whichever the
    // material actually carries — and if neither is tagged, fall back to any
    // non-normal texture so we don't inherit the previous draw's binding.
    let diffuse = textures
        .get(&TextureType::Diffuse)
        .or_else(|| textures.get(&TextureType::BaseColor))
        .or_else(|| {
            textures.iter().find_map(|(t, tex)| {
                if matches!(
                    t,
                    TextureType::Normals
                        | TextureType::NormalCamera
                        | TextureType::Height
                        | TextureType::Roughness
                        | TextureType::Metalness
                        | TextureType::AmbientOcclusion
                ) {
                    None
                } else {
                    Some(tex)
                }
            })
        });
    if let Some(diffuse) = diffuse {
        unsafe { gl::ActiveTexture(gl::TEXTURE0) };
        diffuse.bind();
        shader.set_uniform_1i("texture_diffuse", 0);
    }
    let normal = textures
        .get(&TextureType::Normals)
        .or_else(|| textures.get(&TextureType::NormalCamera))
        .or_else(|| textures.get(&TextureType::Height));
    if let Some(normal) = normal {
        unsafe { gl::ActiveTexture(gl::TEXTURE1) };
        normal.bind();
        shader.set_uniform_1i("texture_normals", 1);
    }
}
