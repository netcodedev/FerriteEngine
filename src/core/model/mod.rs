use std::collections::HashMap;

use cgmath::{Matrix4, Point3, Quaternion, Vector3};
use russimp::{material::TextureType, scene::Scene};

use crate::core::renderer::{
    shader::{DynamicVertexArray, Shader},
    texture::Texture,
};

mod animation;
pub mod animation_graph;
mod bone;
mod channel;
mod model;
mod model_mesh;
mod pose;

pub struct Model {
    model: Scene,
    meshes: HashMap<String, ModelMesh>,
    shader: Shader,
    textures: HashMap<TextureType, Texture>,
    position: Point3<f32>,
    scale: f32,
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
