use std::collections::HashMap;

use russimp::{material::{DataContent, TextureType}, scene::{PostProcess, Scene}};

use crate::{camera::{Camera, Projection}, mesh::Mesh, shader::Shader, texture::Texture};

pub struct Model {
    model: Scene,
    mesh: Option<Mesh>,
    shader: Shader,
    textures: HashMap<TextureType, Texture>
}

impl Model {
    pub fn new(path: &str) -> Result<Model, Box<dyn std::error::Error>> {
        let scene = Scene::from_file(path, vec![
                PostProcess::Triangulate, 
                // PostProcess::JoinIdenticalVertices,
                PostProcess::GenerateSmoothNormals,
                PostProcess::FlipUVs,
            ])?;
        let shader: Shader = Shader::new(include_str!("shaders/model_vertex.glsl"), include_str!("shaders/model_fragment.glsl"));
        Ok(Model {
            model: scene,
            mesh: None,
            shader,
            textures: HashMap::<TextureType, Texture>::new()
        })
    }

    pub fn init (&mut self) {
        let materials = &self.model.materials;
        for material in materials {
            for (tex_type, texture) in &material.textures {
                let tex = texture.borrow();
                if let DataContent::Bytes(texture_data) = &tex.data {
                    let data = image::load_from_memory(texture_data.as_slice()).unwrap();
                    self.textures.insert(tex_type.clone(), Texture::from_data(data.width(), data.height(), data.to_rgba8().into_raw()));
                }
            }
        }
        let texture_coords: Vec<f32> = self.model.meshes[0].texture_coords.iter().flat_map(|tx| {
            if let Some(tx) = tx {
                let coords: Vec<f32> = tx.iter().flat_map(|v| vec![v.x, v.y]).collect();
                coords
            } else {
                Vec::<f32>::new()
            }
        }).collect();
        let mesh = Mesh::new(
            self.model.meshes[0].vertices.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect(),
            Some(self.model.meshes[0].faces.iter().flat_map(|f| vec![f.0[0], f.0[1], f.0[2]]).collect::<Vec<u32>>()),
            Some(self.model.meshes[0].normals.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect()),
            Some(texture_coords),
            None
        );
        self.mesh = Some(mesh);
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        if let Some(mesh) = &mut self.mesh {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            self.shader.bind();
            self.shader.set_uniform_mat4("view", &camera.calc_matrix());
            self.shader.set_uniform_mat4("projection", &projection.calc_matrix());
            for (i, (texture_type, texture)) in self.textures.iter().enumerate() {
                unsafe { gl::ActiveTexture(gl::TEXTURE0 + i as u32) };
                texture.bind();
                match texture_type {
                    TextureType::Diffuse => self.shader.set_uniform_1i("texture_diffuse", i as i32),
                    TextureType::Shininess => self.shader.set_uniform_1i("texture_shininess", i as i32),
                    TextureType::Normals => self.shader.set_uniform_1i("texture_normal", i as i32),
                    TextureType::Specular => self.shader.set_uniform_1i("texture_specular", i as i32),
                    _ => {}
                }
            }
            unsafe { gl::Disable(gl::CULL_FACE) };
            mesh.render(&self.shader, (0.0, 91.0, 0.0), Some(0.01));
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
    }
}