use glfw::Context;
use cgmath::{Deg, Matrix};
use std::fs;
use std::sync::mpsc;
use std::thread;

mod shader;
mod mesh;
mod camera;
use camera::{Camera, CameraController, Projection};
use mesh::Chunk;
use shader::create_shader_program;

const WIREFRAME: bool = false;
const SHOW_FPS: bool = false;
const VSYNC: bool = true;

fn main() {
    let mut glfw = glfw::init(glfw::log_errors).unwrap_or_else(|err| {
        eprintln!("Fehler bei der GLFW-Initialisierung: {}", err);
        std::process::exit(1);
    });

    let (width, height) = (1920, 1080);

    let (mut window, events) = glfw.create_window(width, height, "Voxel engine", glfw::WindowMode::Windowed)
        .expect("Fenster konnte nicht erstellt werden");

    window.make_current();
    window.set_key_polling(true);

    gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

    let vertex_source = fs::read_to_string("src/shaders/vertex.glsl").unwrap();
    let fragment_source = fs::read_to_string("src/shaders/fragment.glsl").unwrap();
    let shader_program = create_shader_program(&vertex_source, &fragment_source);

    if !VSYNC {
        glfw.set_swap_interval(glfw::SwapInterval::None);
    }
    window.set_cursor_mode(glfw::CursorMode::Disabled);
    window.set_cursor_pos_polling(true);
    window.set_framebuffer_size_polling(true);

    unsafe {
        gl::Enable(gl::DEPTH_TEST);
        gl::Enable(gl::CULL_FACE);
    }

    let mut chunks  = Vec::<Chunk>::new();
    let mut camera: Camera = Camera::new((64.0, 200.0, 64.0), Deg(0.0), Deg(0.0));
    let mut projection: Projection = Projection::new(width, height, Deg(45.0), 0.1, 100.0);
    let mut camera_controller: CameraController = CameraController::new(50.0, 1.0);

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

    // wireframe
    if WIREFRAME {
        unsafe {
            gl::PolygonMode(gl::FRONT_AND_BACK, gl::LINE);
        }
    }
    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }

        // Load new chunks
        if let Ok(chunk) = rx.try_recv() {
            chunks.push(chunk);
        }
        
        // Render the mesh
        for chunk in &mut chunks {
            chunk.render(shader_program);
        }

        window.swap_buffers();
        glfw.poll_events();

        for (_, event) in glfw::flush_messages(&events) {
            camera_controller.process_keyboard(&event);
            camera_controller.process_mouse(&mut window, &event);
            projection.resize(&event);
        }

        let (delta_time, fps) = calculate_frametime(&glfw);
        camera_controller.update_camera(&mut camera, delta_time as f32);

        unsafe {
            let view_loc = gl::GetUniformLocation(shader_program, "view\0".as_ptr() as *const i8);
            let projection_loc = gl::GetUniformLocation(shader_program, "projection\0".as_ptr() as *const i8);

            gl::UniformMatrix4fv(view_loc, 1, gl::FALSE, camera.calc_matrix().as_ptr());
            gl::UniformMatrix4fv(projection_loc, 1, gl::FALSE, projection.calc_matrix().as_ptr());
        }

        window.set_cursor_pos(0.0, 0.0);
        if SHOW_FPS {
            println!("frametime: {}ms FPS: {}", delta_time * 1000.0, fps);
        }
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