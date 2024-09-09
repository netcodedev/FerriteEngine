use glfw::Context;
use cgmath::Matrix;
use std::fs;

mod shader;
mod matrix;
mod mesh;
use mesh::Chunk;
use shader::create_shader_program;
use matrix::create_transformation_matrices;

fn main() {
    let mut glfw = glfw::init(glfw::log_errors).unwrap_or_else(|err| {
        eprintln!("Fehler bei der GLFW-Initialisierung: {}", err);
        std::process::exit(1);
    });

    let (mut window, _events) = glfw.create_window(800, 600, "Voxel engine", glfw::WindowMode::Windowed)
        .expect("Fenster konnte nicht erstellt werden");

    window.make_current();

    gl::load_with(|symbol| window.get_proc_address(symbol) as *const _);

    let vertex_source = fs::read_to_string("src/shaders/vertex.glsl").unwrap();
    let fragment_source = fs::read_to_string("src/shaders/fragment.glsl").unwrap();
    let shader_program = create_shader_program(&vertex_source, &fragment_source);

    //glfw.set_swap_interval(glfw::SwapInterval::None);

    unsafe {
        gl::Enable(gl::DEPTH_TEST);
    }

    let chunk = Chunk::new((0.0, 0.0, 0.0));
    let (mut view, projection) = create_transformation_matrices(&glfw);

    while !window.should_close() {
        unsafe {
            gl::ClearColor(0.3, 0.3, 0.5, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
        }
        unsafe {
            let view_loc = gl::GetUniformLocation(shader_program, "view\0".as_ptr() as *const i8);
            let projection_loc = gl::GetUniformLocation(shader_program, "projection\0".as_ptr() as *const i8);

            gl::UniformMatrix4fv(view_loc, 1, gl::FALSE, view.as_ptr());
            gl::UniformMatrix4fv(projection_loc, 1, gl::FALSE, projection.as_ptr());
        }

        // Render the mesh
        chunk.render(shader_program);

        window.swap_buffers();
        glfw.poll_events();

        // Move the camera
        if window.get_key(glfw::Key::W) == glfw::Action::Press {
            let translation = cgmath::Matrix4::from_translation(cgmath::Vector3::new(0.0, 0.0, 0.1));
            view = view * translation;
        }
        if window.get_key(glfw::Key::S) == glfw::Action::Press {
            let translation = cgmath::Matrix4::from_translation(cgmath::Vector3::new(0.0, 0.0, -0.1));
            view = view * translation;
        }
        if window.get_key(glfw::Key::A) == glfw::Action::Press {
            let translation = cgmath::Matrix4::from_translation(cgmath::Vector3::new(0.1, 0.0, 0.0));
            view = view * translation;
        }
        if window.get_key(glfw::Key::D) == glfw::Action::Press {
            let translation = cgmath::Matrix4::from_translation(cgmath::Vector3::new(-0.1, 0.0, 0.0));
            view = view * translation;
        }
        if window.get_key(glfw::Key::Space) == glfw::Action::Press {
            let translation = cgmath::Matrix4::from_translation(cgmath::Vector3::new(0.0, -0.1, 0.0));
            view = view * translation;
        }
        if window.get_key(glfw::Key::LeftShift) == glfw::Action::Press {
            let translation = cgmath::Matrix4::from_translation(cgmath::Vector3::new(0.0, 0.1, 0.0));
            view = view * translation;
        }
        // Handle mouse movement
        let (x, y) = window.get_cursor_pos();
        let sensitivity = 0.001;
        let (width, height) = window.get_size();
        let center_x = width as f64 / 2.0;
        let center_y = height as f64 / 2.0;
        let delta_x = (x - center_x) * sensitivity;
        let delta_y = (y - center_y) * sensitivity;
        
        let rotation_x = cgmath::Matrix4::from_angle_x(cgmath::Rad(delta_y as f32));
        let rotation_y = cgmath::Matrix4::from_angle_y(cgmath::Rad(delta_x as f32));
        
        view = view * rotation_x * rotation_y;
        
        window.set_cursor_pos(center_x, center_y);

        // let (delta_time, fps) = calculate_frametime(&glfw);
        // println!("frametime: {} FPS: {}", delta_time, fps);
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