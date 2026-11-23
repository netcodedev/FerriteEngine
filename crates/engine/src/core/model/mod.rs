use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use cgmath::{Matrix4, Point3, Quaternion, Vector3};
use russimp_ng::scene::Scene;

use crate::core::renderer::{
    shader::DynamicVertexArray,
    texture::Texture,
};

mod animation;
pub mod anchor;
pub mod animation_graph;
pub mod ik;
mod bone;
mod channel;
pub mod clip_file;
pub mod generated;
pub mod grip;
pub mod material_maps;
mod model;
pub(crate) mod model_geometry;
pub mod model_materials;
mod model_mesh;
mod model_skeleton;
pub mod outline;
mod pose;
pub mod background;
mod prepared;
pub use prepared::PreparedModel;

pub struct Model {
    model: Scene,
    /// By name, in file order. Names need not be unique (a glTF mesh with
    /// several primitives, an FBX mesh split per material): lookups by
    /// name (variants, parts) take every mesh of that name.
    meshes: Vec<(String, ModelMesh)>,
    /// Materials by the file's material index. A mesh selects its own via
    /// `ModelMesh::material_index`.
    materials: Vec<crate::core::material::Material>,
    /// What the loader made of the file's materials, before material
    /// assets replaced them (see `material::asset::apply_to_model`).
    file_materials: Vec<crate::core::material::Material>,
    /// Texture files several materials refer to, loaded once: the
    /// materials hold `share`d handles on these (see `Model::init`).
    shared_textures: Vec<std::rc::Rc<Texture>>,
    /// Every map `init` found (the owning handles): materials hold shared
    /// handles, so replacing a material (with a material asset's) never
    /// frees a texture another one still draws with.
    map_textures: Vec<std::rc::Rc<Texture>>,
    /// Maps a `PreparedModel` decoded already, for `init` to take.
    prepared_images: Option<prepared::DecodedMaps>,
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
    /// Bones drawn collapsed to their joint, with everything below them,
    /// when the model is drawn to the screen (shadows keep them), e.g. the
    /// head of a first-person player so the camera does not see into it.
    hidden_bones: Vec<String>,
    /// Drawn as a see-through silhouette of this colour; see
    /// `Model::set_tint`.
    tint: Option<[f32; 4]>,
    /// Skinning matrices per mesh index, as last uploaded: without (0) and
    /// with (1) the hidden bones collapsed. A frame draws the model in up
    /// to about nine passes with the same pose; emptied when the pose,
    /// the meshes or the hidden bones change.
    bone_cache: [RefCell<HashMap<usize, Vec<Matrix4<f32>>>>; 2],
    /// Changes with `bone_cache` (see `Model::pose_revision`).
    pose_revision: u64,
    /// Box around every mesh in model space, for frustum culling. `None`
    /// for skinned models, whose bones can move vertices anywhere, and
    /// before `init`.
    cull_bounds: Option<(Point3<f32>, Point3<f32>)>,
    /// Where each mesh was drawn in the last view, for the motion vectors
    /// of TAA (see `renderer::taa::motion`).
    motion: crate::core::renderer::taa::motion::History,
    /// Counts the CPU copies of the geometry in the RAM map.
    tally: crate::core::memory::Tally,
}

/// Raw per-submesh geometry extracted from a `Model` for callers that need
/// to build their own GPU buffers (e.g. a custom instanced renderer).
/// Positions are already pre-multiplied by `Model::scale`.
pub struct SubmeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// RGBA8, red lowest (see `model_geometry::pack_color`).
    pub colors: Vec<u32>,
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
    /// RGBA8, red lowest.
    color: u32,
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
    /// The channel of the topmost animated node (the hips), if known.
    root_channel: Option<String>,
}

#[derive(Clone)]
struct Channel {
    bone_id: String,
    position_keys: Vec<(f32, cgmath::Vector3<f32>)>,
    rotation_keys: Vec<(f32, cgmath::Quaternion<f32>)>,
    scaling_keys: Vec<(f32, cgmath::Vector3<f32>)>,
}
