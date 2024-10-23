use cgmath::Deg;
use std::{cell::RefCell, rc::Rc};

mod camera;
mod debug;
mod dual_contouring;
mod line;
mod marching_cubes;
mod model;
mod plane;
mod shader;
mod terrain;
mod text;
mod texture;
mod ui;
mod utils;
mod voxel;
mod window;
use camera::{Camera, CameraController, MousePicker, Projection};
use debug::DebugController;
use dual_contouring::DualContouringChunk;
use line::Line;
use model::Model;
use plane::PlaneRenderer;
use terrain::Terrain;
use text::TextRenderer;
use ui::{UIRenderer, UI};
use window::Window;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280, 720);
    let mut window = Window::new(width, height);

    TextRenderer::resize(width, height);
    PlaneRenderer::resize(width, height);
    let mut ui = UIRenderer::new();

    let mut camera: Camera = Camera::new((0.0, 92.0, 2.0), Deg(-90.0), Deg(0.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let camera_controller: Rc<RefCell<CameraController>> =
        Rc::new(RefCell::new(CameraController::new(10.0, 1.0)));
    let mut debug_controller: DebugController = DebugController::new();

    let mut mouse_picker = MousePicker::new();

    let mut terrain = Terrain::<DualContouringChunk>::new();

    let mut models: Vec<&mut Model> = Vec::new();
    let mut model = Model::new("assets/models/char_anim.fbx")?;
    model.init();
    model.play_animation("mixamo.com");
    models.push(&mut model);

    let camera_controller_ref1 = Rc::clone(&camera_controller);
    let camera_controller_ref2 = Rc::clone(&camera_controller);
    let camera_controller_ref3 = Rc::clone(&camera_controller);
    ui.add(UI::panel("Camera controls", |builder| {
        builder
            .position(10.0, 120.0)
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

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        let mut line: Option<(Line, glfw::MouseButton)> = None;

        window.handle_events(|mut window, mut glfw, event| {
            if ui.handle_events(window, &mut glfw, &event) {
                return;
            }
            let mut camera_controller = camera_controller.borrow_mut();
            camera_controller.process_keyboard(&mut window, &event);
            camera_controller.process_mouse(&mut window, &event);
            projection.resize(&event);
            debug_controller.process_keyboard(&mut glfw, &event);
            line = mouse_picker.process_mouse(&event, &camera, &projection);
            PlaneRenderer::resize_from_event(&event);
            TextRenderer::resize_from_event(&event);
        });

        terrain.process_line(line);

        let delta_time = window.calculate_frametime();
        {
            let mut camera_controller = camera_controller.borrow_mut();
            camera_controller.update_camera(&mut camera, delta_time as f32);
        }

        terrain.update();
        terrain.render(&camera, &projection);

        for model in models.iter_mut() {
            model.update_and_render(delta_time as f32, &camera, &projection);
        }

        ui.render();

        debug_controller.draw_debug_ui(
            delta_time as f32,
            &camera,
            &projection,
            &mouse_picker,
            &models,
        );

        window.swap_buffers();
    }

    Ok(())
}
