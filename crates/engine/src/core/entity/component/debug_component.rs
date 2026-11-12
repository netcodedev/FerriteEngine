use glfw::{Action, Glfw, Key};
use rapier3d::prelude::{ColliderHandle, TypedShape};
use std::collections::HashMap;

use crate::{
    core::{
        entity::{
            component::{camera_component, Component},
            Entity,
        },
        renderer::{
            light::skylight::SkyLight,
            line::{Line, LineRenderer},
            text::{Fonts, Text},
        },
        scene::Scene,
    },
    terrain::{dual_contouring::DualContouringChunk, ChunkBounds, Terrain, CHUNK_SIZE},
};
use cgmath::{Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, Vector3};

use super::model_component::ModelComponent;

const TRIMESH_DRAW_DISTANCE: f32 = 96.0;

pub struct DebugController {
    pub debug_ui: bool,
    wireframe: bool,
    vsync: bool,
    show_rays: bool,
    show_colliders: bool,
    f3_held: bool,
    f3_used_as_modifier: bool,
    delta_time: f64,

    bounds: ChunkBounds,

    fps_text: Text,
    pos_text: Text,
    cam_text: Text,
    chunk_min_text: Text,
    chunk_max_text: Text,
    triangle_count_text: Text,
    time_text: Text,

    /// Pending time-of-day delta (in fractions of a day) accumulated from F5/F6
    /// keypresses, applied in `update` where we have a `&mut Scene`.
    pending_time_delta: f32,
    /// Pending request to toggle the day/night cycle pause (F7).
    pending_toggle_cycle: bool,
    /// Pending cloud-coverage delta accumulated from F8/F9.
    pending_coverage_delta: f32,

    // Pre-built per-collider geometry: (centroid, va, vb, vc) in world space.
    // Terrain trimeshes are static so this is computed once per handle.
    trimesh_cache:
        HashMap<ColliderHandle, Vec<(Point3<f32>, Point3<f32>, Point3<f32>, Point3<f32>)>>,
}

impl DebugController {
    pub fn new() -> Self {
        Self {
            debug_ui: false,
            wireframe: false,
            vsync: true,
            show_rays: false,
            show_colliders: false,
            f3_held: false,
            f3_used_as_modifier: false,
            delta_time: 0.0,

            bounds: ChunkBounds {
                min: (0, 0, 0),
                max: (0, 0, 0),
            },

            fps_text: Text::new(Fonts::RobotoMono, 5, 5, 0, 26.0, String::from("FPS: 0.0")),
            pos_text: Text::new(Fonts::RobotoMono, 5, 30, 0, 16.0, String::from("")),
            cam_text: Text::new(Fonts::RobotoMono, 5, 50, 0, 16.0, String::from("")),
            chunk_min_text: Text::new(Fonts::RobotoMono, 5, 70, 0, 16.0, String::from("")),
            chunk_max_text: Text::new(Fonts::RobotoMono, 5, 90, 0, 16.0, String::from("")),
            triangle_count_text: Text::new(Fonts::RobotoMono, 5, 110, 0, 16.0, String::from("")),
            time_text: Text::new(Fonts::RobotoMono, 5, 130, 0, 16.0, String::from("")),

            pending_time_delta: 0.0,
            pending_toggle_cycle: false,
            pending_coverage_delta: 0.0,

            trimesh_cache: HashMap::new(),
        }
    }
}

// Wireframe helpers live in `physics::collider_debug` (shared with the
// editor's per-component collider display).
use crate::core::physics::collider_debug::{apply_pose, ball_wires, capsule_wires, seg};

// --- component impl ---

impl Component for DebugController {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, delta_time: f64) {
        self.delta_time = delta_time;

        // Apply pending sky/time mutations queued from the event handler.
        if let Some(skylight) = scene.get_component_mut::<SkyLight>() {
            if self.pending_toggle_cycle {
                skylight.toggle_paused();
                self.pending_toggle_cycle = false;
            }
            if self.pending_time_delta != 0.0 {
                skylight.advance_time(self.pending_time_delta);
                self.pending_time_delta = 0.0;
            }
            if self.pending_coverage_delta != 0.0 {
                skylight.adjust_cloud_coverage(self.pending_coverage_delta);
                self.pending_coverage_delta = 0.0;
            }
        }

        let fps = 1.0 / self.delta_time;
        self.fps_text.set_content(&format!(
            "{:.2} FPS ({:.2}ms)",
            fps,
            self.delta_time * 1000.0
        ));

        if self.debug_ui {
            if let Some(skylight) = scene.get_component::<SkyLight>() {
                let (h, m) = skylight.get_clock();
                let suffix = if skylight.is_paused() { " (paused)" } else { "" };
                self.time_text
                    .set_content(&format!("Time: {:02}:{:02}{}", h, m, suffix));
            }
        }
        if self.debug_ui && self.show_colliders {
            // Cache trimesh geometry for any colliders we haven't seen yet.
            // Terrain is static so world-space positions are computed once.
            for (handle, collider) in scene.physics_engine.colliders.iter() {
                if self.trimesh_cache.contains_key(&handle) {
                    continue;
                }
                if let TypedShape::TriMesh(mesh) = collider.shape().as_typed_shape() {
                    let pos = collider.translation();
                    let rot = collider.rotation();
                    let tris = mesh
                        .triangles()
                        .map(|tri| {
                            let va =
                                apply_pose(&pos, &rot, Vector3::new(tri.a.x, tri.a.y, tri.a.z));
                            let vb =
                                apply_pose(&pos, &rot, Vector3::new(tri.b.x, tri.b.y, tri.b.z));
                            let vc =
                                apply_pose(&pos, &rot, Vector3::new(tri.c.x, tri.c.y, tri.c.z));
                            let centroid = va + (vb - va) * (1.0 / 3.0) + (vc - va) * (1.0 / 3.0);
                            (centroid, va, vb, vc)
                        })
                        .collect();
                    self.trimesh_cache.insert(handle, tris);
                }
            }

            if let Some(camera_component) =
                scene.get_component::<camera_component::CameraComponent>()
            {
                let camera = camera_component.get_camera();
                let pos = camera.get_position();
                let rel_pos = camera.get_relative_position();
                self.bounds = ChunkBounds::parse(pos.to_vec());

                self.pos_text.set_content(&format!(
                    "x: {:.2} ({:.2}) y: {:.2} ({:.2}) z: {:.2} ({:.2})",
                    pos.x, rel_pos.x, pos.y, rel_pos.y, pos.z, rel_pos.z
                ));
                self.cam_text.set_content(&format!(
                    "yaw: {:?} pitch {:?}",
                    Deg::from(camera.get_yaw()),
                    Deg::from(camera.get_pitch())
                ));
                self.chunk_min_text.set_content(&format!(
                    "Chunk: xMin: {} yMin: {} zMin: {}",
                    self.bounds.min.0, self.bounds.min.1, self.bounds.min.2
                ));
                self.chunk_max_text.set_content(&format!(
                    "       xMax: {} yMax: {} zMax: {}",
                    self.bounds.max.0, self.bounds.max.1, self.bounds.max.2
                ));
            }
            let mut triangle_count = 0;
            for terrain in scene.get_entities_with_component::<Terrain<DualContouringChunk>>() {
                triangle_count += terrain
                    .get_component::<Terrain<DualContouringChunk>>()
                    .unwrap()
                    .get_triangle_count(&terrain);
            }
            self.triangle_count_text
                .set_content(&format!("Triangles: {}", triangle_count));
        }
    }

    fn handle_event(&mut self, glfw: &mut Glfw, _: &mut glfw::Window, event: &glfw::WindowEvent) {
        match event {
            glfw::WindowEvent::Key(Key::F1, _, Action::Press, _) => {
                self.wireframe = !self.wireframe;
                unsafe {
                    if self.wireframe {
                        gl::PolygonMode(gl::FRONT_AND_BACK, gl::LINE);
                    } else {
                        gl::PolygonMode(gl::FRONT_AND_BACK, gl::FILL);
                    }
                }
            }
            glfw::WindowEvent::Key(Key::F2, _, Action::Press, _) => {
                self.vsync = !self.vsync;
                if self.vsync {
                    glfw.set_swap_interval(glfw::SwapInterval::Sync(1));
                } else {
                    glfw.set_swap_interval(glfw::SwapInterval::None);
                }
            }
            glfw::WindowEvent::Key(Key::F3, _, Action::Press, _) => {
                self.f3_held = true;
                self.f3_used_as_modifier = false;
            }
            glfw::WindowEvent::Key(Key::F3, _, Action::Release, _) => {
                if !self.f3_used_as_modifier {
                    self.debug_ui = !self.debug_ui;
                }
                self.f3_held = false;
                self.f3_used_as_modifier = false;
            }
            glfw::WindowEvent::Key(Key::C, _, Action::Press, _) if self.f3_held => {
                self.show_colliders = !self.show_colliders;
                self.f3_used_as_modifier = true;
            }
            glfw::WindowEvent::Key(Key::F4, _, Action::Press, _) => {
                self.show_rays = !self.show_rays;
            }
            // F5: step time backwards by one in-game hour (1/24 of a day).
            glfw::WindowEvent::Key(Key::F5, _, Action::Press, _) => {
                self.pending_time_delta -= 1.0 / 24.0;
            }
            // F6: step time forwards by one in-game hour.
            glfw::WindowEvent::Key(Key::F6, _, Action::Press, _) => {
                self.pending_time_delta += 1.0 / 24.0;
            }
            // F7: pause/resume the day/night cycle.
            glfw::WindowEvent::Key(Key::F7, _, Action::Press, _) => {
                self.pending_toggle_cycle = true;
            }
            // Numpad -/+: decrease/increase cloud coverage in 0.02 steps.
            glfw::WindowEvent::Key(Key::KpSubtract, _, Action::Press, _) => {
                self.pending_coverage_delta -= 0.02;
            }
            glfw::WindowEvent::Key(Key::KpAdd, _, Action::Press, _) => {
                self.pending_coverage_delta += 0.02;
            }
            _ => {}
        }
    }

    fn display_name(&self) -> String {
        "Debug controller".into()
    }

    fn to_data(&self) -> Option<crate::core::scene::scene_file::ComponentData> {
        Some(crate::core::scene::scene_file::ComponentData::DebugController)
    }

    fn properties(&self) -> Vec<super::Property> {
        vec![
            super::Property::bool("overlay (F3)", self.debug_ui),
            super::Property::bool("show colliders", self.show_colliders),
            super::Property::bool("show rays", self.show_rays),
        ]
    }

    fn set_property(&mut self, name: &str, value: super::PropertyValue) {
        let Some(v) = value.as_bool() else {
            return;
        };
        match name {
            "overlay (F3)" => self.debug_ui = v,
            "show colliders" => self.show_colliders = v,
            "show rays" => self.show_rays = v,
            _ => {}
        }
    }

    fn render(&self, scene: &Scene, _: &Entity, view_projection: &Matrix4<f32>, _: &Matrix4<f32>) {
        if self.show_rays {
            if let Some(terrain) = scene.get_component::<Terrain<DualContouringChunk>>() {
                if let Some((line, _)) = &terrain.get_mouse_picker().ray {
                    LineRenderer::render(
                        view_projection,
                        &line,
                        Vector3::new(1.0, 0.0, 0.0),
                        false,
                    );
                }
            }
        }

        if self.debug_ui {
            self.fps_text.render();
            self.pos_text.render();
            self.cam_text.render();
            self.chunk_min_text.render();
            self.chunk_max_text.render();
            self.triangle_count_text.render();
            self.time_text.render();

            let mut lines: Vec<Line> = Vec::new();
            let mut corner_lines: Vec<Line> = Vec::new();
            let spacing = (CHUNK_SIZE / 8) as i32;
            for i in 0..9 {
                for j in 0..9 {
                    if i != 0 && i != 8 && j != 0 && j != 8 {
                        continue;
                    }
                    let x = self.bounds.min.0 as i32 + i * spacing;
                    let z = self.bounds.min.2 as i32 + j * spacing;
                    let line = Line {
                        position: Point3::new(x as f32, self.bounds.min.1 as f32, z as f32),
                        direction: Vector3::new(0.0, 1.0, 0.0),
                        length: CHUNK_SIZE as f32,
                    };
                    if (i == 0 || i == 8) && (j == 0 || j == 8) {
                        corner_lines.push(line);
                    } else {
                        lines.push(line);
                    }
                }
            }
            LineRenderer::render_lines(view_projection, &lines, Vector3::new(1.0, 1.0, 0.0), false);
            LineRenderer::render_lines(
                view_projection,
                &corner_lines,
                Vector3::new(1.0, 0.0, 0.0),
                false,
            );

            for entity in scene.get_entities_with_component::<ModelComponent>() {
                let rot: Matrix4<f32> = entity.get_rotation().into();
                let transform = Matrix4::from_translation(entity.get_position().to_vec()) * rot;
                if let Some(model_component) = entity.get_component::<ModelComponent>() {
                    model_component
                        .get_model()
                        .render_bones(view_projection, &transform);
                }
            }

            // Collider wireframes (toggled separately with F3+C)
            if !self.show_colliders {
                return;
            }

            let camera_pos = scene
                .get_component::<camera_component::CameraComponent>()
                .map(|cc| cc.get_camera().get_position())
                .unwrap_or_else(|| Point3::new(0.0, 0.0, 0.0));

            let mut collider_lines: Vec<Line> = Vec::new();
            let mut terrain_lines: Vec<Line> = Vec::new();

            for (handle, collider) in scene.physics_engine.colliders.iter() {
                // Skip colliders on dynamic bodies — those are the main physics proxy
                // (e.g. the player capsule). Bone capsules use kinematic bodies.
                if let Some(parent_handle) = collider.parent() {
                    if let Some(rb) = scene.physics_engine.rigid_bodies.get(parent_handle) {
                        if rb.is_dynamic() {
                            continue;
                        }
                    }
                }

                let pos = collider.translation();
                let rot = collider.rotation();
                match collider.shape().as_typed_shape() {
                    TypedShape::Ball(ball) => {
                        collider_lines.extend(ball_wires(&pos, ball.radius));
                    }
                    TypedShape::Capsule(capsule) => {
                        let r = capsule.radius;
                        let sa = capsule.segment.a;
                        let sb = capsule.segment.b;
                        let a = apply_pose(&pos, &rot, Vector3::new(sa.x, sa.y, sa.z));
                        let b = apply_pose(&pos, &rot, Vector3::new(sb.x, sb.y, sb.z));
                        collider_lines.extend(capsule_wires(a, b, r));
                    }
                    TypedShape::ConvexPolyhedron(poly) => {
                        let pts = poly.points();
                        for edge in poly.edges() {
                            let i0 = edge.vertices[0] as usize;
                            let i1 = edge.vertices[1] as usize;
                            let va = apply_pose(
                                &pos,
                                &rot,
                                Vector3::new(pts[i0].x, pts[i0].y, pts[i0].z),
                            );
                            let vb = apply_pose(
                                &pos,
                                &rot,
                                Vector3::new(pts[i1].x, pts[i1].y, pts[i1].z),
                            );
                            collider_lines.push(seg(va, vb));
                        }
                    }
                    TypedShape::TriMesh(_) => {
                        if let Some(tris) = self.trimesh_cache.get(&handle) {
                            for (centroid, va, vb, vc) in tris {
                                if (*centroid - camera_pos).magnitude() > TRIMESH_DRAW_DISTANCE {
                                    continue;
                                }
                                terrain_lines.push(seg(*va, *vb));
                                terrain_lines.push(seg(*vb, *vc));
                                terrain_lines.push(seg(*vc, *va));
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Bone capsules in cyan (always on top so they show through the mesh).
            // Terrain trimesh in green, depth-tested so it stays grounded visually.
            LineRenderer::render_lines(
                view_projection,
                &collider_lines,
                Vector3::new(0.0, 1.0, 1.0),
                true,
            );
            LineRenderer::render_lines(
                view_projection,
                &terrain_lines,
                Vector3::new(0.2, 0.8, 0.2),
                false,
            );
        }
    }
}
