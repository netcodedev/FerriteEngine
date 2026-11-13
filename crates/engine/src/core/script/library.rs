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
