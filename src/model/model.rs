use std::{collections::HashMap, rc::Rc};

use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Vector4};
use russimp::{material::{DataContent, TextureType}, node::Node, scene::{PostProcess, Scene}};

use crate::{camera::{Camera, Projection}, line::{Line, LineRenderer}, shader::{DynamicVertexArray, Shader, VertexAttributes}, texture::Texture};

use super::{Bone, Model, ModelMesh, ModelMeshVertex};
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
            let mut root_bone = None;
            if let Some(root_node) = &self.model.root {
                for node in root_node.children.borrow().iter() {
                    for (id, bone) in mesh.bones.iter().enumerate() {
                        if bone.name != node.name {
                            continue;
                        }
                        root_bone = Some(Bone {
                            id,
                            name: bone.name.clone(),
                            transformation_matrix: node.transformation.to_matrix_4(),
                            offset_matrix: bone.offset_matrix.to_matrix_4(),
                            weights: bone.weights.iter().map(|w| (w.vertex_id, w.weight)).collect(),
                            children: self.get_child_bones(node, &mesh.bones, Matrix4::identity())
                        });
                    }
                }
            }
            let model_mesh = ModelMesh::new(
                mesh.vertices.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect(),
                mesh.faces.iter().flat_map(|f| vec![f.0[0], f.0[1], f.0[2]]).collect::<Vec<u32>>(),
                mesh.normals.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect(),
                texture_coords.clone(),
                root_bone
            );
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
                for (id, bone) in bones.iter().enumerate() {
                    if bone.name != child.name {
                        continue;
                    }
                    children.push(Bone {
                        id,
                        name: bone.name.clone(),
                        transformation_matrix: offset_matrix * child.transformation.to_matrix_4(),
                        offset_matrix: bone.offset_matrix.to_matrix_4(),
                        weights: bone.weights.iter().map(|w| (w.vertex_id, w.weight)).collect(),
                        children: self.get_child_bones(child, bones, Matrix4::identity())
                    });
                }
            } else if let Some(child_bones) = self.get_child_bones(child, bones, offset_matrix * child.transformation.to_matrix_4()) {
                children.extend(child_bones);
            }
        }
        Some(children)
    }

    fn get_bone_transformations(bone: &Bone, parent_transform: Matrix4<f32>) -> Vec<Matrix4<f32>> {
        let mut transformations = Vec::<Matrix4<f32>>::new();
        let global_transformation = parent_transform * bone.transformation_matrix;
        transformations.push(global_transformation * bone.offset_matrix);
        if let Some(children) = &bone.children {
            for child in children {
                transformations.extend(Self::get_bone_transformations(child, global_transformation));
            }
        }
        transformations
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        for mesh in self.meshes.values_mut() {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            self.shader.bind();
            self.shader.set_uniform_mat4("view", &camera.calc_matrix());
            self.shader.set_uniform_mat4("projection", &projection.calc_matrix());
            if let Some(root_bone) = &mesh.root_bone {
                self.shader.set_uniform_mat4_array("boneTransforms", &Model::get_bone_transformations(root_bone, Matrix4::identity()));
            }
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
            mesh.render(&self.shader, (self.position.x, self.position.y, self.position.z), Some(self.scale));
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
    }

    pub fn render_bones(&self, line_renderer: &LineRenderer, camera: &Camera, projection: &Projection) {
        let root = Matrix4::from_translation(self.position) * Matrix4::from_scale(self.scale);
        let mut lines: Vec<Line> = Vec::new();
        for mesh in self.meshes.values() {
            if let Some(root_bone) = &mesh.root_bone {
                lines.extend(self.render_child_bones(root_bone, line_renderer, camera, projection, root));
            }
        }
        line_renderer.render_lines(camera, projection, &lines, cgmath::Vector3::new(1.0, 0.0, 0.0), true);
    }

    fn render_child_bones(&self, bone: &Bone, line_renderer: &LineRenderer, camera: &Camera, projection: &Projection, root: cgmath::Matrix4<f32>) -> Vec<Line> {
        let bone_matrix = bone.transformation_matrix;
        let position = root * bone_matrix;
        let pos_vec = (position * Vector4::new(0.0,0.0,0.0,1.0)).truncate();
        let root_vec = (root * Vector4::new(0.0,0.0,0.0,1.0)).truncate();
        let direction = pos_vec - root_vec;
        let mut lines = vec![Line {
            position: Point3::new(root_vec.x, root_vec.y, root_vec.z),
            direction: direction.normalize(),
            length: direction.magnitude(),
        }];
        if let Some(children) = &bone.children {
            for child in children {
                lines.extend(self.render_child_bones(child, line_renderer, camera, projection, position));
            }
        }
        lines
    }
}

impl ModelMesh {
    pub fn new(vertices: Vec<f32>, indices: Vec<u32>, normals: Vec<f32>, texture_coords: Vec<f32>, root_bone: Option<Bone>) -> ModelMesh {
        let mut mesh_vertices = Vec::<ModelMeshVertex>::new();
        if let Some(root_bone) = &root_bone {
            let bone_weights = ModelMesh::get_bone_weights(root_bone.clone());
            for i in 0..vertices.len() / 3 {
                let weights = &bone_weights[i];
                mesh_vertices.push(ModelMeshVertex {
                    position: (vertices[i * 3], vertices[i * 3 + 1], vertices[i * 3 + 2]),
                    normal: (normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]),
                    texture_coords: (texture_coords[i * 2], texture_coords[i * 2 + 1]),
                    bone_ids: (
                        if weights.len() >= 1 { weights[0].0 as u32 } else { 0 },
                        if weights.len() >= 2 { weights[1].0 as u32 } else { 0 },
                        if weights.len() >= 3 { weights[2].0 as u32 } else { 0 },
                        if weights.len() >= 4 { weights[3].0 as u32 } else { 0 }
                    ),
                    bone_weights: (
                        if weights.len() >= 1 { weights[0].1 } else { 0.0 },
                        if weights.len() >= 2 { weights[1].1 } else { 0.0 },
                        if weights.len() >= 3 { weights[2].1 } else { 0.0 },
                        if weights.len() >= 4 { weights[3].1 } else { 0.0 }
                    ),
                });
            }
        }
        ModelMesh {
            root_bone,
            indices,
            vertices: mesh_vertices,
            vertex_array: None,
        }
    }

    pub fn render(&self, shader: &Shader, position: (f32, f32, f32), scale: Option<f32>) {
        if let Some(vertex_array) = &self.vertex_array {
            unsafe {
                gl::Enable(gl::DEPTH_TEST);
                gl::Enable(gl::CULL_FACE);
            }
            vertex_array.bind();
            let mut model = cgmath::Matrix4::from_translation(cgmath::Vector3::new(position.0, position.1, position.2));
            if let Some(scale) = scale {
                model = model * cgmath::Matrix4::from_scale(scale);
            }
            shader.set_uniform_mat4("model", &model);
            unsafe {
                gl::DrawElements(gl::TRIANGLES, self.indices.len() as i32, gl::UNSIGNED_INT, std::ptr::null());
                DynamicVertexArray::<ModelMeshVertex>::unbind();
                gl::Disable(gl::DEPTH_TEST);
                gl::Disable(gl::CULL_FACE);
            }
        }
    }

    pub fn is_buffered(&self) -> bool {
        self.vertex_array.is_some()
    }

    pub fn buffer_data(&mut self) {
        let mut vertex_array = DynamicVertexArray::<ModelMeshVertex>::new();
        vertex_array.buffer_data_dyn(&self.vertices, &Some(self.indices.clone()));
        self.vertex_array = Some(vertex_array);
    }

    fn get_bone_weights(root_bone: Bone) -> Vec<Vec<(usize, f32)>> {
        let bones = root_bone.get_as_vec();
        let mut bone_weights: Vec<Vec<(usize, f32)>> = Vec::new();
        for bone in bones {
            for weight in &bone.weights {
                if weight.1 == 0.0 {
                    continue;
                }
                if bone_weights.len() <= weight.0 as usize {
                    bone_weights.resize(weight.0 as usize + 1, Vec::new());
                }
                bone_weights[weight.0 as usize].push((bone.id, weight.1));
            }

        }
        bone_weights
    }
}

impl Bone {
    pub fn get_as_vec(&self) -> Vec<Bone> {
        let mut bones = Vec::<Bone>::new();
        bones.push(self.clone());
        if let Some(children) = &self.children {
            for child in children {
                bones.extend(child.get_as_vec());
            }
        }
        bones
    }
}

impl VertexAttributes for ModelMeshVertex {
    fn get_vertex_attributes() -> Vec<(usize, gl::types::GLuint)> {
        vec![
            (3, gl::FLOAT),
            (3, gl::FLOAT),
            (2, gl::FLOAT),
            (4, gl::UNSIGNED_INT),
            (4, gl::FLOAT),
        ]
    }
}