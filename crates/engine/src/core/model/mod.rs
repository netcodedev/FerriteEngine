use std::collections::{HashMap, HashSet};

use cgmath::{Matrix4, Point3, Quaternion, Vector3};
use russimp_ng::{material::TextureType, scene::Scene};

use crate::core::renderer::{
    shader::{DynamicVertexArray, Shader},
    texture::Texture,
};

mod animation;
pub mod animation_graph;
mod bone;
mod channel;
pub mod material_maps;
mod model;
mod model_mesh;
pub mod outline;
mod pose;

pub struct Model {
    model: Scene,
    meshes: HashMap<String, ModelMesh>,
    shader: Shader,
    /// Textures grouped by material index. A mesh selects its set via
    /// `ModelMesh::material_index`. Storing them in a single flat map (as the
    /// older code did) caused later materials to overwrite earlier ones so
    /// every mesh ended up sampling the last-loaded texture set.
    textures: Vec<HashMap<TextureType, Texture>>,
    /// File name without extension; names the variant of a model that holds
    /// just one.
    name: String,
    /// Directory the model was loaded from (relative to the project root).
    /// Used as a search location when resolving external texture references
    /// — many FBX/GLTF files ship their maps next to the model file.
    model_dir: String,
    pub position: Point3<f32>,
    scale: f32,
    /// Names of the meshes that are drawn; all of them without. Set by
    /// `show_variant` for files that hold several variants or LODs.
    visible: Option<HashSet<String>>,
}

/// Raw per-submesh geometry extracted from a `Model` for callers that need
/// to build their own GPU buffers (e.g. a custom instanced renderer).
/// Positions are already pre-multiplied by `Model::scale`.
pub struct SubmeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub material_index: usize,
}

/// One variant of a model (e.g. one tree of a pack) with the geometry of
/// its discrete LOD tiers. See `outline::group_nodes` for how a file is
/// split into variants.
pub struct TreeGroup {
    pub name: String,
    /// (LOD name, submeshes) in scene-graph order. Caller is responsible for
    /// sorting by detail level.
    pub lods: Vec<(String, Vec<SubmeshData>)>,
}

pub struct ModelBuilder {
    model: Model,
}

#[derive(Debug, Clone)]
#[repr(C)]
struct ModelMeshVertex {
    position: (f32, f32, f32),
    normal: (f32, f32, f32),
    texture_coords: (f32, f32),
    bone_ids: (u32, u32, u32, u32),
    bone_weights: (f32, f32, f32, f32),
}

struct ModelMesh {
    vertex_array: Option<DynamicVertexArray<ModelMeshVertex>>,
    indices: Vec<u32>,
    vertices: Vec<ModelMeshVertex>,
    root_bone: Option<Bone>,
    material_index: usize,
}

#[derive(Clone)]
struct Bone {
    id: usize,
    name: String,
    offset_matrix: Matrix4<f32>,
    weights: Vec<(u32, f32)>,
    children: Option<Vec<Bone>>,
    current_transform: Matrix4<f32>,
    last_translation: Vector3<f32>,
}

#[derive(Clone)]
pub struct LocalTransform {
    translation: Vector3<f32>,
    rotation: Quaternion<f32>,
    scale: Vector3<f32>,
}

pub struct Pose {
    transforms: HashMap<String, LocalTransform>,
    pub cycle_completed: bool,
    pub transition_finished: bool,
    pub translation_errors: HashMap<String, Vector3<f32>>,
}

#[derive(Clone)]
pub struct Animation {
    name: String,
    pub duration: f32,
    pub ticks_per_second: f32,
    channels: HashMap<String, Channel>,
}

#[derive(Clone)]
struct Channel {
    bone_id: String,
    position_keys: Vec<(f32, cgmath::Vector3<f32>)>,
    rotation_keys: Vec<(f32, cgmath::Quaternion<f32>)>,
    scaling_keys: Vec<(f32, cgmath::Vector3<f32>)>,
}
