use std::{collections::HashMap, rc::Rc};

use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Vector4};
use russimp::{material::{DataContent, TextureType}, node::Node, scene::{PostProcess, Scene}};

use crate::{camera::{Camera, Projection}, line::{Line, LineRenderer}, mesh::Mesh, shader::Shader, texture::Texture};

use super::{Bone, Model, ModelMesh};
use crate::utils::ToMatrix4;

impl Model {
    pub fn new(path: &str) -> Result<Model, Box<dyn std::error::Error>> {
        let scene = Scene::from_file(path, vec![
                PostProcess::Triangulate, 
                // PostProcess::JoinIdenticalVertices,
                PostProcess::GenerateSmoothNormals,
                PostProcess::FlipUVs,
            ])?;
        let shader: Shader = Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        Ok(Model {
            model: scene,
            meshes: HashMap::<String, ModelMesh>::new(),
            shader,
            textures: HashMap::<TextureType, Texture>::new(),
            position: cgmath::Vector3::new(0.0, 91.0, 0.0),
            scale: 0.01,
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
        for mesh in &self.model.meshes {
            let mut model_mesh = ModelMesh {
                mesh: Mesh::new(
                    mesh.vertices.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect(),
                    Some(mesh.faces.iter().flat_map(|f| vec![f.0[0], f.0[1], f.0[2]]).collect::<Vec<u32>>()),
                    Some(mesh.normals.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect()),
                    Some(texture_coords.clone()),
                ),
                root_bone: None
            };
            let mut root_bone = None;
            if let Some(root_node) = &self.model.root {
                for node in root_node.children.borrow().iter() {
                    for bone in &mesh.bones {
                        if bone.name != node.name {
                            continue;
                        }
                        root_bone = Some(Bone {
                            name: bone.name.clone(),
                            transformation_matrix: node.transformation.to_matrix_4(),
                            offset_matrix: bone.offset_matrix.to_matrix_4(),
                            weights: bone.weights.iter().map(|w| (w.vertex_id, w.weight)).collect(),
                            children: self.get_child_bones(node, &mesh.bones, Matrix4::identity())
                        });
                    }
                }
            }
            model_mesh.root_bone = root_bone;
            self.meshes.insert(mesh.name.clone(), model_mesh);
        }
    }

    fn get_child_bones(&self, node: &Rc<Node>, bones: &Vec<russimp::bone::Bone>, offset_matrix: Matrix4<f32>) -> Option<Vec<Bone>> {
        if node.children.borrow().len() == 0 {
            return None;
        }
        let mut children = Vec::<Bone>::new();
        for child in node.children.borrow().iter() {
            if bones.iter().any(|b| b.name == child.name) {
                let bone = bones.iter().find(|b| b.name == child.name).unwrap();
                children.push(Bone {
                    name: bone.name.clone(),
                    transformation_matrix: offset_matrix * child.transformation.to_matrix_4(),
                    offset_matrix: bone.offset_matrix.to_matrix_4(),
                    weights: bone.weights.iter().map(|w| (w.vertex_id, w.weight)).collect(),
                    children: self.get_child_bones(child, bones, Matrix4::identity())
                });
            } else if let Some(child_bones) = self.get_child_bones(child, bones, offset_matrix * child.transformation.to_matrix_4()) {
                children.extend(child_bones);
            }
        }
        Some(children)
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        for mesh in self.meshes.values_mut() {
            if !mesh.mesh.is_buffered() {
                mesh.mesh.buffer_data();
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
            mesh.mesh.render(&self.shader, (self.position.x, self.position.y, self.position.z), Some(self.scale));
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
    }

    pub fn render_bones(&self, line_renderer: &LineRenderer, camera: &Camera, projection: &Projection) {
        let root = Matrix4::from_translation(self.position) * Matrix4::from_scale(self.scale);
        for mesh in self.meshes.values() {
            if let Some(root_bone) = &mesh.root_bone {
                self.render_child_bones(root_bone, line_renderer, camera, projection, root);
            }
        }
    }

    fn render_child_bones(&self, bone: &Bone, line_renderer: &LineRenderer, camera: &Camera, projection: &Projection, root: cgmath::Matrix4<f32>) {
        let bone_matrix = bone.transformation_matrix;
        let position = root * bone_matrix;
        let offset_pos = position * bone.offset_matrix;
        let pos_vec = (position * Vector4::new(0.0,0.0,0.0,1.0)).truncate();
        let root_vec = (root * Vector4::new(0.0,0.0,0.0,1.0)).truncate();
        let offset_vec = (offset_pos * Vector4::new(0.0,0.0,0.0,1.0)).truncate();
        let direction = pos_vec - root_vec;
        let offset_dir = offset_vec - root_vec;
        line_renderer.render(camera, projection, &Line {
                position: Point3::new(root_vec.x, root_vec.y, root_vec.z),
                direction: direction.normalize(),
                length: direction.magnitude(),
            }, 
            cgmath::Vector3::new(1.0, 0.0, 0.0),
            true
        );
        line_renderer.render(camera, projection, &Line {
                position: Point3::new(root_vec.x, root_vec.y, root_vec.z),
                direction: offset_dir.normalize(),
                length: offset_dir.magnitude(),
            }, 
            cgmath::Vector3::new(0.0, 0.0, 1.0),
            true
        );
        if let Some(children) = &bone.children {
            for child in children {
                self.render_child_bones(child, line_renderer, camera, projection, position);
            }
        }
    }
}