//! Turning a project's `scripts` folder into a library, and loading it.
//!
//! The scripts are compiled by the crate `ferrite_scripts` of the engine's
//! workspace, whose build script includes every `.rs` file of the folder
//! given in FERRITE_SCRIPTS as a module and registers every
//! `#[derive(Script)]` struct in them. Built in the engine's workspace, it
//! links the very engine library the editor runs with; a crate of its own
//! elsewhere would make cargo build a second, incompatible one.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU32, Ordering},
};

use super::ScriptRegistry;

/// Name of the function `export_scripts!` exports from a script library,
/// a `fn(&mut ScriptRegistry)`.
pub const REGISTER_SYMBOL: &[u8] = b"ferrite_register_scripts";

const SCRIPTS_PACKAGE: &str = "ferrite_scripts";

/// Name of the library the scripts are compiled into (without the
/// platform's `lib` prefix and extension).
pub const SCRIPTS_LIBRARY: &str = SCRIPTS_PACKAGE;

/// The scripts library an exported game ships next to its executable, if
/// this process is such a game.
pub fn packaged_library() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    // A player or server built in the engine's workspace (`cargo run -p
    // sandbox`) lies next to the library the scripts are built into, which
    // holds whichever project's scripts were built last: not packaged.
    if is_engine_target(dir) {
        return None;
    }
    let path = dir.join(format!(
        "{}{SCRIPTS_PACKAGE}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    path.is_file().then_some(path)
}

/// Whether `dir` is where the engine's workspace builds to (its `target`
/// folder, any profile or target triple below it).
fn is_engine_target(dir: &Path) -> bool {
    let target = engine_workspace().join("target");
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    dir.starts_with(&target)
}

/// The engine's cargo workspace, where it was built from.
pub fn engine_workspace() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    path.canonicalize().unwrap_or(path)
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// A project's scripts and the library they are compiled into.
#[derive(Clone)]
pub struct ScriptCrate {
    /// The project's `scripts` folder.
    sources: PathBuf,
}

impl ScriptCrate {
    pub fn new(project_dir: &Path) -> Self {
        let sources = project_dir.join("scripts");
        Self {
            sources: sources.canonicalize().unwrap_or(sources),
        }
    }

    pub fn sources(&self) -> &Path {
        &self.sources
    }

    /// Where the built library is.
    pub fn library_path(&self) -> PathBuf {
        engine_workspace().join("target").join(profile()).join(format!(
            "lib{SCRIPTS_PACKAGE}{}",
            std::env::consts::DLL_SUFFIX
        ))
    }

    /// Compile the scripts. Returns the first compiler error.
    pub fn build(&self) -> Result<(), String> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        clean_cargo_env(&mut command);
        command
            .current_dir(engine_workspace())
            .args(["build", "-p", SCRIPTS_PACKAGE, "--message-format=short"])
            .env("FERRITE_SCRIPTS", &self.sources);
        if !cfg!(debug_assertions) {
            command.arg("--release");
        }
        let output = command
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
        // Paths of the script files, shown relative to the project.
        let first = first.replace(&format!("{}/", self.sources.display()), "scripts/");
        Err(first.trim().to_string())
    }
}

/// Whether `name` is a variable cargo sets for a program it runs (`cargo
/// run`), as opposed to one that configures cargo itself.
pub fn is_inherited_cargo_var(name: &str) -> bool {
    const EXACT: [&str; 9] = [
        "CARGO_MANIFEST_DIR",
        "CARGO_MANIFEST_PATH",
        "CARGO_MANIFEST_LINKS",
        "CARGO_CRATE_NAME",
        "CARGO_BIN_NAME",
        "CARGO_PRIMARY_PACKAGE",
        "CARGO_TARGET_TMPDIR",
        "CARGO_RUSTC_CURRENT_DIR",
        "OUT_DIR",
    ];
    EXACT.contains(&name)
        || name.starts_with("CARGO_PKG_")
        || name.starts_with("CARGO_CFG_")
        || name.starts_with("CARGO_BIN_EXE_")
}

/// Drop the variables an editor started with `cargo run` inherits. Build
/// scripts of dependencies (ring's, for one) rerun when they change, which
/// rebuilds the engine library under the running editor.
pub fn clean_cargo_env(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(is_inherited_cargo_var) {
            command.env_remove(name);
        }
    }
}

/// Load the script library at `path` and return what it registers. It is
/// loaded from a copy, so the file can be rebuilt while in use, and stays
/// loaded for good: components made by it may outlive a reload.
///
/// The library must be built against the engine library this process runs
/// with; the caller makes sure of that.
pub fn load_library(path: &Path) -> Result<ScriptRegistry, String> {
    static LOADED: AtomicU32 = AtomicU32::new(0);
    let copy = std::env::temp_dir().join(format!(
        "ferrite-scripts-{}-{}{}",
        std::process::id(),
        LOADED.fetch_add(1, Ordering::Relaxed),
        std::env::consts::DLL_SUFFIX
    ));
    std::fs::copy(path, &copy).map_err(|e| format!("could not copy {}: {e}", path.display()))?;
    let result = unsafe {
        libloading::Library::new(&copy)
            .map_err(|e| format!("could not load the scripts: {e}"))
            .and_then(|library| {
                let mut registry = ScriptRegistry::default();
                {
                    let register: libloading::Symbol<fn(&mut ScriptRegistry)> = library
                        .get(REGISTER_SYMBOL)
                        .map_err(|e| format!("not a script library: {e}"))?;
                    register(&mut registry);
                }
                std::mem::forget(library);
                Ok(registry)
            })
    };
    // Mapped by now (or not loaded at all); the file itself is not needed.
    let _ = std::fs::remove_file(&copy);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_built_in_the_engine_workspace_is_not_a_packaged_game() {
        // `cargo run -p sandbox`: next to the library the scripts are
        // built into, which must be built for the project, not taken as is.
        let target = engine_workspace().join("target");
        assert!(is_engine_target(&target.join("debug")));
        assert!(is_engine_target(&target.join("x86_64-pc-windows-gnu/release")));
        assert!(!is_engine_target(&std::env::temp_dir().join("exported-game")));
    }

    #[test]
    fn variables_cargo_run_sets_are_dropped_but_cargo_config_is_kept() {
        for name in [
            "CARGO_MANIFEST_DIR",
            "CARGO_PKG_NAME",
            "CARGO_PKG_VERSION_MAJOR",
            "CARGO_CFG_TARGET_OS",
            "CARGO_BIN_EXE_ferrite-editor",
            "OUT_DIR",
        ] {
            assert!(is_inherited_cargo_var(name), "{name}");
        }
        for name in ["CARGO", "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "PATH"] {
            assert!(!is_inherited_cargo_var(name), "{name}");
        }
    }

    #[test]
    fn clean_cargo_env_removes_inherited_variables_from_the_command() {
        // Already set when run by `cargo test`; set it for any other runner.
        std::env::set_var("CARGO_PKG_NAME", "ferrite-engine");
        let mut command = Command::new("true");
        clean_cargo_env(&mut command);
        let removed = command
            .get_envs()
            .any(|(name, value)| name == "CARGO_PKG_NAME" && value.is_none());
        assert!(removed);
    }
}
