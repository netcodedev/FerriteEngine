use std::{cell::RefCell, rc::Rc};

use glfw::{Action, Glfw, Key};

use crate::{camera::{Camera, MousePicker, Projection}, line::{Line, LineRenderer}, model::Model, terrain::{ChunkBounds, CHUNK_SIZE}, text::TextRenderer};
use cgmath::{Deg, EuclideanSpace, Point3, Vector3};

pub struct DebugController {
    text_renderer: Rc<RefCell<TextRenderer>>,
    line_renderer: Rc<RefCell<LineRenderer>>,
    pub debug_ui: bool,
    wireframe: bool,
    vsync: bool,
    show_rays: bool,
}

impl DebugController {
    pub fn new(text_renderer: Rc<RefCell<TextRenderer>>, line_renderer: Rc<RefCell<LineRenderer>>) -> Self {
        Self {
            text_renderer,
            line_renderer,
            debug_ui: false,
            wireframe: false,
            vsync: true,
            show_rays: false,
        }
    }

    pub fn process_keyboard(&mut self, glfw: &mut Glfw, event: &glfw::WindowEvent) {
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

    pub fn draw_debug_ui(&self, delta_time: f32, camera: &Camera, projection: &Projection, mouse_picker: &MousePicker, models: &Vec<&mut Model>) {
        if self.show_rays {
            if let Some(line) = &mouse_picker.ray {
                self.line_renderer.borrow().render(&camera, &projection, &line, Vector3::new(1.0, 0.0, 0.0), false);
            }
        }

        if self.debug_ui {
            let mut text_renderer = self.text_renderer.borrow_mut();
            let fps = 1.0 / delta_time;
            let fps_text = format!("{:.2} FPS ({:.2}ms)", fps, delta_time * 1000.0);
            text_renderer.render(5,5,20.0, &fps_text);
            let pos = camera.position;
            let bounds = ChunkBounds::parse(pos.to_vec());
            text_renderer.render(5, 25, 20.0, format!("x: {:.2} y: {:.2} z: {:.2}", pos.x, pos.y, pos.z).as_str());
            text_renderer.render(5, 45, 20.0, format!("yaw: {:?} pitch {:?}", Deg::from(camera.yaw), Deg::from(camera.pitch)).as_str());
            text_renderer.render(5, 65, 20.0, format!("Chunk: xMin: {} yMin: {} zMin: {}", bounds.min.0, bounds.min.1, bounds.min.2).as_str());
            text_renderer.render(5, 85, 20.0, format!("       xMax: {} yMax: {} zMax: {}", bounds.max.0, bounds.max.1, bounds.max.2).as_str());
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
            self.line_renderer.borrow().render_lines(camera, projection, &lines, Vector3::new(1.0, 1.0, 0.0), false);
            self.line_renderer.borrow().render_lines(camera, projection, &corner_lines, Vector3::new(1.0, 0.0, 0.0), false);

            for model in models {
                model.render_bones(&self.line_renderer.borrow(), camera, projection);
            }
        }
    }
}