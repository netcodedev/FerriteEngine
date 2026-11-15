//! A model's bones: posing, IK skeletons and drawing them.

use std::{collections::HashMap, rc::Rc};

use cgmath::{EuclideanSpace, InnerSpace, Matrix4, Point3, SquareMatrix, Vector3, Vector4, Zero};
use russimp_ng::node::Node;

use crate::core::renderer::line::{Line, LineRenderer};

use super::{Bone, Model, Pose};
use crate::core::utils::ToMatrix4;

impl Model {
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

    /// Returns (bone_name, parent_world_pos, bone_world_pos) for every bone, in world space.
    /// `entity_transform` should be the entity's translation * rotation matrix.
    pub fn get_bone_segments(
        &self,
        entity_transform: &Matrix4<f32>,
    ) -> Vec<(String, Point3<f32>, Point3<f32>)> {
        let root = entity_transform
            * Matrix4::from_translation(self.position.to_vec())
            * Matrix4::from_scale(self.scale);
        let mut out = Vec::new();
        for mesh in self.meshes.values() {
            if let Some(bone) = &mesh.root_bone {
                self.collect_bone_segments(bone, root, &mut out);
            }
        }
        out
    }

    fn collect_bone_segments(
        &self,
        bone: &Bone,
        parent_transform: Matrix4<f32>,
        out: &mut Vec<(String, Point3<f32>, Point3<f32>)>,
    ) {
        use cgmath::Transform;
        let bone_transform = parent_transform * bone.current_transform;
        let parent_pos = parent_transform.transform_point(Point3::new(0.0, 0.0, 0.0));
        let bone_pos = bone_transform.transform_point(Point3::new(0.0, 0.0, 0.0));
        out.push((bone.name.clone(), parent_pos, bone_pos));
        if let Some(children) = &bone.children {
            for child in children {
                self.collect_bone_segments(child, bone_transform, out);
            }
        }
    }

    /// A copy of the bone hierarchy in its current pose, from the mesh with
    /// the most bones; `None` for a model without bones.
    pub fn skeleton(&self) -> Option<super::ik::Skeleton> {
        fn count(bone: &Bone) -> usize {
            1 + bone
                .children
                .as_ref()
                .map_or(0, |children| children.iter().map(count).sum())
        }
        fn collect(bone: &Bone, parent: Option<usize>, out: &mut Vec<super::ik::Joint>) {
            out.push(super::ik::Joint {
                name: bone.name.clone(),
                parent,
                local: bone.current_transform,
                skinned: bone.id != usize::MAX,
            });
            let index = out.len() - 1;
            if let Some(children) = &bone.children {
                for child in children {
                    collect(child, Some(index), out);
                }
            }
        }
        let root = self
            .meshes
            .values()
            .filter_map(|mesh| mesh.root_bone.as_ref())
            .max_by_key(|bone| count(bone))?;
        let mut joints = Vec::new();
        collect(root, None, &mut joints);
        Some(super::ik::Skeleton::new(joints))
    }

    /// Pose the bones of every mesh as `skeleton` says, by name.
    pub fn apply_skeleton(&mut self, skeleton: &super::ik::Skeleton) {
        fn set(bone: &mut Bone, locals: &HashMap<&str, Matrix4<f32>>) {
            if let Some(local) = locals.get(bone.name.as_str()) {
                bone.current_transform = *local;
            }
            if let Some(children) = &mut bone.children {
                for child in children {
                    set(child, locals);
                }
            }
        }
        let locals: HashMap<&str, Matrix4<f32>> = skeleton
            .joints
            .iter()
            .map(|joint| (joint.name.as_str(), joint.local))
            .collect();
        for mesh in self.meshes.values_mut() {
            if let Some(root) = &mut mesh.root_bone {
                set(root, &locals);
            }
        }
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

    pub(super) fn get_child_bones(
        &self,
        node: &Rc<Node>,
        bones: &Vec<russimp_ng::bone::Bone>,
    ) -> Option<Vec<Bone>> {
        if node.children.borrow().len() == 0 {
            return None;
        }
        let mut children = Vec::<Bone>::new();
        for child in node.children.borrow().iter() {
            let mut bone_id = usize::MAX;
            let mut bone_offset_matrix = Matrix4::identity();
            let mut weights = Vec::new();

            for (id, bone) in bones.iter().enumerate() {
                if bone.name == child.name {
                    bone_id = id;
                    bone_offset_matrix = bone.offset_matrix.to_matrix_4();
                    weights = bone
                        .weights
                        .iter()
                        .map(|w| (w.vertex_id, w.weight))
                        .collect();
                    break;
                }
            }

            children.push(Bone {
                id: bone_id,
                name: child.name.clone(),
                current_transform: child.transformation.to_matrix_4(),
                offset_matrix: bone_offset_matrix,
                weights,
                children: self.get_child_bones(child, bones),
                last_translation: Vector3::zero(),
            });
        }
        Some(children)
    }

    /// Skinning matrices of `bone` and its descendants. A bone named in
    /// `hidden`, and everything below it, maps all its vertices onto the
    /// joint `collapse_to` (the hidden bone's own joint).
    pub(super) fn get_bone_transformations(
        bone: &Bone,
        parent_transform: Matrix4<f32>,
        hidden: &[String],
        collapse_to: Option<Vector3<f32>>,
    ) -> Vec<(usize, Matrix4<f32>)> {
        let mut transformations = Vec::<(usize, Matrix4<f32>)>::new();
        let global_transformation = parent_transform * bone.current_transform;
        let collapse_to = collapse_to.or_else(|| {
            hidden
                .contains(&bone.name)
                .then(|| global_transformation.w.truncate())
        });

        if bone.id != usize::MAX {
            let transform = match collapse_to {
                Some(joint) => Matrix4::from_translation(joint) * Matrix4::from_scale(0.0),
                None => global_transformation * bone.offset_matrix,
            };
            transformations.push((bone.id, transform));
        }

        if let Some(children) = &bone.children {
            for child in children {
                transformations.extend(Self::get_bone_transformations(
                    child,
                    global_transformation,
                    hidden,
                    collapse_to,
                ));
            }
        }
        transformations
    }
}
