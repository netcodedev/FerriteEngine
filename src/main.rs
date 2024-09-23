use glfw::Context;
use cgmath::Deg;

mod shader;
mod mesh;
mod camera;
mod debug;
mod text;
mod terrain;
mod line;
mod texture;
mod model;
mod utils;
use camera::{Camera, CameraController, Projection, MousePicker};
use debug::DebugController;
use text::TextRenderer;
use line::{Line, LineRenderer};
use model::Model;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut glfw = glfw::init(glfw::log_errors).unwrap_or_else(|err| {
        eprintln!("Fehler bei der GLFW-Initialisierung: {}", err);
        std::process::exit(1);
    });

    let (width, height) = (1280, 720);

    let (mut window, events) = glfw.create_window(width, height, "Voxel engine", glfw::WindowMode::Windowed)
        .expect("Fenster konnte nicht erstellt werden");

    window.make_current();
    window.set_key_polling(true);
    window.set_mouse_button_polling(true);

    gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

    window.set_cursor_mode(glfw::CursorMode::Disabled);
    window.set_cursor_pos_polling(true);
    window.set_framebuffer_size_polling(true);

    window.set_cursor_pos(0.0, 0.0);

    let mut camera: Camera = Camera::new((0.0, 92.0, 2.0), Deg(0.0), Deg(90.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let mut camera_controller: CameraController = CameraController::new(10.0, 1.0);
    let mut debug_controller: DebugController = DebugController::new();

    let mut mouse_picker = MousePicker::new();

    let mut terrain = terrain::Terrain::new();

    let mut text_renderer = TextRenderer::new(width, height);
    let line_renderer = LineRenderer::new();

    let mut model = Model::new("assets/models/char_anim.fbx")?;
    model.init();

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        glfw.poll_events();
        let mut line: Option<(Line, glfw::MouseButton)> = None;
        for (_, event) in glfw::flush_messages(&events) {
            camera_controller.process_keyboard(&event);
            camera_controller.process_mouse(&mut window, &event);
            debug_controller.process_keyboard(&mut glfw, &mut window, &event);
            line = mouse_picker.process_mouse(&event, &camera, &projection);
            projection.resize(&event);
            text_renderer.resize(&event);
        }

        terrain.process_line(line);

        let delta_time = calculate_frametime(&glfw);
        camera_controller.update_camera(&mut camera, delta_time as f32);

        terrain.update();
        terrain.render(&camera, &projection);

        model.render(&camera, &projection);
        model.render_bones(&line_renderer, &camera, &projection);

        debug_controller.draw_debug_ui(delta_time as f32, &mouse_picker, &line_renderer, &mut text_renderer, &camera, &projection);

        window.swap_buffers();
    }

    Ok(())
}

fn calculate_frametime(glfw: &glfw::Glfw) -> f64 {
    static mut LAST_FRAME_TIME: f64 = 0.0;
    let current_time = glfw.get_time();
    let delta_time;
    unsafe {
        delta_time = current_time - LAST_FRAME_TIME;
        LAST_FRAME_TIME = current_time;
    }
    delta_time
}