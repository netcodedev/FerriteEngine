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
    let target = target_dir();
    let target = target.canonicalize().unwrap_or(target);
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    dir.starts_with(&target)
}

/// Where cargo builds the engine's workspace to: CARGO_TARGET_DIR (kept
/// for the cargo the editor runs, see `clean_cargo_env`) or its `target`.
pub fn target_dir() -> PathBuf {
    target_dir_from(std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
}

fn target_dir_from(variable: Option<PathBuf>) -> PathBuf {
    match variable.filter(|dir| !dir.as_os_str().is_empty()) {
        // Relative to where cargo runs: the workspace.
        Some(dir) => engine_workspace().join(dir),
        None => engine_workspace().join("target"),
    }
}

/// Where cargo puts what it builds of the engine's workspace in this
/// profile (`target/debug`, say).
pub fn build_dir() -> PathBuf {
    let triple = std::env::var("CARGO_BUILD_TARGET").ok();
    profile_dir(target_dir(), triple.as_deref())
}

/// Where cargo puts what it builds for this profile: below the target
/// triple's own folder when CARGO_BUILD_TARGET names one.
fn profile_dir(target_dir: PathBuf, triple: Option<&str>) -> PathBuf {
    match triple.filter(|triple| !triple.is_empty()) {
        Some(triple) => target_dir.join(triple).join(profile()),
        None => target_dir.join(profile()),
    }
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
        build_dir().join(library_file_name())
    }

    /// Compile the scripts. Returns the first compiler error.
    pub fn build(&self) -> Result<(), String> {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        // Gently: the editor (and the desktop) stay responsive while it
        // builds, also when the engine is built anew with the scripts (all
        // cores and many gigabytes otherwise, and the system stalls).
        let mut command = match nice() {
            Some(nice) => {
                let mut command = Command::new(nice);
                command.args(["-n", "10"]).arg(cargo);
                command
            }
            None => Command::new(cargo),
        };
        clean_cargo_env(&mut command);
        command
            .current_dir(engine_workspace())
            .args(["build", "-p", SCRIPTS_PACKAGE, "--message-format=short", "-j", &build_jobs().to_string()])
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
        Err(first_error(&stderr, Some(&self.sources)))
    }
}

/// How many jobs a scripts build runs at once: `FERRITE_SCRIPT_JOBS`, or
/// half of the processor's threads (at least two).
fn build_jobs() -> usize {
    std::env::var("FERRITE_SCRIPT_JOBS")
        .ok()
        .and_then(|jobs| jobs.parse().ok())
        .filter(|&jobs: &usize| jobs > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).max(2)))
}

/// `nice`, to run the build at a lower priority, where there is one.
fn nice() -> Option<&'static str> {
    ["/usr/bin/nice", "/bin/nice"].into_iter().find(|path| cfg!(unix) && Path::new(path).exists())
}

/// The library's file name on this platform: `libferrite_scripts.so`,
/// `ferrite_scripts.dll`, ...
fn library_file_name() -> String {
    format!(
        "{}{SCRIPTS_PACKAGE}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// The first error in what `cargo build --message-format=short` wrote, not
/// a warning that mentions "error". Paths below `sources` (the scripts
/// folder) are shown as `scripts/...`.
pub(crate) fn first_error(stderr: &str, sources: Option<&Path>) -> String {
    let is_error = |line: &str| {
        let line = line.trim_start();
        // `error: ...`, or `path:line:col: error[E0425]: ...`.
        line.starts_with("error")
            || line
                .split_once(": ")
                .is_some_and(|(_, message)| message.starts_with("error"))
    };
    let first = stderr.lines().find(|line| is_error(line)).unwrap_or("the build failed");
    let mut first = first.trim().to_string();
    if let Some(sources) = sources {
        let shown = sources.display().to_string();
        // Windows' canonical paths start with `\\?\`, which cargo leaves out.
        let plain = shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string();
        for prefix in [&shown, &plain] {
            for separator in ['/', '\\'] {
                first = first.replace(&format!("{prefix}{separator}"), "scripts/");
            }
        }
    }
    first
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
    let loaded = LOADED.fetch_add(1, Ordering::Relaxed);
    if loaded == 0 {
        remove_stale_copies();
    }
    let copy = std::env::temp_dir().join(format!(
        "{COPY_PREFIX}{}-{loaded}{}",
        std::process::id(),
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
    // (Windows does not remove a loaded library: `remove_stale_copies`
    // does, when the next editor starts.)
    let _ = std::fs::remove_file(&copy);
    result
}

/// How the copies `load_library` loads are named in the temp folder.
const COPY_PREFIX: &str = "ferrite-scripts-";

/// Remove the copies earlier runs left in the temp folder. One still loaded
/// by a running process is not removed on Windows (it is locked) and stays
/// loaded elsewhere (its mapping does not need the file).
fn remove_stale_copies() {
    let own = format!("{COPY_PREFIX}{}-", std::process::id());
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(COPY_PREFIX)
            && !name.starts_with(&own)
            && name.ends_with(std::env::consts::DLL_SUFFIX)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
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
    fn the_library_is_found_where_cargo_builds_it() {
        let workspace = engine_workspace();
        assert_eq!(target_dir_from(None), workspace.join("target"));
        assert_eq!(target_dir_from(Some("".into())), workspace.join("target"));
        assert_eq!(target_dir_from(Some("out".into())), workspace.join("out"));
        let elsewhere = std::env::temp_dir().join("ferrite-target");
        assert_eq!(target_dir_from(Some(elsewhere.clone())), elsewhere);
        assert_eq!(
            profile_dir(elsewhere.clone(), Some("x86_64-pc-windows-gnu")),
            elsewhere.join("x86_64-pc-windows-gnu").join(profile())
        );
        assert_eq!(profile_dir(elsewhere.clone(), None), elsewhere.join(profile()));
        let file = library_file_name();
        assert!(file.starts_with(std::env::consts::DLL_PREFIX));
        assert!(file.ends_with(std::env::consts::DLL_SUFFIX));
        assert!(file.contains(SCRIPTS_PACKAGE));
    }

    #[test]
    fn the_first_error_is_reported_not_a_warning_that_mentions_one() {
        let stderr = "\
   Compiling ferrite_scripts v0.1.0
/p/scripts/x.rs:3:9: warning: unused variable: `error_count`
/p/scripts/y.rs:7:1: error[E0425]: cannot find value `z` in this scope
error: could not compile `ferrite_scripts` (lib) due to 1 previous error";
        assert_eq!(
            first_error(stderr, Some(Path::new("/p/scripts"))),
            "scripts/y.rs:7:1: error[E0425]: cannot find value `z` in this scope"
        );
        let windows = r"C:\p\scripts\y.rs:7:1: error[E0425]: oops";
        assert_eq!(
            first_error(windows, Some(Path::new(r"\\?\C:\p\scripts"))),
            "scripts/y.rs:7:1: error[E0425]: oops"
        );
        assert_eq!(first_error("warning: about errors", None), "the build failed");
        assert_eq!(
            first_error("error: linker `cc` not found", None),
            "error: linker `cc` not found"
        );
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
