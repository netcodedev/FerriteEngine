use std::{collections::HashMap, rc::Rc};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4, Zero};
use russimp::{
    material::{DataContent, TextureType},
    node::Node,
    scene::{PostProcess, Scene},
};

use crate::core::renderer::{
    line::{Line, LineRenderer},
    shader::Shader,
    texture::Texture,
};

use super::{Bone, Model, ModelBuilder, ModelMesh, Pose};
use crate::core::utils::ToMatrix4;

impl Model {
    pub fn new<P: Into<Point3<f32>>>(
        path: &str,
        position: P,
    ) -> Result<Model, Box<dyn std::error::Error>> {
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
            shader,
            textures: HashMap::<TextureType, Texture>::new(),
            position: position.into(),
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
                    let texture = Texture::new();
                    texture.load_from_data(data.width(), data.height(), data.to_rgba8().into_raw());
                    self.textures.insert(tex_type.clone(), texture);
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
                            current_transform: node.transformation.to_matrix_4(),
                            offset_matrix: bone.offset_matrix.to_matrix_4(),
                            weights: bone
                                .weights
                                .iter()
                                .map(|w| (w.vertex_id, w.weight))
                                .collect(),
                            children: self.get_child_bones(node, &mesh.bones, Matrix4::identity()),
                            last_translation: Vector3::zero(),
                        });
                    }
                }
            }
            let mut model_mesh = ModelMesh::new(
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
            model_mesh.buffer_data();
            self.meshes.insert(mesh.name.clone(), model_mesh);
        }
    }

    pub fn render(
        &self,
        light_position: &Point3<f32>,
        parent_transform: &Matrix4<f32>,
        camera_projection: &Matrix4<f32>,
    ) {
        for mesh in self.meshes.values() {
            if !mesh.is_buffered() {
                panic!("Mesh is not buffered");
            }
            self.shader.bind();
            self.shader.set_uniform_3f(
                "lightPosition",
                light_position.x,
                light_position.y,
                light_position.z,
            );
            self.shader
                .set_uniform_mat4("viewProjection", &camera_projection);
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
                parent_transform * Matrix4::from_translation(self.position.to_vec().into()),
                Some(self.scale),
            );
            unsafe { gl::Enable(gl::CULL_FACE) };
        }
    }

    pub fn render_bones(&self, view_projection: &Matrix4<f32>, parent_transform: &Matrix4<f32>) {
        let root = parent_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        let mut lines: Vec<Line> = Vec::new();
        for mesh in self.meshes.values() {
            if let Some(root_bone) = &mesh.root_bone {
                lines.extend(self.render_child_bones(root_bone, root));
            }
        }
        LineRenderer::render_lines(
            view_projection,
            &lines,
            cgmath::Vector3::new(1.0, 0.0, 0.0),
            true,
        );
    }

    pub fn reset_position(&mut self) -> Vector3<f32> {
        let position = self.position;
        self.position = Point3::new(0.0, 0.0, 0.0);
        position.to_vec()
    }

    pub fn apply_pose(&mut self, pose: &Pose) {
        let mut root_translation = Vector3::zero();
        for mesh in self.meshes.values_mut() {
            if let Some(root_bone) = &mut mesh.root_bone {
                root_translation += root_bone.apply_pose(pose, true);
            }
        }
        self.position += root_translation * self.scale;
    }

    fn render_child_bones(&self, bone: &Bone, root: cgmath::Matrix4<f32>) -> Vec<Line> {
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
                lines.extend(self.render_child_bones(child, position));
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
                        current_transform: offset_matrix * child.transformation.to_matrix_4(),
                        offset_matrix: bone.offset_matrix.to_matrix_4(),
                        weights: bone
                            .weights
                            .iter()
                            .map(|w| (w.vertex_id, w.weight))
                            .collect(),
                        children: self.get_child_bones(child, bones, Matrix4::identity()),
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
            model: Model::new(path, (0.0, 0.0, 0.0))?,
        })
    }

    pub fn with_position<P: Into<Point3<f32>>>(mut self, position: P) -> ModelBuilder {
        self.model.position = position.into();
        self
    }

    pub fn build(self) -> Model {
        self.model
    }
}
