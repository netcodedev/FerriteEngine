use cgmath::Deg;
use glfw::{Glfw, MouseButton, WindowEvent};
use std::{cell::RefCell, rc::Rc};

mod core;
mod debug;
mod terrain;
use core::{
    application::{Application, Layer},
    camera::{Camera, CameraController, Projection},
    model::{Model, ModelBuilder},
    mouse_picker::MousePicker,
    renderer::{
        line::Line,
        ui::{UIRenderer, UI},
    },
};
use debug::DebugController;
use terrain::{dual_contouring::DualContouringChunk, Terrain};

fn main() {
    let mut application = Application::new(1280, 720, "Engine");
    if let Ok(layer) = WorldLayer::new(1280, 720) {
        application.add_layer(Box::new(layer));
        application.start();
    }
}

struct WorldLayer {
    camera: Camera,
    line: Option<(Line, MouseButton)>,
    projection: Projection,
    camera_controller: Rc<RefCell<CameraController>>,
    debug_controller: DebugController,
    terrain: Terrain<DualContouringChunk>,
    mouse_picker: MousePicker,
    models: Vec<Model>,
    ui: UIRenderer,
}

impl WorldLayer {
    pub fn new(width: u32, height: u32) -> Result<WorldLayer, Box<dyn std::error::Error>> {
        let start = std::time::Instant::now();
        let mut ui = UIRenderer::new();
        let camera: Camera = Camera::new((-119.4, 52.7, -30.0), Deg(-138.0), Deg(-17.0));
        let projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
        let camera_controller: Rc<RefCell<CameraController>> =
            Rc::new(RefCell::new(CameraController::new(10.0, 1.0)));
        let debug_controller: DebugController = DebugController::new();
        let mouse_picker = MousePicker::new();

        let terrain = Terrain::<DualContouringChunk>::new();

        let mut models: Vec<Model> = Vec::new();
        let mut model = ModelBuilder::new("Mannequin.fbx")?
            .with_animation("idle", "Idle.fbx")
            .with_animation("walk", "Walk.fbx")
            .with_animation("run", "Run.fbx")
            .build();
        model.init();
        model.blend_animations("walk", "run", 0.5, true);
        model.play_animation("idle");
        models.push(model);

        let camera_controller_ref1 = Rc::clone(&camera_controller);
        let camera_controller_ref2 = Rc::clone(&camera_controller);
        let camera_controller_ref3 = Rc::clone(&camera_controller);
        ui.add(UI::panel("Camera controls", |builder| {
            builder
                .position(10.0, 130.0)
                .add_child(UI::text("Camera Speed", 16.0, |b| b))
                .add_child(UI::input(|input| {
                    input
                        .size(190.0, 26.0)
                        .get_fn(move || {
                            let camera_controller = camera_controller_ref1.borrow();
                            camera_controller.get_speed().to_string()
                        })
                        .set_fn(move |v| {
                            let mut camera_controller = camera_controller_ref2.borrow_mut();
                            match v.parse::<f32>() {
                                Ok(v) => camera_controller.set_speed(v),
                                Err(_) => {}
                            }
                        })
                }))
                .add_child(UI::button(
                    "Reset Speed",
                    Box::new(move || {
                        let mut camera_controller = camera_controller_ref3.borrow_mut();
                        camera_controller.set_speed(10.0);
                    }),
                    |b| b,
                ))
        }));

        println!("WorldLayer created in {:?}", start.elapsed());

        Ok(Self {
            camera,
            line: None,
            projection,
            camera_controller,
            debug_controller,
            terrain,
            mouse_picker,
            models,
            ui,
        })
    }
}

impl Layer for WorldLayer {
    fn on_update(&mut self, delta_time: f64) {
        {
            let mut camera_controller = self.camera_controller.borrow_mut();
            camera_controller.update_camera(&mut self.camera, delta_time as f32);
        }

        self.terrain.process_line(self.line.clone());
        self.terrain.update();
        self.terrain.render(&self.camera, &self.projection);

        for model in self.models.iter_mut() {
            model.update_and_render(delta_time as f32, &self.camera, &self.projection);
        }

        self.ui.render();

        self.debug_controller.draw_debug_ui(
            delta_time as f32,
            &self.camera,
            &self.projection,
            &self.mouse_picker,
            &self.terrain,
            &self.models,
        );
    }

    fn on_event(
        &mut self,
        glfw: &mut Glfw,
        window: &mut glfw::Window,
        event: &WindowEvent,
    ) {
        if self.ui.handle_events(window, glfw, &event) {
            return;
        }
        let mut camera_controller = self.camera_controller.borrow_mut();
        camera_controller.process_keyboard(window, &event);
        camera_controller.process_mouse(window, &event);
        self.projection.resize(&event);
        self.debug_controller.process_keyboard(glfw, &event);
        self.line = self
            .mouse_picker
            .process_mouse(&event, &self.camera, &self.projection);
    }

    fn get_name(&self) -> &str {
        "World"
    }
}
