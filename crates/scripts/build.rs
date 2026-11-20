//! Writes `scripts.rs`: a module for every `.rs` file in the folder named by
//! FERRITE_SCRIPTS and for every folder in it with a `mod.rs` (whose own
//! modules it declares as usual), and `register`, which registers every
//! `#[derive(Script)]` struct and every `impl TerrainGenerator for ...` in
//! those top-level files. Without the variable there are none.
//!
//! A world (`TerrainGenerator`) is registered with the hash of its
//! scripts: its module and every top-level module it names as
//! `crate::<module>`, transitively. A change to any of those makes the world
//! anew; a change to other scripts (gameplay) keeps it (see
//! `GeneratorEntry::fingerprint`).

use std::{collections::BTreeMap, env, fs, path::PathBuf};

#[allow(dead_code)]
#[path = "src/scan.rs"]
mod scan;

fn main() {
    println!("cargo:rerun-if-env-changed=FERRITE_SCRIPTS");
    let mut modules = String::new();
    let mut registrations = String::new();
    // Scenes save a script by its struct's name: the module of each name.
    let mut script_modules: BTreeMap<String, String> = BTreeMap::new();
    // Every top-level module's path, and the worlds to register with
    // their module's hash (once all modules are known).
    let mut module_paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut worlds: Vec<(String, String)> = Vec::new();
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
                    "cargo:warning={} is not a valid module name (a Rust keyword or a name the \
                     scripts crate uses) and is left out; rename it",
                    file.display()
                );
                continue;
            };
            modules.push_str(&format!("#[path = {:?}]\npub mod {module};\n", file.to_string_lossy()));
            module_paths.insert(module.clone(), path.clone());
            let source = fs::read_to_string(&file).unwrap_or_default();
            for script in scan::script_structs(&source) {
                if let Some(first) = script_modules.get(&script) {
                    println!(
                        "cargo:warning=the script {script} in {} has the name of the script in \
                         {first} and is left out; scenes save scripts by name, so rename one",
                        file.display()
                    );
                    continue;
                }
                script_modules.insert(script.clone(), module.clone());
                registrations.push_str(&format!("    registry.register::<{module}::{script}>();\n"));
            }
            for world in scan::terrain_generators(&source) {
                worlds.push((module.clone(), world));
            }
        }
    }
    for (module, world) in &worlds {
        let hash = world_hash(module, &module_paths);
        registrations.push_str(&format!("    registry.register_terrain_hashed::<{module}::{world}>({hash:#x});\n"));
    }
    let code = format!(
        "{modules}\n\
         #[allow(unused_variables)]\n\
         pub fn register(registry: &mut ::ferrite::core::script::ScriptRegistry) {{\n\
         {registrations}}}\n"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("scripts.rs");
    if fs::read_to_string(&out).ok().as_deref() != Some(code.as_str()) {
        fs::write(out, code).unwrap();
    }
}

/// Every `.rs` file of a top-level module (a file, or a folder's files).
fn module_files(path: &std::path::Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(path) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for entry in entries {
        if entry.is_dir() {
            module_files(&entry, out);
        } else if entry.extension().is_some_and(|e| e == "rs") {
            out.push(entry);
        }
    }
}

/// FNV-1a over the files of `module` and of every module it names as
/// `crate::<name>`, transitively (paths and contents, in a fixed order).
fn world_hash(module: &str, modules: &BTreeMap<String, PathBuf>) -> u64 {
    let mut todo = vec![module.to_string()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(name) = todo.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(path) = modules.get(&name) else { continue };
        let mut files = Vec::new();
        module_files(path, &mut files);
        for file in files {
            let text = fs::read_to_string(&file).unwrap_or_default();
            for (i, _) in text.match_indices("crate::") {
                let rest = &text[i + "crate::".len()..];
                let used: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if modules.contains_key(&used) && !seen.contains(&used) {
                    todo.push(used);
                }
            }
        }
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for name in &seen {
        let mut files = Vec::new();
        module_files(&modules[name], &mut files);
        for file in files {
            // A change inside any of them makes the world anew.
            println!("cargo:rerun-if-changed={}", file.display());
            add(file.strip_prefix(&modules[name]).unwrap_or(&file).to_string_lossy().as_bytes());
            add(name.as_bytes());
            add(&fs::read(&file).unwrap_or_default());
        }
    }
    hash
}
