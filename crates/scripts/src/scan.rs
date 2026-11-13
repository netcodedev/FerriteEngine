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
    (valid && stem != "lib" && stem != "mod" && stem != "register").then(|| stem.to_string())
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
    fn file_names_become_module_names() {
        assert_eq!(module_name(Path::new("a/tree_felling.rs")).as_deref(), Some("tree_felling"));
        assert_eq!(module_name(Path::new("a/2d-thing.rs")), None);
        assert_eq!(module_name(Path::new("a/lib.rs")), None);
    }
}
