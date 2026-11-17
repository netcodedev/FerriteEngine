//! Writes `scripts.rs`: a module for every `.rs` file in the folder named by
//! FERRITE_SCRIPTS and for every folder in it with a `mod.rs` (whose own
//! modules it declares as usual), and `register`, which registers every
//! `#[derive(Script)]` struct and every `impl TerrainGenerator for ...` in
//! those top-level files. Without the variable there are none.

use std::{env, fs, path::PathBuf};

#[allow(dead_code)]
#[path = "src/scan.rs"]
mod scan;

fn main() {
    println!("cargo:rerun-if-env-changed=FERRITE_SCRIPTS");
    let mut modules = String::new();
    let mut registrations = String::new();
    if let Some(dir) = env::var_os("FERRITE_SCRIPTS").map(PathBuf::from) {
        // Files added or removed change the folder.
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        (p.is_file() && p.extension().is_some_and(|e| e == "rs"))
                            || (p.is_dir() && p.join("mod.rs").is_file())
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for path in files {
            let (module, file) = if path.is_dir() {
                (scan::folder_module_name(&path), path.join("mod.rs"))
            } else {
                (scan::module_name(&path), path.clone())
            };
            // A changed file may define other scripts.
            println!("cargo:rerun-if-changed={}", file.display());
            let Some(module) = module else {
                println!(
                    "cargo:warning={} is not a valid module name and is left out",
                    file.display()
                );
                continue;
            };
            modules.push_str(&format!("#[path = {:?}]\npub mod {module};\n", file.to_string_lossy()));
            let source = fs::read_to_string(&file).unwrap_or_default();
            for script in scan::script_structs(&source) {
                registrations.push_str(&format!("    registry.register::<{module}::{script}>();\n"));
            }
            for world in scan::terrain_generators(&source) {
                registrations.push_str(&format!("    registry.register_terrain::<{module}::{world}>();\n"));
            }
        }
    }
    let code = format!(
        "{modules}\n\
         #[allow(unused_variables)]\n\
         pub fn register(registry: &mut ferrite::core::script::ScriptRegistry) {{\n\
         {registrations}}}\n"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("scripts.rs");
    if fs::read_to_string(&out).ok().as_deref() != Some(code.as_str()) {
        fs::write(out, code).unwrap();
    }
}
