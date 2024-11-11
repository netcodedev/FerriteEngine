use glfw::{Action, Glfw, Key};

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
use cgmath::{Deg, EuclideanSpace, Matrix4, Point3, Vector3};

use super::model_component::ModelComponent;

pub struct DebugController {
    pub debug_ui: bool,
    wireframe: bool,
    vsync: bool,
    show_rays: bool,
    delta_time: f64,

    bounds: ChunkBounds,

    fps_text: Text,
    pos_text: Text,
    cam_text: Text,
    chunk_min_text: Text,
    chunk_max_text: Text,
    triangle_count_text: Text,
}

impl DebugController {
    pub fn new() -> Self {
        Self {
            debug_ui: false,
            wireframe: false,
            vsync: true,
            show_rays: false,
            delta_time: 0.0,

            bounds: ChunkBounds {
                min: (0, 0, 0),
                max: (0, 0, 0),
            },

            fps_text: Text::new(Fonts::RobotoMono, 5, 5, 26.0, String::from("FPS: 0.0")),
            pos_text: Text::new(Fonts::RobotoMono, 5, 30, 16.0, String::from("")),
            cam_text: Text::new(Fonts::RobotoMono, 5, 50, 16.0, String::from("")),
            chunk_min_text: Text::new(Fonts::RobotoMono, 5, 70, 16.0, String::from("")),
            chunk_max_text: Text::new(Fonts::RobotoMono, 5, 90, 16.0, String::from("")),
            triangle_count_text: Text::new(Fonts::RobotoMono, 5, 110, 16.0, String::from("")),
        }
    }
}

impl Component for DebugController {
    fn update(&mut self, scene: &mut Scene, _: &mut Entity, delta_time: f64) {
        self.delta_time = delta_time;

        let fps = 1.0 / self.delta_time;
        self.fps_text.set_content(format!(
            "{:.2} FPS ({:.2}ms)",
            fps,
            self.delta_time * 1000.0
        ));
        if self.debug_ui {
            if let Some(camera_component) =
                scene.get_component::<camera_component::CameraComponent>()
            {
                let camera = camera_component.get_camera();
                let pos = camera.get_position();
                let rel_pos = camera.get_relative_position();
                self.bounds = ChunkBounds::parse(pos.to_vec());

                self.pos_text.set_content(format!(
                    "x: {:.2} ({:.2}) y: {:.2} ({:.2}) z: {:.2} ({:.2})",
                    pos.x, rel_pos.x, pos.y, rel_pos.y, pos.z, rel_pos.z
                ));
                self.cam_text.set_content(format!(
                    "yaw: {:?} pitch {:?}",
                    Deg::from(camera.get_yaw()),
                    Deg::from(camera.get_pitch())
                ));
                self.chunk_min_text.set_content(format!(
                    "Chunk: xMin: {} yMin: {} zMin: {}",
                    self.bounds.min.0, self.bounds.min.1, self.bounds.min.2
                ));
                self.chunk_max_text.set_content(format!(
                    "       xMax: {} yMax: {} zMax: {}",
                    self.bounds.max.0, self.bounds.max.1, self.bounds.max.2
                ));
            }
            if let Some(terrain) = scene.get_component::<Terrain<DualContouringChunk>>() {
                self.triangle_count_text
                    .set_content(format!("Triangles: {}", terrain.get_triangle_count()));
            }
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
                self.debug_ui = !self.debug_ui;
            }
            glfw::WindowEvent::Key(Key::F4, _, Action::Press, _) => {
                self.show_rays = !self.show_rays;
            }
            _ => {}
        }
    }

    fn render(&self, scene: &Scene, view_projection: &Matrix4<f32>, _: &Matrix4<f32>) {
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
                let transform = Matrix4::from_translation(entity.get_position().to_vec());
                if let Some(model_component) = entity.get_component::<ModelComponent>() {
                    model_component
                        .get_model()
                        .render_bones(view_projection, &transform);
                }
            }
        }
    }
}
