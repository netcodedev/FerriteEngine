use std::f32::consts::FRAC_PI_2;

use cgmath::{Point3, Quaternion, Rad, Rotation3, Vector3, Zero};
use glfw::{Action, CursorMode, Glfw, Key, WindowEvent};
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};

use crate::core::{
    entity::{
        component::{
            animation_component::AnimationComponent, camera_component::CameraComponent,
            model_component::ModelComponent, Component,
        },
        Entity,
    },
    model::{animation_graph::AnimationGraph, ModelBuilder},
    physics::rigidbody::RigidBody,
    scene::Scene,
};

use super::{BoneColliders, Player, PlayerController};

const CAPSULE_HALF_HEIGHT: f32 = 0.5;
const CAPSULE_RADIUS: f32 = 0.5;
const JUMP_VELOCITY: f32 = 4.0;

const SAFE_FRAC_PI_2: f32 = FRAC_PI_2 - 0.0001;

impl Player {
    pub fn new<P: Into<Point3<f32>>>(
        scene: &mut Scene,
        position: P,
        animation_graph: AnimationGraph,
    ) -> Result<Entity, Box<dyn std::error::Error>> {
        let mut entity = Entity::new("player");
        entity.set_position(scene, position);

        let mut model = ModelBuilder::new("Mannequin.fbx")?.build();
        model.init();

        let animation_component = AnimationComponent::new(animation_graph);

        let collider = ColliderBuilder::capsule_y(CAPSULE_HALF_HEIGHT, CAPSULE_RADIUS)
            .friction(0.0)
            .build();

        let translation = entity.get_position();
        let rigid_body = RigidBodyBuilder::dynamic()
            .translation(rapier3d::prelude::Vector::new(
                translation.x,
                translation.y,
                translation.z,
            ))
            .lock_rotations()
            .build();
        let rigid_body_handle = scene.physics_engine.add_rigid_body(rigid_body);
        scene
            .physics_engine
            .add_collider(collider, Some(rigid_body_handle));

        let rigidbody_component = RigidBody { rigid_body_handle };
        let controller = PlayerController::new(rigid_body_handle);

        entity.add_component(animation_component);
        entity.add_component(rigidbody_component);
        entity.add_component(ModelComponent::new(model));
        entity.add_component(controller);

        let bone_colliders = BoneColliders::build(scene, &entity);
        entity.add_component(bone_colliders);

        Ok(entity)
    }
}

impl PlayerController {
    pub fn new(rigid_body_handle: rapier3d::prelude::RigidBodyHandle) -> Self {
        Self {
            forward: 0.0,
            backward: 0.0,
            left: 0.0,
            right: 0.0,
            sprint: 0.0,
            jump: 0.0,
            jump_impulse_pending: false,
            dirty: false,
            rigid_body_handle,
            // Start facing roughly +Z (camera looks into the scene).
            yaw: Rad(FRAC_PI_2),
            pitch: Rad(0.0),
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            sensitivity: 0.003,
            is_free_camera: false,
            // Local-space offset: slightly right, above head, behind the player.
            camera_offset: Vector3::new(0.25, 1.33, -2.05),
        }
    }
}

impl Component for PlayerController {
    fn update(&mut self, scene: &mut Scene, entity: &mut Entity, dt: f64) {
        // --- Mouse look (only in locked mode) ---
        if !self.is_free_camera {
            self.yaw += Rad(self.mouse_dx * self.sensitivity);
            self.pitch += Rad(-self.mouse_dy * self.sensitivity);
            if self.pitch.0 < -SAFE_FRAC_PI_2 {
                self.pitch = Rad(-SAFE_FRAC_PI_2);
            } else if self.pitch.0 > SAFE_FRAC_PI_2 {
                self.pitch = Rad(SAFE_FRAC_PI_2);
            }
        }
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;

        // Rotate the entity so the model faces the camera look direction.
        // from_angle_y(θ) maps local +Z to world (sin θ, 0, cos θ).
        // Camera look direction = (cos yaw, 0, sin yaw).
        // We need sin(θ) = cos(yaw) → θ = π/2 - yaw.
        let model_rotation: Quaternion<f32> = Quaternion::from_angle_y(Rad(FRAC_PI_2) - self.yaw);
        entity.set_rotation(scene, model_rotation);

        // --- Animation inputs ---
        if let Some(anim) = entity.get_component_mut::<AnimationComponent>() {
            if self.dirty {
                anim.set_input("forward", self.forward);
                anim.set_input("backward", self.backward);
                anim.set_input("left", self.left);
                anim.set_input("right", self.right);
                anim.set_input("sprint", self.sprint);
                anim.set_input("jump", self.jump);
            }
        }

        // --- Root motion → world-space velocity ---
        // Root motion is in model-local space where +Z is forward.
        // Rotate it by the same transform used for the entity (θ = π/2 - yaw):
        //   world_x = rm.x * sin(yaw) + rm.z * cos(yaw)
        //   world_z = -rm.x * cos(yaw) + rm.z * sin(yaw)
        let root_motion = if let Some(mc) = entity.get_component_mut::<ModelComponent>() {
            mc.get_model_mut().reset_position()
        } else {
            Vector3::zero()
        };

        let (sin_yaw, cos_yaw) = self.yaw.0.sin_cos();
        let world_rm = Vector3::new(
            root_motion.x * sin_yaw + root_motion.z * cos_yaw,
            root_motion.y,
            -root_motion.x * cos_yaw + root_motion.z * sin_yaw,
        );

        let current_vy = {
            let rb = &scene.physics_engine.rigid_bodies[self.rigid_body_handle];
            // Clamp upward velocity: ghost collisions from trimesh edges can inject a large
            // positive vy that gets preserved here and compounds across frames.
            rb.linvel().y.min(JUMP_VELOCITY)
        };
        let vy = if self.jump_impulse_pending {
            self.jump_impulse_pending = false;
            JUMP_VELOCITY
        } else {
            current_vy
        };
        let hvel = if dt > 0.0 {
            world_rm / dt as f32
        } else {
            Vector3::zero()
        };
        {
            let rb = &mut scene.physics_engine.rigid_bodies[self.rigid_body_handle];
            rb.set_linvel(rapier3d::prelude::Vector::new(hvel.x, vy, hvel.z), true);
        }

        // --- Camera sync ---
        let camera_component = scene.get_component_mut::<CameraComponent>().unwrap();
        camera_component.get_camera_controller_mut().is_free = self.is_free_camera;

        if !self.is_free_camera {
            // Rotate local camera offset into world space by the player's yaw.
            let off = self.camera_offset;
            let world_offset = Vector3::new(
                off.x * sin_yaw + off.z * cos_yaw,
                off.y,
                -off.x * cos_yaw + off.z * sin_yaw,
            );
            let camera = camera_component.get_camera_mut();
            camera.set_position(entity.get_position());
            camera.set_relative_position(Point3::new(
                world_offset.x,
                world_offset.y,
                world_offset.z,
            ));
            camera.set_yaw_pitch(self.yaw, self.pitch);
        }

        self.dirty = false;
    }

    fn handle_event(&mut self, _: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        match event {
            // Toggle free-camera mode
            WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                self.is_free_camera = !self.is_free_camera;
                if self.is_free_camera {
                    window.set_cursor_mode(CursorMode::Normal);
                } else {
                    window.set_cursor_mode(CursorMode::Disabled);
                    // Reset cursor to center so there is no position jump on re-lock.
                    window.set_cursor_pos(0.0, 0.0);
                    self.mouse_dx = 0.0;
                    self.mouse_dy = 0.0;
                }
            }

            // Capture raw mouse delta for look rotation (locked mode only).
            WindowEvent::CursorPos(x, y) if !self.is_free_camera => {
                if window.get_cursor_mode() == CursorMode::Disabled {
                    self.mouse_dx = *x as f32;
                    self.mouse_dy = *y as f32;
                    window.set_cursor_pos(0.0, 0.0);
                }
            }

            // Movement — only active in locked (player-control) mode.
            WindowEvent::Key(Key::W, _, action, _) if !self.is_free_camera => match action {
                Action::Press => {
                    self.forward = 1.0;
                    self.dirty = true;
                }
                Action::Release => {
                    self.forward = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            WindowEvent::Key(Key::S, _, action, _) if !self.is_free_camera => match action {
                Action::Press => {
                    self.backward = 1.0;
                    self.dirty = true;
                }
                Action::Release => {
                    self.backward = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            WindowEvent::Key(Key::A, _, action, _) if !self.is_free_camera => match action {
                Action::Press => {
                    self.left = 1.0;
                    self.dirty = true;
                }
                Action::Release => {
                    self.left = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            WindowEvent::Key(Key::D, _, action, _) if !self.is_free_camera => match action {
                Action::Press => {
                    self.right = 1.0;
                    self.dirty = true;
                }
                Action::Release => {
                    self.right = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            WindowEvent::Key(Key::LeftShift, _, action, _) if !self.is_free_camera => {
                match action {
                    Action::Press => {
                        self.sprint = 1.0;
                        self.dirty = true;
                    }
                    Action::Release => {
                        self.sprint = 0.0;
                        self.dirty = true;
                    }
                    _ => {}
                }
            }
            WindowEvent::Key(Key::Space, _, action, _) if !self.is_free_camera => match action {
                Action::Press => {
                    self.jump = 1.0;
                    self.jump_impulse_pending = true;
                    self.dirty = true;
                }
                Action::Release => {
                    self.jump = 0.0;
                    self.dirty = true;
                }
                _ => {}
            },
            _ => {}
        }
    }
}
