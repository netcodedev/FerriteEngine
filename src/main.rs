use std::{cell::RefCell, rc::Rc};

use cgmath::Deg;

mod camera;
mod debug;
mod line;
mod model;
mod plane;
mod shader;
mod text;
mod terrain;
mod texture;
mod utils;
mod ui;
mod window;
use camera::{Camera, CameraController, Projection, MousePicker};
use debug::DebugController;
use terrain::Terrain;
use text::TextRenderer;
use line::{Line, LineRenderer};
use model::Model;
use ui::{button::ButtonBuilder, container::ContainerBuilder, text::Text, UIRenderer};
use window::Window;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280, 720);
    let mut window = Window::new(width, height);

    let text_renderer = Rc::new(RefCell::new(TextRenderer::new(width, height)));
    let line_renderer = Rc::new(RefCell::new(LineRenderer::new()));
    let plane_renderer = Rc::new(RefCell::new(plane::PlaneRenderer::new()));
    let ui = Rc::new(RefCell::new(UIRenderer::new(Rc::clone(&text_renderer), Rc::clone(&plane_renderer))));

    let mut camera: Camera = Camera::new((0.0, 92.0, 2.0), Deg(-90.0), Deg(0.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let mut camera_controller: CameraController = CameraController::new(1.0, 1.0);
    let mut debug_controller: DebugController = DebugController::new(Rc::clone(&text_renderer), Rc::clone(&line_renderer));

    let mut mouse_picker = MousePicker::new();

    let mut terrain = Terrain::new();


    let mut models: Vec<&mut Model> = Vec::new();
    let mut model = Model::new("assets/models/char_anim.fbx")?;
    model.init();
    model.play_animation("mixamo.com");
    models.push(&mut model);

    ui.borrow_mut().add(ContainerBuilder::new()
        .position(10.0, 10.0)
        .size(200.0, 200.0)
        .add_child(Box::new(ButtonBuilder::new()
            .position(15.0, 15.0)
            .size(100.0, 20.0)
            .on_click(Box::new(|| {println!("button clicked")}))
            .add_child(Box::new(Text::new("Hello World!", 30.0)))
            .build()
        ))
        .build()
    );

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        let mut line: Option<(Line, glfw::MouseButton)> = None;

        window.handle_events(|mut window, mut glfw, event| {
            camera_controller.process_keyboard(&event);
            camera_controller.process_mouse(&mut window, &event);
            ui.borrow_mut().handle_events(window, &event);
            debug_controller.process_keyboard(&mut glfw, &mut window, &event);
            line = mouse_picker.process_mouse(&event, &camera, &projection);
            projection.resize(&event);
            text_renderer.borrow_mut().resize(&event);
        });

        terrain.process_line(line);

        let delta_time = window.calculate_frametime();
        camera_controller.update_camera(&mut camera, delta_time as f32);

        terrain.update();
        terrain.render(&camera, &projection);

        for model in models.iter_mut() {
            model.update_and_render(delta_time as f32, &camera, &projection);
        }

        ui.borrow().render();

        debug_controller.draw_debug_ui(delta_time as f32, &camera, &projection, &mouse_picker, &models);


        window.swap_buffers();
    }

    Ok(())
}
