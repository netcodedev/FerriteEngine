use std::{path::PathBuf, time::Instant};

use cgmath::{EuclideanSpace, Matrix4, Point3, SquareMatrix, Vector3};
use ferrite::core::{
    assets::{self, MaterialMap, ModelSettings},
    model::{
        clip_file::Transform as BoneTransform,
        grip::{self, Grip, Hand},
        ik::{IkChain, Skeleton},
        Animation, Model, ModelBuilder,
    },
};

use super::*;

/// A skinned model with the skeleton it is posed by.
pub(super) struct Rigged {
    pub(super) model: Model,
    pub(super) skeleton: Skeleton,
    pub(super) chains: Vec<IkChain>,
    pub(super) center: Point3<f32>,
    pub(super) radius: f32,
    pub(super) floor: f32,
}

/// Load a model file as the working copy's settings say: its scale, the
/// textures given to its materials, the variant shown.
fn load_model(
    source: &std::path::Path,
    settings: &ModelSettings,
    variant: usize,
) -> Result<Model, String> {
    let overrides: Vec<Vec<(MaterialMap, PathBuf)>> = {
        let database = assets::database();
        settings
            .materials
            .iter()
            .map(|material| {
                MaterialMap::ALL
                    .into_iter()
                    .filter_map(|map| {
                        let entry = database.get(material.map(map)?)?;
                        Some((map, entry.source_path()))
                    })
                    .collect()
            })
            .collect()
    };
    let path = source.to_string_lossy().replace('\\', "/");
    let mut model = ModelBuilder::from_file(&path)
        .map_err(|e| e.to_string())?
        .with_scale(settings.scale)
        .build();
    model.init();
    for (material, maps) in overrides.into_iter().enumerate() {
        for (map, file) in maps {
            model.set_material_map_file(material, map, &file);
        }
    }
    let variant = settings.variants.get(variant).map(|v| v.name.as_str());
    model.show_variant(variant)?;
    Ok(model)
}

/// A model held in the hand of the player model, which stands still or
/// plays a clip.
pub(super) struct Held {
    pub(super) player: Rigged,
    pub(super) item: Model,
    /// The clip played (index into the asset's hold poses + 1; 0 none).
    pose_index: usize,
    pose: Option<Animation>,
    /// Ticks into the clip.
    time: f32,
    last: Instant,
    /// What the camera turns around.
    pub(super) center: Point3<f32>,
    pub(super) radius: f32,
}

impl Held {
    pub(super) fn load(edit: &AssetEdit, settings: &ModelSettings) -> Result<Held, String> {
        let player = {
            let database = assets::database();
            let source = database
                .root()
                .join("models")
                .join(ferrite::player::PLAYER_MODEL_FILE);
            database
                .find_by_source(&source)
                .and_then(|id| database.get(id))
                .and_then(|entry| Some((entry.source_path(), entry.meta.as_model()?.clone())))
        };
        let (source, player_settings) =
            player.ok_or("there is no player model to hold it (models/Mannequin.fbx)")?;
        let player = Rigged::load(&source, &player_settings, 0)?
            .ok_or("the player model has no bones")?;
        let item = load_model(&edit.source_path(), settings, edit.variant)?;
        // Between the body and the right hand, close enough to see the grip.
        let hand = player
            .skeleton
            .joints
            .iter()
            .position(|joint| ferrite::core::model::ik::base_name(&joint.name) == "RightHand")
            .map(|joint| player.skeleton.world_positions(&player.root())[joint]);
        let center = match hand {
            Some(hand) => Point3::from_vec((player.center.to_vec() + hand.to_vec()) * 0.5),
            None => player.center,
        };
        Ok(Held {
            radius: player.radius * 0.6,
            center,
            player,
            item,
            pose_index: 0,
            pose: None,
            time: 0.0,
            last: Instant::now(),
        })
    }

    /// Play `index` of the hold poses (0: stand in the rest pose).
    pub(super) fn set_pose(&mut self, index: usize, poses: &[HoldPose]) {
        if index == self.pose_index {
            return;
        }
        self.pose_index = index;
        self.time = 0.0;
        self.pose = index.checked_sub(1).and_then(|i| poses.get(i)).and_then(|pose| {
            Animation::load(&pose.label, &pose.source, pose.clip.as_deref())
                .map_err(|e| ferrite::log::warn!("{}: {e}", pose.label))
                .ok()
        });
    }

    /// Move the clip on and pose the player model, the other hand on the
    /// grip.
    pub(super) fn animate(&mut self, grip: &Grip) {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        let player = &mut self.player;
        player.skeleton.reset();
        player.model.apply_skeleton(&player.skeleton);
        if let Some(pose) = &self.pose {
            let duration = pose.duration.max(f32::EPSILON);
            self.time = (self.time + dt * pose.ticks_per_second) % duration;
            player.model.apply_pose(&pose.sample(self.time));
            player.model.reset_position();
            // The root's movement is the character's (root motion, in the
            // game): it stands where it is.
            if let (Some(mut posed), Some(rest)) =
                (player.model.skeleton(), player.skeleton.joints.first())
            {
                if let Some(root) = posed.joints.first_mut() {
                    root.local = rest.local;
                }
                player.model.apply_skeleton(&posed);
            }
        }
        grip::reach_with_other_hand(
            &mut player.model,
            &Matrix4::identity(),
            grip,
            Hand::Right,
            1.0,
        );
    }

    /// Where the held model is drawn.
    pub(super) fn placed(&self, grip: &Grip) -> Option<Matrix4<f32>> {
        grip::held_transform(&self.player.model, &Matrix4::identity(), grip, Hand::Right)
    }
}

/// A grip being moved in the preview.
pub(super) enum GripDrag {
    /// The held model follows the cursor, sliding through the hand.
    Item {
        /// Where it was drawn when the drag began.
        placed: Matrix4<f32>,
        start_point: [f32; 3],
        plane_normal: Vector3<f32>,
        start_hit: Point3<f32>,
    },
    /// The other hand's point follows the cursor over the model.
    OtherHand {
        placed: Matrix4<f32>,
        plane_normal: Vector3<f32>,
        plane_point: Point3<f32>,
        grab: Vector3<f32>,
    },
}

impl Rigged {
    /// Load the model of the working copy skinned; `None` if it has no
    /// bones.
    pub(super) fn load(
        source: &std::path::Path,
        settings: &ModelSettings,
        variant: usize,
    ) -> Result<Option<Rigged>, String> {
        // Texture files of the working copy's materials, found before the
        // (slow) load.
        let overrides: Vec<Vec<(MaterialMap, PathBuf)>> = {
            let database = assets::database();
            settings
                .materials
                .iter()
                .map(|material| {
                    MaterialMap::ALL
                        .into_iter()
                        .filter_map(|map| {
                            let entry = database.get(material.map(map)?)?;
                            Some((map, entry.source_path()))
                        })
                        .collect()
                })
                .collect()
        };
        let path = source.to_string_lossy().replace('\\', "/");
        let mut model = ModelBuilder::from_file(&path)
            .map_err(|e| e.to_string())?
            .with_scale(settings.scale)
            .build();
        model.init();
        let Some(skeleton) = model.skeleton() else {
            return Ok(None);
        };
        for (material, maps) in overrides.into_iter().enumerate() {
            for (map, file) in maps {
                model.set_material_map_file(material, map, &file);
            }
        }
        let variant = settings.variants.get(variant).map(|v| v.name.as_str());
        model.show_variant(variant)?;

        let vertices = model.get_vertices_scaled();
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for v in &vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(v[axis]);
                max[axis] = max[axis].max(v[axis]);
            }
        }
        if vertices.is_empty() {
            (min, max) = ([0.0; 3], [1.0; 3]);
        }
        let center = Point3::new(
            (min[0] + max[0]) * 0.5,
            (min[1] + max[1]) * 0.5,
            (min[2] + max[2]) * 0.5,
        );
        let radius = (0..3)
            .map(|axis| (max[axis] - min[axis]) * 0.5)
            .fold(0.0f32, |sum, half| sum + half * half)
            .sqrt()
            .max(0.01);
        let chains = skeleton.default_chains();
        Ok(Some(Rigged {
            model,
            skeleton,
            chains,
            center,
            radius,
            floor: min[1],
        }))
    }

    pub(super) fn root(&self) -> Matrix4<f32> {
        self.model.root_transform(&Matrix4::identity())
    }

    /// Whether a joint can be selected and keyed (importer helper nodes
    /// cannot).
    pub(super) fn is_bone(&self, joint: usize) -> bool {
        !self.skeleton.joints[joint].name.contains("$AssimpFbx$")
    }

    /// Pose the skeleton as a clip has it at `time`.
    pub(super) fn pose_from(&mut self, clip: &ferrite::core::model::clip_file::ClipFile, time: f32) {
        self.skeleton.reset();
        let pose = clip.sample(time);
        for joint in &mut self.skeleton.joints {
            if let Some(transform) = pose.get(&joint.name) {
                joint.local = transform.to_matrix();
            }
        }
        self.model.apply_skeleton(&self.skeleton);
    }

    /// The bones whose local transform differs from `start`, with it.
    pub(super) fn changed_since(&self, start: &[Matrix4<f32>]) -> Vec<usize> {
        let close = |a: &Matrix4<f32>, b: &Matrix4<f32>| {
            (0..4).all(|c| (0..4).all(|r| (a[c][r] - b[c][r]).abs() < 1e-6))
        };
        self.skeleton
            .joints
            .iter()
            .enumerate()
            .filter(|(i, joint)| self.is_bone(*i) && start.get(*i).is_none_or(|s| !close(s, &joint.local)))
            .map(|(i, _)| i)
            .collect()
    }

    pub(super) fn transform_of(&self, joint: usize) -> (String, BoneTransform) {
        let joint = &self.skeleton.joints[joint];
        (joint.name.clone(), BoneTransform::from_matrix(&joint.local))
    }

    /// World position of each chain's handle.
    pub(super) fn handles(&self) -> Vec<Point3<f32>> {
        let positions = self.skeleton.world_positions(&self.root());
        self.chains.iter().map(|chain| positions[chain.end]).collect()
    }
}

/// A handle being dragged: its end effector follows the cursor on a plane
/// facing the camera.
pub(super) struct IkDrag {
    pub(super) chain: usize,
    /// Pose when the drag began; every solve starts from it.
    pub(super) start: Vec<Matrix4<f32>>,
    pub(super) plane_point: Point3<f32>,
    pub(super) plane_normal: Vector3<f32>,
    /// From where the cursor hit the plane to the handle.
    pub(super) grab: Vector3<f32>,
    /// Whether the handle was moved (a click without moving selects it).
    pub(super) moved: bool,
}

/// A ring of the selected bone being dragged: the bone turns about the
/// ring's axis as the cursor goes round the bone.
pub(super) struct RingDrag {
    pub(super) joint: usize,
    pub(super) axis: usize,
    pub(super) start: Vec<Matrix4<f32>>,
    /// Screen angle of the cursor around the bone last frame.
    pub(super) angle: f32,
    /// +1 when the axis points towards the camera.
    pub(super) sign: f32,
}
