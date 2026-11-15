use std::{path::{Path, PathBuf}, process::Command};

use serde_json::{json, Value};

use super::*;

pub(super) fn list_scripts(state: &State) -> Result<Value, String> {
    Ok(json!({"scripts": state.project()?.list_scripts()}))
}

/// `scripts/<name>.rs` of the project. The file name is the module name,
/// so it has to be snake_case.
fn script_path(state: &State, name: &str) -> Result<PathBuf, String> {
    let name = name.trim_end_matches(".rs");
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!(
            "'{name}' is not a script file name: lowercase letters, digits and _ (e.g. door_opener)"
        ));
    }
    Ok(state.project()?.dir().join("scripts").join(format!("{name}.rs")))
}

pub(super) fn read_script(state: &State, args: Value) -> Result<Value, String> {
    let path = script_path(state, &string_field(&args, "name")?)?;
    let source = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({"path": path.display().to_string(), "source": source}))
}

pub(super) fn write_script(state: &State, args: Value) -> Result<Value, String> {
    let path = script_path(state, &string_field(&args, "name")?)?;
    let source = string_field(&args, "source")?;
    let overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
    if path.exists() && !overwrite {
        return Err(format!("{} exists; pass overwrite: true to replace it", path.display()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, source).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({
        "written": path.display().to_string(),
        "next": "a running editor rebuilds on its own (editor_status → scripts_message); \
                 build_scripts shows all compiler errors",
    }))
}

/// Whether `name` is a variable `cargo run` sets for the program it runs.
/// Passed on to a nested cargo, some make dependencies rebuild (and with
/// them the engine library a running editor has loaded).
pub(super) fn is_inherited_cargo_var(name: &str) -> bool {
    matches!(
        name,
        "CARGO_MANIFEST_DIR"
            | "CARGO_MANIFEST_PATH"
            | "CARGO_MANIFEST_LINKS"
            | "CARGO_CRATE_NAME"
            | "CARGO_BIN_NAME"
            | "CARGO_PRIMARY_PACKAGE"
            | "CARGO_TARGET_TMPDIR"
            | "CARGO_RUSTC_CURRENT_DIR"
            | "OUT_DIR"
    ) || name.starts_with("CARGO_PKG_")
        || name.starts_with("CARGO_CFG_")
        || name.starts_with("CARGO_BIN_EXE_")
}

/// The diagnostics in cargo's short output, with script paths shown
/// relative to the project.
pub(super) fn diagnostics(stderr: &str, scripts_dir: &Path) -> (Vec<String>, Vec<String>) {
    let prefix = format!("{}/", scripts_dir.display());
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for line in stderr.lines() {
        let line = line.trim().replace(&prefix, "scripts/");
        // Cargo's summary lines repeat what the diagnostics said.
        if line.starts_with("error: could not compile") || line.starts_with("warning: `") {
            continue;
        }
        if line.contains("error") {
            errors.push(line);
        } else if line.contains("warning") && line.contains("scripts/") {
            warnings.push(line);
        }
    }
    (errors, warnings)
}

pub(super) fn build_scripts(state: &State) -> Result<Value, String> {
    let scripts = state.project()?.dir().join("scripts");
    let scripts = scripts.canonicalize().unwrap_or(scripts);
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(is_inherited_cargo_var) {
            command.env_remove(name);
        }
    }
    let output = command
        .current_dir(engine_workspace())
        .args(["build", "-p", "ferrite_scripts", "--message-format=short"])
        .env("FERRITE_SCRIPTS", &scripts)
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let (errors, warnings) = diagnostics(&stderr, &scripts);
    if output.status.success() {
        Ok(json!({"ok": true, "warnings": warnings}))
    } else {
        Err(serde_json::to_string_pretty(&json!({
            "ok": false,
            "errors": if errors.is_empty() { vec![stderr.trim().to_string()] } else { errors },
            "warnings": warnings,
        }))
        .unwrap_or_default())
    }
}
