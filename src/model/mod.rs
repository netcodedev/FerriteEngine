use std::collections::HashMap;

use cgmath::Matrix4;
use russimp::{material::TextureType, scene::Scene};

use crate::{mesh::Mesh, shader::Shader, texture::Texture};

pub mod model;

pub struct Model {
    model: Scene,
    meshes: HashMap<String, ModelMesh>,
    shader: Shader,
    textures: HashMap<TextureType, Texture>,
    position: cgmath::Vector3<f32>,
    scale: f32,
}

struct ModelMesh {
    mesh: Mesh,
    root_bone: Option<Bone>
}

#[allow(dead_code)]
#[derive(Clone)]
struct Bone {
    name: String,
    transformation_matrix: Matrix4<f32>,
    offset_matrix: Matrix4<f32>,
    weights: Vec<(u32, f32)>,
    children: Option<Vec<Bone>>
}