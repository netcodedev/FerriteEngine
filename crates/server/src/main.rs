//! A dedicated game server: plays a project's startup scene without a
//! window or GPU, for the players who connect (see `ferrite::core::net`).
//!
//! ```text
//! ferrite-server [PROJECT] [--port PORT]
//! ```
//!
//! PROJECT is a `.fp` file or a folder with one: the working directory, or
//! the folder of the executable when it sits next to an exported game.

// Links the engine as the shared library the project's scripts link too.
#[allow(unused_imports)]
use ferrite_dylib;

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use ferrite::core::{
    assets, headless, net,
    project::Project,
    scene::Scene,
    script::{self, load_library},
};

/// Simulation steps per second.
const TICK_RATE: f64 = 60.0;

struct Options {
    project: Option<PathBuf>,
    port: u16,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        project: None,
        port: net::DEFAULT_PORT,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" | "-p" => {
                let port = args.next().ok_or("--port needs a number")?;
                options.port = port.parse().map_err(|_| format!("not a port: {port}"))?;
            }
            "--help" | "-h" => {
                return Err("usage: ferrite-server [PROJECT] [--port PORT]".into());
            }
            _ if options.project.is_none() => options.project = Some(PathBuf::from(arg)),
            _ => return Err(format!("unexpected argument: {arg}")),
        }
    }
    Ok(options)
}

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
        Project::find_in(&given)
            .ok_or_else(|| format!("no project (.fp file) in {}", given.display()))?
    } else {
        given
    };
    let project = Project::load(&path)?;
    project.enter()?;
    Ok(project)
}

fn run() -> Result<(), String> {
    let options = options()?;
    headless::enable();
    let project = open_project(options.project)?;
    let (_, problems) = assets::database().register_new_files();
    for problem in problems {
        log::warn!("{problem}");
    }
    let scripts = match script::packaged_library() {
        Some(library) => load_library(&library),
        None => {
            let scripts = project.scripts();
            scripts
                .build()
                .and_then(|()| load_library(&scripts.library_path()))
        }
    };
    match scripts {
        Ok(registry) => script::install(registry),
        Err(e) => log::warn!("scripts: {e}"),
    }

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
    net::make_dedicated(&mut scene);
    log::info!(
        "{} loaded in {:.1}s",
        project.name(),
        started.elapsed().as_secs_f32()
    );

    net::host(options.port)?;
    log::info!("listening on port {} (UDP)", options.port);

    let tick = Duration::from_secs_f64(1.0 / TICK_RATE);
    let mut next = Instant::now();
    let mut players = 0;
    loop {
        scene.update(tick.as_secs_f64());
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
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
