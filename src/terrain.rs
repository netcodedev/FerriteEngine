use crate::line::Line;
use crate::mesh::{Chunk, ChunkBounds};
use crate::camera::{Camera, Projection};
use crate::shader::create_shader;

use std::sync::mpsc;
use std::thread;
use std::collections::HashMap;

pub struct Terrain {
    pub chunks: HashMap<ChunkBounds, Chunk>,
    chunk_receiver: mpsc::Receiver<Chunk>,
    shader: u32,
}

impl Terrain {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let origin = Chunk::new((0.0, 0.0, 0.0));
        tx.send(origin).unwrap();

        let shader = create_shader(include_str!("shaders/vertex.glsl"), include_str!("shaders/fragment.glsl"));

        let tx1 = tx.clone();
        let tx2 = tx.clone();
        let tx3 = tx.clone();
        let tx4 = tx.clone();
        const RADIUS: i32 = 5;
        let _ = thread::spawn(move || chunkloader(RADIUS,1,1,tx1));
        let _ = thread::spawn(move || chunkloader(RADIUS,-1,1,tx2));
        let _ = thread::spawn(move || chunkloader(RADIUS,1,-1,tx3));
        let _ = thread::spawn(move || chunkloader(RADIUS,-1,-1,tx4));

        Self {
            chunks: HashMap::<ChunkBounds, Chunk>::new(),
            chunk_receiver: rx,
            shader,
        }
    }

    pub fn update(&mut self) {
        if let Ok(chunk) = self.chunk_receiver.try_recv() {
            self.chunks.insert(chunk.get_bounds(), chunk);
        }
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        for (_, chunk) in &mut self.chunks {
            chunk.render(camera, projection, self.shader);
        }
    }

    pub fn process_line(&mut self, line: Option<(Line, glfw::MouseButton)>) {
        if let Some((line, button)) = line {
            for chunk_bounds in ChunkBounds::get_chunk_bounds_on_line(&line) {
                if let Some(chunk) = self.chunks.get_mut(&chunk_bounds) {
                    if chunk.process_line(&line, &button) {
                        break;
                    }
                }
            }
        }
    }
}

pub fn chunkloader(radius: i32, x_dir: i32, z_dir: i32, tx: mpsc::Sender<Chunk>) {
    let mut x: i32 = 1;
    let mut z: i32 = 0;

    loop {
        if x > radius {
            break;
        }
        let new_chunk: Chunk;
        if z_dir > 0 {
            new_chunk = Chunk::new(((x * x_dir) as f32, 0.0, z as f32));
        } else {
            new_chunk = Chunk::new(((z * z_dir) as f32, 0.0, (x * x_dir) as f32));
        }
        
        let result = tx.send(new_chunk);
        if result.is_err() {
            break;
        }

        z = -z;
        if z == -x*z_dir {
            x += 1;
            z = 0;
        } else if z >= 0 {
            z += 1;
        }
    }
}