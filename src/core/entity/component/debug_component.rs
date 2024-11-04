use glfw::{Action, Glfw, Key};

use crate::{
    core::{
        entity::component::{camera_component, model_component::ModelComponent, Component},
        renderer::{
            line::{Line, LineRenderer},
            text::TextRenderer,
        },
        scene::Scene,
    },
    terrain::{dual_contouring::DualContouringChunk, ChunkBounds, Terrain, CHUNK_SIZE},
};
use cgmath::{Deg, EuclideanSpace, Point3, Vector3};

pub struct DebugController {
    pub debug_ui: bool,
    wireframe: bool,
    vsync: bool,
    show_rays: bool,
    delta_time: f64,
}

impl DebugController {
    pub fn new() -> Self {
        Self {
            debug_ui: false,
            wireframe: false,
            vsync: true,
            show_rays: false,
            delta_time: 0.0,
        }
    }
}

impl Component for DebugController {
    fn update(&mut self, _: &Scene, delta_time: f64) {
        self.delta_time = delta_time;
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
                self.debug_ui = !self.debug_ui;
            }
            glfw::WindowEvent::Key(Key::F4, _, Action::Press, _) => {
                self.show_rays = !self.show_rays;
            }
            _ => {}
        }
    }

    fn render(&self, scene: &Scene) {
        if let Some(camera_component) = scene.get_component::<camera_component::CameraComponent>() {
            let camera = camera_component.get_camera();
            let projection = camera_component.get_projection();

            if self.show_rays {
                if let Some(terrain) = scene.get_component::<Terrain<DualContouringChunk>>() {
                    if let Some((line, _)) = &terrain.get_mouse_picker().ray {
                        LineRenderer::render(
                            &camera,
                            &projection,
                            &line,
                            Vector3::new(1.0, 0.0, 0.0),
                            false,
                        );
                    }
                }
            }

            if self.debug_ui {
                let fps = 1.0 / self.delta_time;
                let fps_text = format!("{:.2} FPS ({:.2}ms)", fps, self.delta_time * 1000.0);
                TextRenderer::render(5, 5, 20.0, &fps_text);
                let pos = camera.position;
                let bounds = ChunkBounds::parse(pos.to_vec());
                TextRenderer::render(
                    5,
                    25,
                    20.0,
                    format!("x: {:.2} y: {:.2} z: {:.2}", pos.x, pos.y, pos.z).as_str(),
                );
                TextRenderer::render(
                    5,
                    45,
                    20.0,
                    format!(
                        "yaw: {:?} pitch {:?}",
                        Deg::from(camera.yaw),
                        Deg::from(camera.pitch)
                    )
                    .as_str(),
                );
                TextRenderer::render(
                    5,
                    65,
                    20.0,
                    format!(
                        "Chunk: xMin: {} yMin: {} zMin: {}",
                        bounds.min.0, bounds.min.1, bounds.min.2
                    )
                    .as_str(),
                );
                TextRenderer::render(
                    5,
                    85,
                    20.0,
                    format!(
                        "       xMax: {} yMax: {} zMax: {}",
                        bounds.max.0, bounds.max.1, bounds.max.2
                    )
                    .as_str(),
                );
                if let Some(terrain) = scene.get_component::<Terrain<DualContouringChunk>>() {
                    TextRenderer::render(
                        5,
                        105,
                        20.0,
                        format!("Triangles: {}", terrain.get_triangle_count()).as_str(),
                    );
                }
                let mut lines: Vec<Line> = Vec::new();
                let mut corner_lines: Vec<Line> = Vec::new();
                let spacing = (CHUNK_SIZE / 8) as i32;
                for i in 0..9 {
                    for j in 0..9 {
                        if i != 0 && i != 8 && j != 0 && j != 8 {
                            continue;
                        }
                        let x = bounds.min.0 as i32 + i * spacing;
                        let z = bounds.min.2 as i32 + j * spacing;
                        let line = Line {
                            position: Point3::new(x as f32, bounds.min.1 as f32, z as f32),
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
                LineRenderer::render_lines(
                    camera,
                    projection,
                    &lines,
                    Vector3::new(1.0, 1.0, 0.0),
                    false,
                );
                LineRenderer::render_lines(
                    camera,
                    projection,
                    &corner_lines,
                    Vector3::new(1.0, 0.0, 0.0),
                    false,
                );

                for model in scene.get_components::<ModelComponent>() {
                    model.get_model().render_bones(camera, projection);
                }
            }
        }
    }
}
