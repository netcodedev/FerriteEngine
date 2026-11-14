//! Running a game: the player (a window playing the project's startup
//! scene) and the dedicated server (the same scene without a window or
//! GPU, for players who connect; see `net`). Shared by the development
//! player and server, which load the project's scripts as a library, and
//! by an exported game, which has them built in (`ferrite-game`).

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use glfw::{Glfw, WindowEvent};

use super::{
    application::{self, Application, Layer},
    assets, headless, net,
    project::Project,
    scene::Scene,
    script::{self, load_library, ScriptRegistry},
    window::Window,
};

/// Where a game's scripts come from.
pub enum Scripts {
    /// Built from the project's `scripts` folder and loaded as a library
    /// (or the library next to the executable): for development.
    Library,
    /// Compiled into the executable: an exported game.
    BuiltIn(fn(&mut ScriptRegistry)),
}

impl Scripts {
    fn install(self, project: &Project) {
        let loaded = match self {
            Scripts::BuiltIn(register) => {
                let mut registry = ScriptRegistry::default();
                register(&mut registry);
                Ok(registry)
            }
            Scripts::Library => match script::packaged_library() {
                Some(library) => load_library(&library),
                None => {
                    let scripts = project.scripts();
                    scripts
                        .build()
                        .and_then(|()| load_library(&scripts.library_path()))
                }
            },
        };
        match loaded {
            Ok(registry) => script::install(registry),
            Err(e) => log::warn!("scripts: {e}"),
        }
    }
}

/// The project: `given` (a `.fp` file or a folder with one), or the folder
/// of the executable when it holds one (an exported game), or the working
/// directory. It becomes the working directory, its assets are scanned.
fn open_project(given: Option<PathBuf>) -> Result<Project, String> {
    let given = match given {
        Some(path) => path,
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
        log::warn!("{problem}");
    }
    Ok(project)
}

fn exit_with(error: String) -> ! {
    eprintln!("{error}");
    std::process::exit(1);
}

/// Play the project in a window.
///
/// ```text
/// PLAYER [PROJECT] [--connect ADDRESS]
/// ```
///
/// `--connect` joins that server once the game runs (the game's menu does
/// it, if it asks `net::take_join_on_start`).
pub fn run_player(scripts: Scripts) {
    let mut project_arg = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect" => match args.next() {
                Some(address) => net::set_join_on_start(address),
                None => exit_with("--connect needs an address".into()),
            },
            _ => project_arg = Some(PathBuf::from(arg)),
        }
    }
    let project = open_project(project_arg).unwrap_or_else(|e| exit_with(e));
    // Before any scene refers to them.
    scripts.install(&project);

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
            log::error!("could not load {path}: {e}");
            Scene::new()
        }),
        None => {
            log::error!("the project has no startup scene");
            Scene::new()
        }
    };
    for problem in scene.problems() {
        log::warn!("not loaded: {problem}");
    }
    scene
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

/// Simulation steps per second of the server.
const TICK_RATE: f64 = 60.0;
/// Where the server's world is saved, relative to the project.
const DEFAULT_SAVE: &str = "saves/server.sav";
const DEFAULT_AUTOSAVE: u64 = 60;
const SERVER_USAGE: &str =
    "usage: SERVER [PROJECT] [--port PORT] [--save FILE] [--autosave SECONDS]";

struct ServerOptions {
    project: Option<PathBuf>,
    port: u16,
    save: PathBuf,
    /// Seconds between saves; 0: only when stopped.
    autosave: u64,
}

fn server_options() -> Result<ServerOptions, String> {
    let mut options = ServerOptions {
        project: None,
        port: net::DEFAULT_PORT,
        save: PathBuf::from(DEFAULT_SAVE),
        autosave: DEFAULT_AUTOSAVE,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" | "-p" => {
                let port = args.next().ok_or("--port needs a number")?;
                options.port = port.parse().map_err(|_| format!("not a port: {port}"))?;
            }
            "--save" => {
                options.save = PathBuf::from(args.next().ok_or("--save needs a file")?);
            }
            "--autosave" => {
                let seconds = args.next().ok_or("--autosave needs seconds")?;
                options.autosave = seconds
                    .parse()
                    .map_err(|_| format!("not a number of seconds: {seconds}"))?;
            }
            "--help" | "-h" => return Err(SERVER_USAGE.into()),
            _ if options.project.is_none() => options.project = Some(PathBuf::from(arg)),
            _ => return Err(format!("unexpected argument: {arg}\n{SERVER_USAGE}")),
        }
    }
    Ok(options)
}

/// Be a dedicated server for the project, without a window or GPU.
///
/// ```text
/// SERVER [PROJECT] [--port PORT] [--save FILE] [--autosave SECONDS]
/// ```
///
/// The world is saved to FILE (`saves/server.sav` in the project unless
/// given) every SECONDS (60; 0: never) and when the server is stopped
/// (Ctrl+C, SIGTERM), and carried on from there when it starts again.
pub fn run_server(scripts: Scripts) {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = serve(scripts) {
        exit_with(e);
    }
}

fn serve(scripts: Scripts) -> Result<(), String> {
    let options = server_options()?;
    // Stopped (Ctrl+C, SIGTERM from `docker stop`): saved first.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::Relaxed))
            .map_err(|e| format!("cannot handle signals: {e}"))?;
    }
    headless::enable();
    let project = open_project(options.project)?;
    scripts.install(&project);

    let path = project
        .file
        .startup_scene
        .clone()
        .ok_or("the project has no startup scene")?;
    let started = Instant::now();
    let mut scene = Scene::load_from_path(path.as_ref()).map_err(|e| format!("{path}: {e}"))?;
    for problem in scene.problems() {
        log::warn!("not loaded: {problem}");
    }
    let scene_file = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    net::make_dedicated(&mut scene);
    log::info!(
        "{} loaded in {:.1}s",
        project.name(),
        started.elapsed().as_secs_f32()
    );

    net::host(options.port)?;
    // Players play in this scene, not in their own copy of it.
    net::share_scene(scene_file);
    // Relative to the project, which is the working directory now.
    let save = options.save;
    if save.exists() {
        let count = net::load_world(&save)?;
        log::info!("carrying on {} ({count} entries)", save.display());
    } else {
        log::info!("new world, saved to {}", save.display());
    }
    log::info!("listening on port {} (UDP)", options.port);
    let autosave = (options.autosave > 0).then(|| Duration::from_secs(options.autosave));
    let mut saved = Instant::now();

    let tick = Duration::from_secs_f64(1.0 / TICK_RATE);
    let mut next = Instant::now();
    let mut players = 0;
    while !stop.load(Ordering::Relaxed) {
        scene.update(tick.as_secs_f64());
        if autosave.is_some_and(|every| saved.elapsed() >= every) {
            if let Err(e) = net::save_world(&save) {
                log::warn!("could not save: {e}");
            }
            saved = Instant::now();
        }
        if let net::Status::Hosting { players: now } = net::status() {
            if now != players {
                log::info!("{now} player(s) connected");
                players = now;
            }
        }
        next += tick;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            // Behind (a slow step): carry on from now instead of catching up.
            next = now;
        }
    }
    net::save_world(&save)?;
    log::info!("saved to {}", save.display());
    net::disconnect();
    Ok(())
}
