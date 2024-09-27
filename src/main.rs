use cgmath::Deg;

mod shader;
mod camera;
mod debug;
mod text;
mod terrain;
mod line;
mod texture;
mod model;
mod utils;
mod window;
use camera::{Camera, CameraController, Projection, MousePicker};
use debug::DebugController;
use terrain::Terrain;
use text::TextRenderer;
use line::{Line, LineRenderer};
use model::Model;
use window::Window;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280, 720);
    let mut window = Window::new(width, height);

    let mut camera: Camera = Camera::new((0.0, 92.0, 2.0), Deg(-90.0), Deg(0.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let mut camera_controller: CameraController = CameraController::new(1.0, 1.0);
    let mut debug_controller: DebugController = DebugController::new();

    let mut mouse_picker = MousePicker::new();

    let mut terrain = Terrain::new();

    let mut text_renderer = TextRenderer::new(width, height);
    let line_renderer = LineRenderer::new();

    let mut models: Vec<&mut Model> = Vec::new();
    let mut model = Model::new("assets/models/char_anim.fbx")?;
    model.init();
    model.play_animation("mixamo.com");
    models.push(&mut model);

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        let mut line: Option<(Line, glfw::MouseButton)> = None;

        window.handle_events(|mut window, mut glfw, event| {
            camera_controller.process_keyboard(&event);
            camera_controller.process_mouse(&mut window, &event);
            debug_controller.process_keyboard(&mut glfw, &mut window, &event);
            line = mouse_picker.process_mouse(&event, &camera, &projection);
            projection.resize(&event);
            text_renderer.resize(&event);
        });

        terrain.process_line(line);

        let delta_time = window.calculate_frametime();
        camera_controller.update_camera(&mut camera, delta_time as f32);

        terrain.update();
        terrain.render(&camera, &projection);

        for model in models.iter_mut() {
            model.update_and_render(delta_time as f32, &camera, &projection);
        }

        debug_controller.draw_debug_ui(delta_time as f32, &line_renderer, &mut text_renderer, &camera, &projection, &mouse_picker, &models);

        window.swap_buffers();
    }

    Ok(())
}
