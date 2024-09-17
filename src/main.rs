use glfw::Context;
use cgmath::Deg;
use std::fs;
use std::sync::mpsc;
use std::thread;

mod shader;
mod mesh;
mod camera;
mod debug;
mod text;
use camera::{Camera, CameraController, Projection};
use mesh::Chunk;
use shader::create_shader_program;
use debug::DebugController;
use text::TextRenderer;

fn main() {
    let mut glfw = glfw::init(glfw::log_errors).unwrap_or_else(|err| {
        eprintln!("Fehler bei der GLFW-Initialisierung: {}", err);
        std::process::exit(1);
    });

    let (width, height) = (1280, 720);

    let (mut window, events) = glfw.create_window(width, height, "Voxel engine", glfw::WindowMode::Windowed)
        .expect("Fenster konnte nicht erstellt werden");

    window.make_current();
    window.set_key_polling(true);

    gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

    let vertex_source = fs::read_to_string("src/shaders/vertex.glsl").unwrap();
    let fragment_source = fs::read_to_string("src/shaders/fragment.glsl").unwrap();
    let shader_program = create_shader_program(&vertex_source, &fragment_source);

    window.set_cursor_mode(glfw::CursorMode::Disabled);
    window.set_cursor_pos_polling(true);
    window.set_framebuffer_size_polling(true);

    let mut chunks  = Vec::<Chunk>::new();
    let mut camera: Camera = Camera::new((64.0, 200.0, 64.0), Deg(0.0), Deg(0.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let mut camera_controller: CameraController = CameraController::new(50.0, 1.0);
    let mut debug_controller: DebugController = DebugController::new();

    window.set_cursor_pos(0.0, 0.0);

    let (tx, rx) = mpsc::channel();
    let origin = Chunk::new((0.0, 0.0, 0.0));
    tx.send(origin).unwrap();

    let tx1 = tx.clone();
    let tx2 = tx.clone();
    let tx3 = tx.clone();
    let tx4 = tx.clone();
    const RADIUS: i32 = 5;
    let _ = thread::spawn(move || mesh::chunkloader(RADIUS,1,1,tx1));
    let _ = thread::spawn(move || mesh::chunkloader(RADIUS,-1,1,tx2));
    let _ = thread::spawn(move || mesh::chunkloader(RADIUS,1,-1,tx3));
    let _ = thread::spawn(move || mesh::chunkloader(RADIUS,-1,-1,tx4));

    let mut text_renderer = TextRenderer::new(width, height);

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        glfw.poll_events();
        for (_, event) in glfw::flush_messages(&events) {
            camera_controller.process_keyboard(&event);
            camera_controller.process_mouse(&mut window, &event);
            debug_controller.process_keyboard(&mut glfw, &mut window, &event);
            projection.resize(&event);
            text_renderer.resize(&event);
        }

        let (delta_time, fps) = calculate_frametime(&glfw);
        camera_controller.update_camera(&mut camera, delta_time as f32);

        // Load new chunks
        if let Ok(chunk) = rx.try_recv() {
            chunks.push(chunk);
        }
        
        // Render the mesh
        for chunk in &mut chunks {
            chunk.render(&camera, &projection, shader_program);
        }

        if debug_controller.show_fps {
            let fps_text = format!("{:.2} FPS, Frametime: {:.2}", fps, delta_time * 1000.0);
            text_renderer.render(5,5,50.0, &fps_text);
        }

        window.swap_buffers();
    }
}

fn calculate_frametime(glfw: &glfw::Glfw) -> (f64, f64) {
    static mut LAST_FRAME_TIME: f64 = 0.0;
    let current_time = glfw.get_time();
    let delta_time;
    unsafe {
        delta_time = current_time - LAST_FRAME_TIME;
        LAST_FRAME_TIME = current_time;
    }
    let fps = 1.0 / delta_time;
    (delta_time, fps)
}