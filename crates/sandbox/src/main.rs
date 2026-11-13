//! Plays a project: the `.fp` file in the working directory (or the one
//! given as the first argument), with its scripts and its startup scene.

// Links the engine as the shared library the project's scripts link too.
#[allow(unused_imports)]
use ferrite_dylib;

use std::path::PathBuf;

use ferrite::core::{
    application::{Application, Layer},
    project::Project,
    scene::Scene,
    script::{self, load_library},
    window::Window,
};
use glfw::{Glfw, WindowEvent};

fn main() {
    let project = match open_project() {
        Ok(project) => project,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // The scripts, built if needed, before any scene refers to them.
    let scripts = project.scripts();
    match scripts.build().and_then(|()| load_library(&scripts.library_path())) {
        Ok(registry) => script::install(registry),
        Err(e) => eprintln!("scripts: {e}"),
    }

    let mut application = Application::new(1280, 720, project.name());
    let scene = match &project.file.startup_scene {
        Some(path) => Scene::load_from_path(path.as_ref()).unwrap_or_else(|e| {
            eprintln!("could not load {path}: {e}");
            Scene::new()
        }),
        None => {
            eprintln!("the project has no startup scene");
            Scene::new()
        }
    };
    for problem in scene.problems() {
        eprintln!("not loaded: {problem}");
    }
    application.add_layer(Box::new(GameLayer { scene }));
    application.start();
}

fn open_project() -> Result<Project, String> {
    let path = match std::env::args().nth(1) {
        Some(path) => PathBuf::from(path),
        None => {
            let here = std::env::current_dir().map_err(|e| e.to_string())?;
            Project::find_in(&here).ok_or_else(|| {
                format!(
                    "no project (.fp file) in {}; give one as the first argument",
                    here.display()
                )
            })?
        }
    };
    let project = Project::load(&path)?;
    project.enter()?;
    Ok(project)
}

struct GameLayer {
    scene: Scene,
}

impl Layer for GameLayer {
    fn on_update(&mut self, window: &Window, delta_time: f64) {
        self.scene.update(delta_time);
        self.scene.render(window);
    }

    fn on_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        self.scene.handle_event(glfw, window, event);
    }

    fn get_name(&self) -> &str {
        "Game"
    }
}
