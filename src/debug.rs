use glfw::{Action, Glfw, Key, CursorMode};

use crate::{camera::{Camera, MousePicker, Projection}, line::{Line, LineRenderer}, terrain::{ChunkBounds, CHUNK_SIZE}, text::TextRenderer};
use cgmath::{EuclideanSpace, Point3, Vector3};

pub struct DebugController {
    pub debug_ui: bool,
    wireframe: bool,
    vsync: bool,
    show_rays: bool,
}

impl DebugController {
    pub fn new() -> Self {
        Self {
            debug_ui: false,
            wireframe: false,
            vsync: true,
            show_rays: false,
        }
    }

    pub fn process_keyboard(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &glfw::WindowEvent) {
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
            glfw::WindowEvent::Key(Key::Escape, _, Action::Press, _) => match window.get_cursor_mode() {
                CursorMode::Disabled => window.set_cursor_mode(CursorMode::Normal),
                CursorMode::Normal => window.set_cursor_mode(CursorMode::Disabled),
                _ => {}
            },
            _ => {}
        }
    }

    pub fn draw_debug_ui(&self, delta_time: f32, mouse_picker: &MousePicker, line_renderer: &LineRenderer, text_renderer: &mut TextRenderer, camera: &Camera, projection: &Projection) {
        if self.show_rays {
            if let Some(line) = &mouse_picker.ray {
                line_renderer.render(&camera, &projection, &line, Vector3::new(1.0, 0.0, 0.0), false);
            }
        }

        if self.debug_ui {
            let fps = 1.0 / delta_time;
            let fps_text = format!("{:.2} FPS ({:.2}ms)", fps, delta_time * 1000.0);
            text_renderer.render(5,5,65.0, &fps_text);
            let pos = camera.position;
            let bounds = ChunkBounds::parse(pos.to_vec());
            text_renderer.render(5, 50, 65.0, format!("x: {:.2} y: {:.2} z: {:.2}", pos.x, pos.y, pos.z).as_str());
            text_renderer.render(5, 95, 65.0, format!("yaw: {:?} pitch {:?}", camera.yaw, camera.pitch).as_str());
            text_renderer.render(5, 140, 65.0, format!("Chunk: xMin: {} yMin: {} zMin: {}", bounds.min.0, bounds.min.1, bounds.min.2).as_str());
            text_renderer.render(5, 185, 65.0, format!("       xMax: {} yMax: {} zMax: {}", bounds.max.0, bounds.max.1, bounds.max.2).as_str());
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
                    let color = if (i == 0 || i == 8) && (j == 0 || j == 8) {
                        Vector3::new(1.0, 0.0, 0.0)
                    } else {
                        Vector3::new(1.0, 1.0, 0.0)
                    };
                    line_renderer.render(camera, projection, &line, color, false);
                }
            }
        }
    }
}