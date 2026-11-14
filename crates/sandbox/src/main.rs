//! Plays a project: the `.fp` file in the working directory (or the one
//! given as the first argument), with its scripts and its startup scene.
//! An exported game (`ferrite::core::export`) is this player next to the
//! project it plays. `--connect ADDRESS` joins a server right away (the
//! game's menu does it, if it asks `net::take_join_on_start`).

// Links the engine as the shared library the project's scripts link too.
#[allow(unused_imports)]
use ferrite_dylib;

use std::path::PathBuf;

use ferrite::core::{
    application::{self, Application, Layer},
    assets, net,
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

    // The scripts, before any scene refers to them: an exported game has
    // them built next to it, otherwise they are built if needed.
    let loaded = match script::packaged_library() {
        Some(library) => load_library(&library),
        None => {
            let scripts = project.scripts();
            scripts
                .build()
                .and_then(|()| load_library(&scripts.library_path()))
        }
    };
    match loaded {
        Ok(registry) => script::install(registry),
        Err(e) => eprintln!("scripts: {e}"),
    }

    let mut application = Application::new(1280, 720, project.name());
    let startup = project.file.startup_scene.clone();
    let scene = load_scene(startup.as_deref());
    application.add_layer(Box::new(GameLayer { scene, startup }));
    application.start();
}

/// The startup scene, or an empty one.
fn load_scene(path: Option<&str>) -> Scene {
    let scene = match path {
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
    scene
}

fn open_project() -> Result<Project, String> {
    // A project file, or a folder with one. Without one given: the folder
    // of the executable (an exported game), else the working directory.
    // `--connect ADDRESS` joins that server once the game runs.
    let mut positional = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect" => {
                let address = args.next().ok_or("--connect needs an address")?;
                net::set_join_on_start(address);
            }
            _ => positional = Some(arg),
        }
    }
    let given = match positional {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from))
            .filter(|dir| Project::find_in(dir).is_some())
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?,
    };
    let path = if given.is_dir() {
        Project::find_in(&given).ok_or_else(|| {
            format!(
                "no project (.fp file) in {}; give one as the first argument",
                given.display()
            )
        })?
    } else {
        given
    };
    let project = Project::load(&path)?;
    project.enter()?;
    // Files put into the asset folder by hand become assets, as in the
    // editor.
    let (_, problems) = assets::database().register_new_files();
    for problem in problems {
        ferrite::log::warn!("{problem}");
    }
    Ok(project)
}

struct GameLayer {
    scene: Scene,
    startup: Option<String>,
}

impl Layer for GameLayer {
    fn on_update(&mut self, window: &Window, delta_time: f64) {
        if application::take_restart_request() {
            net::disconnect();
            self.scene = load_scene(self.startup.as_deref());
        }
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
