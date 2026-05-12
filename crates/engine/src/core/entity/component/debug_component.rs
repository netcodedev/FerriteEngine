use glfw::{Action, Glfw, Key};
use rapier3d::prelude::{ColliderHandle, TypedShape};
use std::collections::HashMap;
use std::f32::consts::PI;

use crate::{
    core::{
        entity::{
            component::{camera_component, Component},
            Entity,
        },
        renderer::{
            line::{Line, LineRenderer},
            text::{Fonts, Text},
        },
        scene::Scene,
    },
    terrain::{dual_contouring::DualContouringChunk, ChunkBounds, Terrain, CHUNK_SIZE},
};
use cgmath::{Deg, EuclideanSpace, InnerSpace, Matrix4, Point3, Vector3};

use super::model_component::ModelComponent;

const CIRCLE_SEGMENTS: usize = 16;
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

            trimesh_cache: HashMap::new(),
        }
    }
}

// --- wireframe helpers ---

fn seg(a: Point3<f32>, b: Point3<f32>) -> Line {
    let dir = b - a;
    let len = dir.magnitude();
    Line {
        position: a,
        direction: if len > 0.001 {
            dir / len
        } else {
            Vector3::new(0.0, 1.0, 0.0)
        },
        length: len,
    }
}

fn apply_pose(
    pos: &rapier3d::math::Vector,
    rot: &rapier3d::math::Rotation,
    local: Vector3<f32>,
) -> Point3<f32> {
    use cgmath::Rotation as _;
    let q = cgmath::Quaternion::new(rot.w, rot.x, rot.y, rot.z);
    let r = q.rotate_vector(local);
    Point3::new(pos.x + r.x, pos.y + r.y, pos.z + r.z)
}

fn perp_basis(axis: Vector3<f32>) -> (Vector3<f32>, Vector3<f32>) {
    let n = axis.normalize();
    let right = if n.x.abs() < 0.9 {
        n.cross(Vector3::new(1.0, 0.0, 0.0)).normalize()
    } else {
        n.cross(Vector3::new(0.0, 1.0, 0.0)).normalize()
    };
    (right, n.cross(right).normalize())
}

fn circle_lines(
    center: Point3<f32>,
    right: Vector3<f32>,
    up: Vector3<f32>,
    radius: f32,
) -> Vec<Line> {
    (0..CIRCLE_SEGMENTS)
        .map(|i| {
            let a = 2.0 * PI * i as f32 / CIRCLE_SEGMENTS as f32;
            let b = 2.0 * PI * (i + 1) as f32 / CIRCLE_SEGMENTS as f32;
            let pa = center + right * (radius * a.cos()) + up * (radius * a.sin());
            let pb = center + right * (radius * b.cos()) + up * (radius * b.sin());
            seg(pa, pb)
        })
        .collect()
}

fn ball_wires(pos: &rapier3d::math::Vector, radius: f32) -> Vec<Line> {
    let c = Point3::new(pos.x, pos.y, pos.z);
    let mut lines = Vec::new();
    lines.extend(circle_lines(
        c,
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
        radius,
    ));
    lines.extend(circle_lines(
        c,
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        radius,
    ));
    lines.extend(circle_lines(
        c,
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(0.0, 1.0, 0.0),
        radius,
    ));
    lines
}

fn capsule_wires(a: Point3<f32>, b: Point3<f32>, r: f32) -> Vec<Line> {
    let axis = (b - a).normalize();
    let (right, up) = perp_basis(axis);
    let mid = a + (b - a) * 0.5;

    let mut lines = Vec::new();
    lines.extend(circle_lines(a, right, up, r));
    lines.extend(circle_lines(b, right, up, r));
    lines.extend(circle_lines(mid, right, up, r));
    for i in 0..4 {
        let angle = PI * 0.5 * i as f32;
        let offset = right * (r * angle.cos()) + up * (r * angle.sin());
        lines.push(seg(a + offset, b + offset));
    }
    lines
}

// --- component impl ---

impl Component for DebugController {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, delta_time: f64) {
        self.delta_time = delta_time;

        let fps = 1.0 / self.delta_time;
        self.fps_text.set_content(&format!(
            "{:.2} FPS ({:.2}ms)",
            fps,
            self.delta_time * 1000.0
        ));
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
