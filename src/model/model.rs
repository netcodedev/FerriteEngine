use std::{collections::HashMap, rc::Rc};

use cgmath::{InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4, Zero};
use russimp::{
    material::{DataContent, TextureType},
    node::Node,
    scene::{PostProcess, Scene},
};

use crate::{
    core::camera::{Camera, Projection},
    renderer::{
        line::{Line, LineRenderer},
        shader::Shader,
        texture::Texture,
    },
};

use super::{Animation, Bone, Model, ModelBuilder, ModelMesh};
use crate::core::utils::ToMatrix4;

impl Model {
    pub fn new(path: &str) -> Result<Model, Box<dyn std::error::Error>> {
        let scene = Scene::from_file(
            format!("assets/models/{path}").as_str(),
            vec![
                PostProcess::Triangulate,
                // PostProcess::JoinIdenticalVertices,
                PostProcess::GenerateSmoothNormals,
                PostProcess::FlipUVs,
            ],
        )?;
        let shader: Shader =
            Shader::new(include_str!("vertex.glsl"), include_str!("fragment.glsl"));
        Ok(Model {
            model: scene,
            meshes: HashMap::<String, ModelMesh>::new(),
            animations: HashMap::<String, Animation>::new(),
            current_animations: Vec::new(),
            sync_animations: false,
            shader,
            textures: HashMap::<TextureType, Texture>::new(),
            position: cgmath::Vector3::new(-121.0, 50.5, -32.0),
            scale: 0.01,
        })
    }

    pub fn init(&mut self) {
        let materials = &self.model.materials;
        for material in materials {
            for (tex_type, texture) in &material.textures {
                let tex = texture.borrow();
                if let DataContent::Bytes(texture_data) = &tex.data {
                    let data = image::load_from_memory(texture_data.as_slice()).unwrap();
                    self.textures.insert(
                        tex_type.clone(),
                        Texture::from_data(data.width(), data.height(), data.to_rgba8().into_raw()),
                    );
                }
            }
        }
        let texture_coords: Vec<f32> = self.model.meshes[0]
            .texture_coords
            .iter()
            .flat_map(|tx| {
                if let Some(tx) = tx {
                    let coords: Vec<f32> = tx.iter().flat_map(|v| vec![v.x, v.y]).collect();
                    coords
                } else {
                    Vec::<f32>::new()
                }
            })
            .collect();
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
                            current_transform: node.transformation.to_matrix_4(),
                            offset_matrix: bone.offset_matrix.to_matrix_4(),
                            weights: bone
                                .weights
                                .iter()
                                .map(|w| (w.vertex_id, w.weight))
                                .collect(),
                            children: self.get_child_bones(node, &mesh.bones, Matrix4::identity()),
                            current_animations: Vec::new(),
                            current_animation_time: Vec::new(),
                            last_translation: Vector3::zero(),
                        });
                    }
                }
            }
            let model_mesh = ModelMesh::new(
                mesh.vertices
                    .iter()
                    .flat_map(|v| vec![v.x, v.y, v.z])
                    .collect(),
                mesh.faces
                    .iter()
                    .flat_map(|f| vec![f.0[0], f.0[1], f.0[2]])
                    .collect::<Vec<u32>>(),
                mesh.normals
                    .iter()
                    .flat_map(|v| vec![v.x, v.y, v.z])
                    .collect(),
                texture_coords.clone(),
                root_bone,
            );
            self.meshes.insert(mesh.name.clone(), model_mesh);
        }
    }

    pub fn add_animation(&mut self, animation: Animation) {
        self.animations.insert(animation.name.clone(), animation);
    }

    pub fn play_animation(&mut self, name: &str) {
        if let Some(animation) = self.animations.get(name) {
            self.current_animations = vec![animation.clone()];
            for mesh in self.meshes.values_mut() {
                if let Some(root_bone) = &mut mesh.root_bone {
                    root_bone.reset();
                    root_bone.set_animation_channel(Some(&animation.channels), 1.0, 0.0);
                }
            }
        } else {
            self.current_animations = Vec::new();
            for mesh in self.meshes.values_mut() {
                if let Some(root_bone) = &mut mesh.root_bone {
                    root_bone.reset();
                    root_bone.set_animation_channel(None, 1.0, 0.0);
                }
            }
        }
    }

    pub fn blend_animations(&mut self, name1: &str, name2: &str, weight: f32, sync: bool) {
        if let Some(animation1) = self.animations.get(name1) {
            if let Some(animation2) = self.animations.get(name2) {
                self.current_animations = vec![animation1.clone(), animation2.clone()];
                self.sync_animations = sync;
                for mesh in self.meshes.values_mut() {
                    if let Some(root_bone) = &mut mesh.root_bone {
                        root_bone.reset();
                        root_bone.set_animation_channel(
                            Some(&animation1.channels),
                            1.0 - weight,
                            0.0,
                        );
                        root_bone.set_animation_channel(Some(&animation2.channels), weight, 0.0);
                    }
                }
            }
        }
    }

    pub fn update_and_render(&mut self, delta_time: f32, camera: &Camera, projection: &Projection) {
        let mut root_translation = Vector3::zero();
        for mesh in self.meshes.values_mut() {
            if let Some(root_bone) = &mut mesh.root_bone {
                let animation_data = self
                    .current_animations
                    .iter()
                    .map(|a| (delta_time * a.ticks_per_second, a.duration))
                    .collect();
                if self.current_animations.len() > 0 {
                    let delta =
                        root_bone.update_animation(animation_data, self.sync_animations, true);
                    root_translation += delta;
                }
            }
        }
        self.position += root_translation * self.scale;
        self.render(camera, projection);
    }

    pub fn render(&mut self, camera: &Camera, projection: &Projection) {
        for mesh in self.meshes.values_mut() {
            if !mesh.is_buffered() {
                mesh.buffer_data();
            }
            self.shader.bind();
            self.shader.set_uniform_mat4("view", &camera.calc_matrix());
            self.shader
                .set_uniform_mat4("projection", &projection.calc_matrix());
            if let Some(root_bone) = &mesh.root_bone {
                let mut bone_transforms =
                    Model::get_bone_transformations(root_bone, Matrix4::identity());
                bone_transforms.sort_by(|a, b| a.0.cmp(&b.0));
                let sorted_bone_transforms = bone_transforms.iter().map(|(_, m)| m);
                let sorted: Vec<Matrix4<f32>> = Vec::from_iter(sorted_bone_transforms.cloned());
                self.shader
                    .set_uniform_mat4_array("boneTransforms", &sorted);
            }
            for (i, (texture_type, texture)) in self.textures.iter().enumerate() {
                unsafe { gl::ActiveTexture(gl::TEXTURE0 + i as u32) };
                texture.bind();
                match texture_type {
                    TextureType::Diffuse => self.shader.set_uniform_1i("texture_diffuse", i as i32),
                    TextureType::Shininess => {
                        self.shader.set_uniform_1i("texture_shininess", i as i32)
                    }
                    TextureType::Normals => self.shader.set_uniform_1i("texture_normal", i as i32),
                    TextureType::Specular => {
                        self.shader.set_uniform_1i("texture_specular", i as i32)
                    }
                    _ => {}
                }
            }
            unsafe { gl::Disable(gl::CULL_FACE) };
            mesh.render(
                &self.shader,
                (self.position.x, self.position.y, self.position.z),
                Some(self.scale),
            );
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
    }

    pub fn render_bones(&self, camera: &Camera, projection: &Projection) {
        let root = Matrix4::from_translation(self.position) * Matrix4::from_scale(self.scale);
        let mut lines: Vec<Line> = Vec::new();
        for mesh in self.meshes.values() {
            if let Some(root_bone) = &mesh.root_bone {
                lines.extend(self.render_child_bones(root_bone, camera, projection, root));
            }
        }
        LineRenderer::render_lines(
            camera,
            projection,
            &lines,
            cgmath::Vector3::new(1.0, 0.0, 0.0),
            true,
        );
    }

    fn render_child_bones(
        &self,
        bone: &Bone,
        camera: &Camera,
        projection: &Projection,
        root: cgmath::Matrix4<f32>,
    ) -> Vec<Line> {
        let position = root * bone.current_transform;
        let pos_vec = (position * Vector4::new(0.0, 0.0, 0.0, 1.0)).truncate();
        let root_vec = (root * Vector4::new(0.0, 0.0, 0.0, 1.0)).truncate();
        let direction = pos_vec - root_vec;
        let mut lines = vec![Line {
            position: Point3::new(root_vec.x, root_vec.y, root_vec.z),
            direction: direction.normalize(),
            length: direction.magnitude(),
        }];
        if let Some(children) = &bone.children {
            for child in children {
                lines.extend(self.render_child_bones(child, camera, projection, position));
            }
        }
        lines
    }

    fn get_child_bones(
        &self,
        node: &Rc<Node>,
        bones: &Vec<russimp::bone::Bone>,
        offset_matrix: Matrix4<f32>,
    ) -> Option<Vec<Bone>> {
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
                        current_transform: offset_matrix * child.transformation.to_matrix_4(),
                        offset_matrix: bone.offset_matrix.to_matrix_4(),
                        weights: bone
                            .weights
                            .iter()
                            .map(|w| (w.vertex_id, w.weight))
                            .collect(),
                        children: self.get_child_bones(child, bones, Matrix4::identity()),
                        current_animations: Vec::new(),
                        current_animation_time: Vec::new(),
                        last_translation: Vector3::zero(),
                    });
                }
            } else if let Some(child_bones) = self.get_child_bones(
                child,
                bones,
                offset_matrix * child.transformation.to_matrix_4(),
            ) {
                children.extend(child_bones);
            }
        }
        Some(children)
    }

    fn get_bone_transformations(
        bone: &Bone,
        parent_transform: Matrix4<f32>,
    ) -> Vec<(usize, Matrix4<f32>)> {
        let mut transformations = Vec::<(usize, Matrix4<f32>)>::new();
        let global_transformation = parent_transform * bone.current_transform;
        transformations.push((bone.id, global_transformation * bone.offset_matrix));
        if let Some(children) = &bone.children {
            for child in children {
                transformations
                    .extend(Self::get_bone_transformations(child, global_transformation));
            }
        }
        transformations
    }
}

impl ModelBuilder {
    pub fn new(path: &str) -> Result<ModelBuilder, Box<dyn std::error::Error>> {
        Ok(ModelBuilder {
            model: Model::new(path)?,
        })
    }

    pub fn with_animation(mut self, name: &str, path: &str) -> ModelBuilder {
        let mut animation = Animation::from_file(path).unwrap();
        animation.set_name(name);
        self.model.add_animation(animation);
        self
    }

    pub fn build(self) -> Model {
        self.model
    }
}
