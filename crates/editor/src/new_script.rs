//! New script files, written from a template.

use std::path::Path;

/// Write a new script named `name` (`rotator`, `Rotator`, `my rotator` all
/// give a struct `Rotator`, file `rotator.rs`) into `folder` from a
/// template. Returns the file name.
pub(crate) fn new_script_file(folder: &Path, name: &str) -> Result<String, String> {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .flat_map(split_camel_case)
        .collect();
    if words.is_empty() || words[0].starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("'{name}' is not a usable name"));
    }
    let type_name: String = words
        .iter()
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + &chars.as_str().to_lowercase())
                .unwrap_or_default()
        })
        .collect();
    let file_name = format!(
        "{}.rs",
        words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_")
    );
    let path = folder.join(&file_name);
    if path.exists() {
        return Err(format!("scripts/{file_name} already exists"));
    }
    let source = format!(
        r#"use ferrite::prelude::*;

/// What this script does.
#[derive(Script)]
pub struct {type_name} {{
    /// Public fields are parameters: shown in the inspector, saved with the
    /// scene.
    pub speed: f32,
    /// Other fields are the script's own state.
    elapsed: f64,
}}

impl Default for {type_name} {{
    fn default() -> Self {{
        Self {{
            speed: 1.0,
            elapsed: 0.0,
        }}
    }}
}}

impl Behaviour for {type_name} {{
    fn update(&mut self, _scene: &mut Scene, _entity: &mut Entity, delta_time: f64) {{
        self.elapsed += delta_time * self.speed as f64;
    }}
}}
"#
    );
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    std::fs::write(&path, source).map_err(|e| e.to_string())?;
    Ok(file_name)
}

/// `MyRotator` → `My`, `Rotator`.
fn split_camel_case(word: &str) -> Vec<String> {
    let mut words = vec![String::new()];
    for c in word.chars() {
        if c.is_ascii_uppercase() && !words.last().unwrap().is_empty() {
            words.push(String::new());
        }
        words.last_mut().unwrap().push(c);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_script_is_named_after_what_the_user_typed() {
        let dir = std::env::temp_dir().join(format!("ferrite-new-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(new_script_file(&dir, "MyRotator").unwrap(), "my_rotator.rs");
        let text = std::fs::read_to_string(dir.join("my_rotator.rs")).unwrap();
        assert!(text.contains("pub struct MyRotator {"));
        assert_eq!(new_script_file(&dir, "door opener").unwrap(), "door_opener.rs");
        assert!(new_script_file(&dir, "my rotator").is_err(), "already exists");
        assert!(new_script_file(&dir, "!!").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
