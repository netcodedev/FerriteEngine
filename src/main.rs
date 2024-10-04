use cgmath::Deg;

mod camera;
mod debug;
mod line;
mod marching_cubes;
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
use marching_cubes::Chunk;
use plane::PlaneRenderer;
use terrain::Terrain;
use text::TextRenderer;
use line::Line;
use model::Model;
use ui::{button::ButtonBuilder, input::InputBuilder, panel::PanelBuilder, text::Text, UIRenderer};
use window::Window;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280, 720);
    let mut window = Window::new(width, height);

    TextRenderer::resize(width, height);
    PlaneRenderer::resize(width, height);
    let mut ui = UIRenderer::new();

    let mut camera: Camera = Camera::new((0.0, 128.0, 2.0), Deg(-90.0), Deg(0.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let mut camera_controller: CameraController = CameraController::new(10.0, 1.0);
    let mut debug_controller: DebugController = DebugController::new();

    let mut mouse_picker = MousePicker::new();

    // let mut terrain = Terrain::new();

    let mut models: Vec<&mut Model> = Vec::new();
    let mut model = Model::new("assets/models/char_anim.fbx")?;
    model.init();
    model.play_animation("mixamo.com");
    models.push(&mut model);

    ui.add(PanelBuilder::new("Test Panel".to_string())
        .position(10.0, 120.0)
        .size(200.0, 200.0)
        .add_child(Box::new(ButtonBuilder::new()
            .size(100.0, 20.0)
            .on_click(Box::new(|| {println!("button clicked")}))
            .add_child(Box::new(Text::new("Click me!", 16.0)))
            .build()
        ))
        .add_child(Box::new(Text::new("Hello World!", 16.0)))
        .add_child(Box::new(InputBuilder::new("Input".to_string())
            .size(190.0, 26.0)
            .build()
        ))
        .build()
    );

    let mut chunk = Chunk::new((0.0, 0.0, 0.0));

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        let mut line: Option<(Line, glfw::MouseButton)> = None;

        window.handle_events(|mut window, mut glfw, event| {
            camera_controller.process_keyboard(&mut window, &event);
            camera_controller.process_mouse(&mut window, &event);
            projection.resize(&event);
            ui.handle_events(window, &mut glfw, &event);
            debug_controller.process_keyboard(&mut glfw, &event);
            line = mouse_picker.process_mouse(&event, &camera, &projection);
            PlaneRenderer::resize_from_event(&event);
            TextRenderer::resize_from_event(&event);
        });

        chunk.render(&camera, &projection);
        // terrain.process_line(line);

        let delta_time = window.calculate_frametime();
        camera_controller.update_camera(&mut camera, delta_time as f32);

        // terrain.update();
        // terrain.render(&camera, &projection);

        for model in models.iter_mut() {
            model.update_and_render(delta_time as f32, &camera, &projection);
        }

        ui.render();

        debug_controller.draw_debug_ui(delta_time as f32, &camera, &projection, &mouse_picker, &models);

        window.swap_buffers();
    }

    Ok(())
}
