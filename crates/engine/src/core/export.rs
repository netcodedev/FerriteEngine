//! Exporting a project as a game that runs on its own, for Linux or
//! Windows: the player and the dedicated server (`crates/game`, each one
//! executable with the engine, the project's scripts, Assimp and GLFW
//! compiled in) next to the project's assets, animations and scenes, all
//! in one folder.
//!
//! ```text
//! <out>/<name>/                 (<name>-windows/ for Windows)
//!     <name>[.exe]              the player
//!     <name>-server[.exe]       the dedicated server (no window or GPU)
//!     <name>.fp                 the project file, without editor settings
//!     assets.pack               models and images, cooked (docs/asset-pack.md)
//!     assets/ animations/ scenes/   the other files, loose
//!     saves/                    what the game saved; kept by a new export
//! ```
//!
//! Built in release mode from the engine's workspace. Windows is cross
//! compiled with MinGW (`x86_64-pc-windows-gnu`): the Rust target and
//! `x86_64-w64-mingw32-gcc` must be installed.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use ron::Value;

use super::{
    project::{Project, PROJECT_EXTENSION},
    script::{engine_workspace, first_error, target_dir},
};

/// Folders of a project the game needs at runtime.
const GAME_FOLDERS: [&str; 3] = ["assets", "animations", "scenes"];
/// What an exported game writes (server worlds, the player's id): kept
/// when it is exported again.
const SAVES: &str = "saves";

/// Marks a folder as made by an export, so exporting again may replace it.
const MARKER: &str = ".ferrite-export";

/// The package with the exported game's player (`game`) and server
/// (`server`).
const GAME_PACKAGE: &str = "ferrite-game";

/// What an export runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Windows,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Linux => "Linux",
            Platform::Windows => "Windows",
        }
    }

    /// The Rust target built for, if it is not this machine's own.
    fn target(self) -> Option<&'static str> {
        match self {
            Platform::Linux if cfg!(target_os = "linux") => None,
            Platform::Linux => Some("x86_64-unknown-linux-gnu"),
            Platform::Windows if cfg!(target_os = "windows") => None,
            Platform::Windows => Some(WINDOWS_TARGET),
        }
    }

    fn exe(self, name: &str) -> String {
        match self {
            Platform::Linux => name.to_string(),
            Platform::Windows => format!("{name}.exe"),
        }
    }

    /// The export's folder name for a game called `name`.
    fn folder(self, name: &str) -> String {
        match self {
            Platform::Linux => name.to_string(),
            Platform::Windows => format!("{name}-windows"),
        }
    }
}

const WINDOWS_TARGET: &str = "x86_64-pc-windows-gnu";
const MINGW_GCC: &str = "x86_64-w64-mingw32-gcc";

/// Export `project` for `platform` into a folder named after it inside
/// `out`; returns that folder. `progress` hears about each step. Blocks
/// while building.
pub fn export(
    project: &Project,
    out: &Path,
    platform: Platform,
    progress: &dyn Fn(&str),
) -> Result<PathBuf, String> {
    let name = file_name(project.name());
    let target = out.join(platform.folder(&name));
    check_toolchain(platform)?;

    progress(&format!("Building the game for {} (release)...", platform.label()));
    build(project, platform)?;

    progress("Copying the game...");
    let saves = prepare_folder(&target)?;
    let copied = copy_game(project, platform, &name, &target, progress);
    // Whatever failed, the saves go back into the folder: they are the
    // game's worlds, not something to rebuild.
    let restored = match saves {
        Some(saves) => std::fs::rename(&saves, target.join(SAVES)).map_err(|e| {
            format!("could not put {} back into {}: {e}", saves.display(), target.display())
        }),
        None => Ok(()),
    };
    copied?;
    restored?;
    Ok(target)
}

/// Copy the built game, the project's folders and its project file into
/// `target`, an empty folder.
fn copy_game(
    project: &Project,
    platform: Platform,
    name: &str,
    target: &Path,
    progress: &dyn Fn(&str),
) -> Result<(), String> {
    let mut built = target_dir();
    if let Some(triple) = platform.target() {
        built = built.join(triple);
    }
    let built = built.join("release");
    copy_file(&built.join(platform.exe("game")), &target.join(platform.exe(name)))?;
    copy_file(
        &built.join(platform.exe("server")),
        &target.join(platform.exe(&format!("{name}-server"))),
    )?;

    copy_folders(project, target, progress)?;
    let mut file = project.file.clone();
    file.editor = Value::Unit;
    let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    let project_file = target.join(format!("{name}.{PROJECT_EXTENSION}"));
    std::fs::write(&project_file, text).map_err(|e| format!("{}: {e}", project_file.display()))
}

/// What building for `platform` needs on this machine, or what to install.
fn check_toolchain(platform: Platform) -> Result<(), String> {
    let Some(triple) = platform.target() else {
        return Ok(());
    };
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == triple))
        .unwrap_or(true);
    if !installed {
        return Err(format!("the Rust target is missing: rustup target add {triple}"));
    }
    if triple == WINDOWS_TARGET && Command::new(MINGW_GCC).arg("--version").output().is_err() {
        return Err(format!(
            "{MINGW_GCC} is missing: install MinGW-w64 (Arch: pacman -S mingw-w64-gcc)"
        ));
    }
    Ok(())
}

/// `name` usable as a file name.
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "-_. ".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "game".into()
    } else {
        cleaned
    }
}

/// Make `target` an empty folder, marked as an export's. Only an earlier
/// export, or an empty folder, is replaced; anything else stays untouched.
/// What the earlier export saved is moved aside: returned, to be put back.
/// If this fails, the saves are back where they were.
fn prepare_folder(target: &Path) -> Result<Option<PathBuf>, String> {
    let aside = target.with_extension("saves-kept");
    let saves = target.join(SAVES);
    // Left by an export that failed before it could put them back: the
    // world the game had then.
    let mut kept = aside.is_dir().then(|| aside.clone());
    if kept.is_some() && saves.is_dir() {
        return Err(format!(
            "{} holds the saves of an earlier export that did not finish, and {} has saves \
             too; keep one of them (move it to {}), remove the other, and export again",
            aside.display(),
            target.display(),
            saves.display()
        ));
    }
    if target.exists() {
        let empty = std::fs::read_dir(target)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if !empty && !target.join(MARKER).is_file() {
            return Err(format!(
                "{} exists and is not an earlier export; choose another folder",
                target.display()
            ));
        }
        if saves.is_dir() {
            std::fs::rename(&saves, &aside)
                .map_err(|e| format!("could not keep {}: {e}", saves.display()))?;
            kept = Some(aside.clone());
        }
        if let Err(e) = std::fs::remove_dir_all(target) {
            // Some of it is still there (a running game's executable, on
            // Windows): leave it an export, with its saves.
            let _ = std::fs::create_dir_all(target);
            let _ = std::fs::write(target.join(MARKER), "");
            if kept.is_some() {
                let _ = std::fs::rename(&aside, &saves);
            }
            return Err(format!("{}: {e}", target.display()));
        }
    }
    std::fs::create_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
    // Marked first: an export that fails halfway may still be replaced.
    std::fs::write(target.join(MARKER), "").map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(kept)
}

/// Build the game (player and server, scripts compiled in) for `platform`
/// in release mode.
fn build(project: &Project, platform: Platform) -> Result<(), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    crate::core::script::clean_cargo_env(&mut command);
    command
        .current_dir(engine_workspace())
        .args(["build", "--release", "--message-format=short", "-p", GAME_PACKAGE])
        .env("FERRITE_SCRIPTS", project.scripts().sources());
    if let Some(triple) = platform.target() {
        command.args(["--target", triple]);
    }
    let output = command
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(first_error(&stderr, Some(project.scripts().sources())))
}

/// The project's folders: models and images cooked into the asset pack,
/// the rest copied loose. Without the `cook` feature, all of it loose.
#[cfg(feature = "cook")]
fn copy_folders(project: &Project, target: &Path, progress: &dyn Fn(&str)) -> Result<(), String> {
    progress("Cooking assets...");
    let last = std::sync::Mutex::new(std::time::Instant::now());
    let report = |what: &str, done: usize, of: usize| {
        // A line now and then: the export dialog shows the last one.
        let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
        if done == of || last.elapsed().as_millis() > 250 {
            *last = std::time::Instant::now();
            log::info!("cooking {done}/{of}: {what}");
        }
    };
    let pack = target.join(super::assets::pack::FILE_NAME);
    let cooked = super::assets::cook::cook_project(project.dir(), &GAME_FOLDERS, &pack, &report)?;
    for (path, why) in &cooked.failed {
        log::warn!("export: {path} not cooked ({why}); copied as it is");
    }
    log::info!(
        "export: {} files in the pack ({} cooked anew, {:.1} MB), {} loose",
        cooked.packed.len(),
        cooked.cooked,
        cooked.pack_size as f64 / 1e6,
        cooked.loose.len()
    );
    progress("Copying the other files...");
    for relative in &cooked.loose {
        let to = target.join(relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        copy_file(&project.dir().join(relative), &to)?;
    }
    Ok(())
}

#[cfg(not(feature = "cook"))]
fn copy_folders(project: &Project, target: &Path, progress: &dyn Fn(&str)) -> Result<(), String> {
    progress("Copying assets and scenes...");
    for folder in GAME_FOLDERS {
        let from = project.dir().join(folder);
        if from.is_dir() {
            copy_dir(&from, &target.join(folder))?;
        }
    }
    Ok(())
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("could not copy {}: {e}", from.display()))
}

#[cfg_attr(feature = "cook", allow(dead_code))]
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for path in entries.flatten().map(|e| e.path()) {
        let Some(name) = path.file_name() else {
            continue;
        };
        if path.is_dir() {
            copy_dir(&path, &to.join(name))?;
        } else {
            copy_file(&path, &to.join(name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Export the project in FERRITE_EXPORT_PROJECT into FERRITE_EXPORT_OUT
    /// for Linux (a release build: minutes). For trying exports by hand.
    #[test]
    #[ignore]
    fn export_a_project_by_hand() {
        let (Ok(project), Ok(out)) = (std::env::var("FERRITE_EXPORT_PROJECT"), std::env::var("FERRITE_EXPORT_OUT")) else {
            eprintln!("set FERRITE_EXPORT_PROJECT and FERRITE_EXPORT_OUT");
            return;
        };
        let _ = env_logger::builder().is_test(true).filter_level(log::LevelFilter::Info).try_init();
        let path = Project::find_in(Path::new(&project)).expect("a project file");
        let project = Project::load(&path).expect("the project");
        project.enter().expect("into the project");
        let folder = export(&project, Path::new(&out), Platform::Linux, &|step| eprintln!("{step}")).unwrap();
        eprintln!("exported to {}", folder.display());
    }

    #[test]
    fn names_become_file_names() {
        assert_eq!(file_name("My Game"), "My Game");
        assert_eq!(file_name("a/b:c"), "a_b_c");
        assert_eq!(file_name("  "), "game");
        assert_eq!(file_name(".."), "game");
    }

    #[test]
    fn only_earlier_exports_are_replaced_and_their_saves_kept() {
        let dir = std::env::temp_dir().join(format!("ferrite-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), "mine").unwrap();
        assert!(prepare_folder(&dir).is_err());
        assert!(dir.join("keep.txt").is_file());

        std::fs::write(dir.join(MARKER), "").unwrap();
        std::fs::create_dir_all(dir.join(SAVES)).unwrap();
        std::fs::write(dir.join(SAVES).join("world.sav"), "w").unwrap();
        let kept = prepare_folder(&dir).unwrap().unwrap();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, [MARKER]);
        assert!(kept.join("world.sav").is_file());

        // An export that failed after that left them aside: the next one
        // takes them, and does not throw them away.
        assert_eq!(prepare_folder(&dir).unwrap(), Some(kept.clone()));
        assert!(kept.join("world.sav").is_file());
        // Unless the game saved a new world since: then it asks.
        std::fs::create_dir_all(dir.join(SAVES)).unwrap();
        assert!(prepare_folder(&dir).is_err());
        assert!(kept.join("world.sav").is_file());
        assert!(dir.join(SAVES).is_dir());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&kept);
    }

    #[test]
    fn windows_exports_have_their_own_folder_and_exe_names() {
        assert_eq!(Platform::Windows.folder("demo"), "demo-windows");
        assert_eq!(Platform::Windows.exe("demo-server"), "demo-server.exe");
        assert_eq!(Platform::Linux.exe("demo"), "demo");
    }
}
