pub mod dual_contouring;
pub mod terrain;

use std::{collections::HashMap, sync::mpsc};

use ndarray::ArrayBase;

use crate::{shader::{DynamicVertexArray, Shader}, terrain::ChunkBounds};

const CHUNK_SIZE: usize = 128;
const ISO_VALUE: f32 = 0.3;

pub struct Terrain {
    pub chunks: HashMap<ChunkBounds, Chunk>,
    chunk_receiver: mpsc::Receiver<Chunk>,
    shader: Shader,
}

pub struct Chunk {
    position: (f32, f32, f32),
    blocks: ArrayBase<ndarray::OwnedRepr<f32>, ndarray::Dim<[usize; 3]>>,
    mesh: Option<ChunkMesh>,
}

pub struct ChunkMesh {
    vertex_array: Option<DynamicVertexArray<Vertex>>,
    indices: Option<Vec<u32>>,
    vertices: Vec<Vertex>,
}

#[derive(Clone, Copy)]
#[warn(dead_code)]
#[repr(C)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}