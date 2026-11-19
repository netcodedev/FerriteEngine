//! Finding the modules and scripts of a scripts folder. Used by the build
//! script (which includes this file) and tested here.

use std::path::Path;

/// A file name as a module name: `player_controller.rs` →
/// `player_controller`; `None` if it cannot be one.
pub fn module_name(file: &Path) -> Option<String> {
    let stem = file.file_stem()?.to_str()?;
    let valid = stem
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (valid && !RESERVED.contains(&stem)).then(|| stem.to_string())
}

/// Names a module of the scripts crate cannot have: Rust's keywords (`pub
/// mod move;` does not compile), the crates the generated code names, and
/// the files the crate has itself.
const RESERVED: &[&str] = &[
    // The crate's own files and the generated `register`.
    "lib", "mod", "register", "scan",
    // Crates the scripts reach by name.
    "ferrite", "ferrite_dylib", "core", "std", "alloc", "crate", "self", "super", "Self",
    // Keywords, strict and reserved (2021 and 2024 editions).
    "as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "move", "mut",
    "pub", "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use",
    "where", "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv",
    "try", "typeof", "unsized", "virtual", "yield", "_",
];

/// A folder as a module name, if it is one: `landscape/` (holding a
/// `mod.rs`) → `landscape`.
pub fn folder_module_name(dir: &Path) -> Option<String> {
    if !dir.join("mod.rs").is_file() {
        return None;
    }
    let name = dir.file_name()?.to_str()?;
    module_name(Path::new(&format!("{name}.rs")))
}

/// Names of the types in `source` that implement `TerrainGenerator` (`impl
/// TerrainGenerator for Name`, the trait named by any path).
pub fn terrain_generators(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in source.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("impl ") else {
            continue;
        };
        let Some((trait_name, ty)) = rest.split_once(" for ") else {
            continue;
        };
        if trait_name.trim() != "TerrainGenerator" && !trait_name.trim().ends_with("::TerrainGenerator") {
            continue;
        }
        let name: String = ty.trim().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            found.push(name);
        }
    }
    found
}

/// Names of the structs in `source` that derive `Script`.
pub fn script_structs(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut deriving_script = false;
    for line in source.lines().map(str::trim) {
        if line.starts_with("#[derive(") {
            if line
                .trim_start_matches("#[derive(")
                .split(|c: char| c == ',' || c == ')' || c.is_whitespace())
                .any(|word| word == "Script" || word.ends_with("::Script"))
            {
                deriving_script = true;
            }
            continue;
        }
        if line.starts_with("#[") || line.starts_with("///") || line.is_empty() {
            continue;
        }
        if deriving_script {
            let mut words = line.split_whitespace().skip_while(|w| w.starts_with("pub"));
            if words.next() == Some("struct") {
                if let Some(name) = words.next() {
                    let name: String = name
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        found.push(name);
                    }
                }
            }
        }
        deriving_script = false;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_found_by_their_derive() {
        let source = r#"
            use ferrite::prelude::*;

            /// Turns things.
            #[derive(Script, Default)]
            pub struct Rotator {
                pub speed: f32,
            }

            #[derive(Default, ferrite::Script)]
            #[allow(dead_code)]
            struct Hidden;

            #[derive(Default)]
            pub struct NotAScript;
        "#;
        assert_eq!(script_structs(source), ["Rotator", "Hidden"]);
    }

    #[test]
    fn terrain_generators_are_found_by_their_impl() {
        let source = r#"
            impl TerrainGenerator for Landscape {
            impl ferrite::terrain::TerrainGenerator for Moon {
            impl TerrainSource for Landscape {
            // impl TerrainGenerator for Commented
        "#;
        assert_eq!(terrain_generators(source), ["Landscape", "Moon"]);
    }

    #[test]
    fn file_names_become_module_names() {
        assert_eq!(module_name(Path::new("a/tree_felling.rs")).as_deref(), Some("tree_felling"));
        assert_eq!(module_name(Path::new("a/2d-thing.rs")), None);
        assert_eq!(module_name(Path::new("a/lib.rs")), None);
        assert_eq!(module_name(Path::new("a/move.rs")), None);
        assert_eq!(module_name(Path::new("a/type.rs")), None);
        assert_eq!(module_name(Path::new("a/ferrite.rs")), None);
        assert_eq!(module_name(Path::new("a/std.rs")), None);
        assert_eq!(module_name(Path::new("a/movement.rs")).as_deref(), Some("movement"));
    }
}
