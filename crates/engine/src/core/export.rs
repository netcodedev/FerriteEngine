//! Exporting a project as a game that runs on its own: the player
//! (`crates/sandbox`), the engine library, the compiled scripts and the
//! Rust standard library, next to the project's assets, animations and
//! scenes, all in one folder.
//!
//! ```text
//! <out>/<name>/
//!     <name>                  the player; finds everything below next to it
//!     <name>-server           the dedicated server (no window or GPU)
//!     libferrite_dylib.so     the engine
//!     libferrite_scripts.so   the project's scripts
//!     libstd-<hash>.so        Rust's standard library
//!     <name>.fp               the project file, without editor settings
//!     assets/ animations/ scenes/
//! ```
//!
//! Everything is built in release mode from the engine's workspace, in one
//! cargo run, so player and scripts link the same engine library.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use ron::Value;

use super::{
    project::{Project, PROJECT_EXTENSION},
    script::{engine_workspace, SCRIPTS_LIBRARY},
};

/// Folders of a project the game needs at runtime.
const GAME_FOLDERS: [&str; 3] = ["assets", "animations", "scenes"];

/// Marks a folder as made by an export, so exporting again may replace it.
const MARKER: &str = ".ferrite-export";

const PLAYER_PACKAGE: &str = "sandbox";
/// The dedicated server, exported as `<name>-server`.
const SERVER_PACKAGE: &str = "ferrite-server";
const ENGINE_LIBRARY: &str = "ferrite_dylib";

/// The file name of a library called `name` on this platform.
fn library_file(name: &str) -> String {
    format!(
        "{}{name}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Export `project` into a folder named after it inside `out`; returns
/// that folder. `progress` hears about each step. Blocks while building.
pub fn export(project: &Project, out: &Path, progress: &dyn Fn(&str)) -> Result<PathBuf, String> {
    let name = file_name(project.name());
    let target = out.join(&name);
    prepare_folder(&target)?;

    progress("Building the game (release)...");
    build(project)?;

    progress("Copying the game...");
    let release = engine_workspace().join("target").join("release");
    copy_file(
        &release.join(format!("{PLAYER_PACKAGE}{}", std::env::consts::EXE_SUFFIX)),
        &target.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
    )?;
    copy_file(
        &release.join(format!("{SERVER_PACKAGE}{}", std::env::consts::EXE_SUFFIX)),
        &target.join(format!("{name}-server{}", std::env::consts::EXE_SUFFIX)),
    )?;
    // A dylib dependency is only left in `deps`; the scripts, being built
    // as a package of their own, are copied up as well.
    let engine = library_file(ENGINE_LIBRARY);
    copy_file(&release.join("deps").join(&engine), &target.join(&engine))?;
    let scripts = library_file(SCRIPTS_LIBRARY);
    copy_file(&release.join(&scripts), &target.join(&scripts))?;
    for std_library in standard_libraries()? {
        let file_name = std_library.file_name().unwrap_or_default();
        copy_file(&std_library, &target.join(file_name))?;
    }

    progress("Copying assets and scenes...");
    for folder in GAME_FOLDERS {
        let from = project.dir().join(folder);
        if from.is_dir() {
            copy_dir(&from, &target.join(folder))?;
        }
    }
    let mut file = project.file.clone();
    file.editor = Value::Unit;
    let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    let project_file = target.join(format!("{name}.{PROJECT_EXTENSION}"));
    std::fs::write(&project_file, text).map_err(|e| format!("{}: {e}", project_file.display()))?;
    std::fs::write(target.join(MARKER), "").map_err(|e| e.to_string())?;
    Ok(target)
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

/// Make `target` an empty folder. Only an earlier export, or an empty
/// folder, is replaced; anything else stays untouched.
fn prepare_folder(target: &Path) -> Result<(), String> {
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
        std::fs::remove_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    std::fs::create_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))
}

/// Build the player and the project's scripts in release mode.
fn build(project: &Project) -> Result<(), String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(engine_workspace())
        .args(["build", "--release", "--message-format=short"])
        .args(["-p", PLAYER_PACKAGE, "-p", SERVER_PACKAGE, "-p", "ferrite_scripts"])
        .env("FERRITE_SCRIPTS", project.scripts().sources())
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let first = stderr
        .lines()
        .find(|line| line.contains("error"))
        .unwrap_or("the build failed");
    Err(first.trim().to_string())
}

/// Rust's standard library as a shared library, which the engine library
/// and the player link against.
fn standard_libraries() -> Result<Vec<PathBuf>, String> {
    let output = Command::new("rustc")
        .current_dir(engine_workspace())
        .args(["--print", "target-libdir"])
        .output()
        .map_err(|e| format!("could not run rustc: {e}"))?;
    let dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let prefix = format!("{}std-", std::env::consts::DLL_PREFIX);
    let libraries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(&prefix) && n.ends_with(std::env::consts::DLL_SUFFIX)
            })
        })
        .collect();
    if libraries.is_empty() {
        return Err(format!("no standard library in {}", dir.display()));
    }
    Ok(libraries)
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("could not copy {}: {e}", from.display()))
}

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

    #[test]
    fn names_become_file_names() {
        assert_eq!(file_name("My Game"), "My Game");
        assert_eq!(file_name("a/b:c"), "a_b_c");
        assert_eq!(file_name("  "), "game");
        assert_eq!(file_name(".."), "game");
    }

    #[test]
    fn only_earlier_exports_are_replaced() {
        let dir = std::env::temp_dir().join(format!("ferrite-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), "mine").unwrap();
        assert!(prepare_folder(&dir).is_err());
        assert!(dir.join("keep.txt").is_file());

        std::fs::write(dir.join(MARKER), "").unwrap();
        prepare_folder(&dir).unwrap();
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
