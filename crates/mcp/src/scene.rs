//! The scene file format, mirrored only as far as the MCP server needs.
//!
//! The engine's scene file is RON with enum variants for components —
//! `SkyLight(time_of_day:0.5,...)`. `ron::Value` round-trips lose those
//! variant names, so we keep each component as its raw RON text slice
//! and reassemble the file ourselves on save.
//!
//! The structural fields (version, settings, per-entity id/name/transform
//! and the tree of children) are parsed into typed values; everything
//! else travels through as text. An AI can list entities, move them
//! around, add or remove them and add components (by supplying their RON
//! text) without the server knowing every variant the engine defines.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub const SCENE_FILE_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct SceneFile {
    pub version: u32,
    /// Raw RON text of the top-level `settings` field, or empty for the
    /// default. Round-trips unchanged.
    pub settings: String,
    pub entities: Vec<EntityData>,
}

#[derive(Clone, Debug)]
pub struct EntityData {
    pub id: u64,
    pub name: String,
    pub position: [f32; 3],
    /// Quaternion, `[w, x, y, z]`.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    /// Each component is raw RON text (e.g. `SkyLight(time_of_day:0.5)`).
    pub components: Vec<String>,
    pub children: Vec<EntityData>,
}

impl SceneFile {
    pub fn empty() -> Self {
        SceneFile {
            version: SCENE_FILE_VERSION,
            settings: String::new(),
            entities: Vec::new(),
        }
    }

    pub fn from_ron(text: &str) -> Result<Self, String> {
        parse_scene(text)
    }

    pub fn to_ron(&self) -> Result<String, String> {
        Ok(serialize_scene(self))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_ron(&text)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let text = self.to_ron()?;
        fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Walk every entity in the scene, depth first.
    pub fn walk(&self) -> impl Iterator<Item = &EntityData> {
        let mut queue: Vec<&EntityData> = self.entities.iter().collect();
        std::iter::from_fn(move || {
            let next = queue.pop()?;
            for c in next.children.iter().rev() {
                queue.push(c);
            }
            Some(next)
        })
    }

    pub fn find(&self, id: u64) -> Option<&EntityData> {
        self.walk().find(|e| e.id == id)
    }

    /// Mutate the entity with this id. Returns `true` when it was found.
    pub fn with_entity_mut<F: FnOnce(&mut EntityData)>(&mut self, id: u64, edit: F) -> bool {
        fn visit<F: FnOnce(&mut EntityData)>(
            list: &mut [EntityData],
            id: u64,
            mut edit: Option<F>,
        ) -> Option<F> {
            for e in list.iter_mut() {
                if e.id == id {
                    if let Some(f) = edit.take() {
                        f(e);
                    }
                    return None;
                }
                edit = visit(&mut e.children, id, edit);
                if edit.is_none() {
                    return None;
                }
            }
            edit
        }
        visit(&mut self.entities, id, Some(edit)).is_none()
    }

    /// Remove an entity (and its subtree) from the scene.
    pub fn remove(&mut self, id: u64) -> Option<EntityData> {
        fn visit(list: &mut Vec<EntityData>, id: u64) -> Option<EntityData> {
            if let Some(index) = list.iter().position(|e| e.id == id) {
                return Some(list.remove(index));
            }
            for e in list.iter_mut() {
                if let Some(removed) = visit(&mut e.children, id) {
                    return Some(removed);
                }
            }
            None
        }
        visit(&mut self.entities, id)
    }

    /// Add `entity` at root (parent_id: None) or under an existing entity.
    pub fn add(&mut self, entity: EntityData, parent_id: Option<u64>) -> Result<(), String> {
        match parent_id {
            None => {
                self.entities.push(entity);
                Ok(())
            }
            Some(id) => {
                let mut once = Some(entity);
                let found = self.with_entity_mut(id, |e| {
                    if let Some(child) = once.take() {
                        e.children.push(child);
                    }
                });
                if found {
                    Ok(())
                } else {
                    Err(format!("no entity with id {id}"))
                }
            }
        }
    }
}

/// Short label for a component (variant name, or `Script <name>` for a
/// script component).
pub fn component_label(raw: &str) -> String {
    let trimmed = raw.trim_start();
    let name: String = trimmed
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name == "Script" {
        let after = &trimmed[name.len()..];
        if let Some(inner) = extract_field(after, "name") {
            return format!("Script {}", inner.trim().trim_matches('"'));
        }
    }
    if name.is_empty() {
        "Component".into()
    } else {
        name
    }
}

/// Full path to `scenes/<rel>` under `project_dir`, rejecting traversal
/// and absolute paths.
pub fn scene_path(project_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_start_matches("./");
    let rel = rel.strip_prefix("scenes/").unwrap_or(rel);
    let candidate = PathBuf::from(rel);
    if candidate.is_absolute() {
        return Err("scene path must be relative to scenes/".into());
    }
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("scene path may not contain '..'".into());
    }
    Ok(project_dir.join("scenes").join(candidate))
}

/// A fresh, non-zero id; a time-salted counter.
pub fn fresh_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let salt = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    salt.wrapping_mul(2654435761)
        .wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed))
}

// -------------------------------------------------------------------------
// RON parser
//
// The scene file is a RON struct at the top level. We only need a handful
// of fields; the rest we treat as opaque text (raw slices) that
// round-trip unchanged. The parser is bracket-balanced and
// string-literal aware; it does NOT try to understand RON semantics.
// -------------------------------------------------------------------------

/// Top-level entry point: parse one scene file.
fn parse_scene(text: &str) -> Result<SceneFile, String> {
    let (body, _) = take_struct_body(text, 0)?;
    let fields = read_fields(text, body)?;

    let mut scene = SceneFile::empty();
    for (name, value) in fields {
        match name {
            "version" => {
                scene.version = text[value.clone()]
                    .trim()
                    .parse()
                    .map_err(|e| format!("version: {e}"))?;
            }
            "settings" => {
                scene.settings = text[value].trim().to_string();
            }
            "entities" => {
                let items = read_list(text, value)?;
                for span in items {
                    scene.entities.push(parse_entity(text, span)?);
                }
            }
            _ => {
                // Unknown fields are silently preserved only if the
                // scene file has them; for now we ignore and reconstruct
                // the known layout on save.
            }
        }
    }
    Ok(scene)
}

type Range = std::ops::Range<usize>;

fn parse_entity(text: &str, span: Range) -> Result<EntityData, String> {
    let (body, _) = take_struct_body(text, span.start)?;
    let fields = read_fields(text, body)?;

    let mut entity = EntityData {
        id: 0,
        name: String::new(),
        position: [0.0; 3],
        rotation: [1.0, 0.0, 0.0, 0.0],
        scale: [1.0, 1.0, 1.0],
        components: Vec::new(),
        children: Vec::new(),
    };

    for (name, value) in fields {
        let slice = text[value.clone()].trim();
        match name {
            "id" => {
                entity.id = slice.parse().map_err(|e| format!("id: {e}"))?;
            }
            "name" => {
                entity.name = parse_string(slice)?;
            }
            "position" => entity.position = parse_f32_array::<3>(slice)?,
            "rotation" => entity.rotation = parse_f32_array::<4>(slice)?,
            "scale" => entity.scale = parse_f32_array::<3>(slice)?,
            "components" => {
                let items = read_list(text, value)?;
                entity.components = items
                    .into_iter()
                    .map(|r| text[r].trim().to_string())
                    .collect();
            }
            "children" => {
                let items = read_list(text, value)?;
                for span in items {
                    entity.children.push(parse_entity(text, span)?);
                }
            }
            _ => {}
        }
    }
    Ok(entity)
}

/// Skip whitespace and the next `(`, return the range of its body
/// (excluding the surrounding parens) and the position right after `)`.
fn take_struct_body(text: &str, from: usize) -> Result<(Range, usize), String> {
    let start = skip_ws(text, from);
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'(') {
        return Err(format!("expected '(' at offset {start}"));
    }
    let close = find_match(text, start, b'(', b')')?;
    Ok(((start + 1)..close, close + 1))
}

/// Within a struct body, read `name: value, name: value, ...` and return
/// the `(name, value_range)` pairs.
fn read_fields(text: &str, body: Range) -> Result<Vec<(&str, Range)>, String> {
    let mut out = Vec::new();
    let mut i = body.start;
    let end = body.end;
    while i < end {
        i = skip_ws(text, i);
        if i >= end || text.as_bytes()[i] == b',' {
            i = i.saturating_add(1);
            continue;
        }
        // Identifier.
        let name_start = i;
        while i < end {
            let c = text.as_bytes()[i];
            if c.is_ascii_alphanumeric() || c == b'_' {
                i += 1;
            } else {
                break;
            }
        }
        if name_start == i {
            break;
        }
        let name = &text[name_start..i];
        i = skip_ws(text, i);
        if text.as_bytes().get(i) != Some(&b':') {
            return Err(format!("expected ':' after field {name}"));
        }
        i += 1;
        i = skip_ws(text, i);
        let value_start = i;
        let value_end = scan_value(text, i, end)?;
        out.push((name, value_start..value_end));
        i = value_end;
        i = skip_ws(text, i);
        if i < end && text.as_bytes()[i] == b',' {
            i += 1;
        }
    }
    Ok(out)
}

/// Within a bracket body `[a, b, c]`, return the ranges of the items.
fn read_list(text: &str, value: Range) -> Result<Vec<Range>, String> {
    let bytes = text.as_bytes();
    let start = skip_ws(text, value.start);
    if bytes.get(start) != Some(&b'[') {
        return Err(format!("expected '[' at offset {start}"));
    }
    let close = find_match(text, start, b'[', b']')?;
    let mut items = Vec::new();
    let mut i = start + 1;
    while i < close {
        i = skip_ws(text, i);
        if i >= close || bytes[i] == b',' {
            i += 1;
            continue;
        }
        let item_start = i;
        let item_end = scan_value(text, i, close)?;
        items.push(item_start..item_end);
        i = item_end;
        i = skip_ws(text, i);
        if i < close && bytes[i] == b',' {
            i += 1;
        }
    }
    Ok(items)
}

/// Scan from `i`, returning the position right after the next complete
/// value (balanced brackets, quoted strings).
fn scan_value(text: &str, from: usize, end: usize) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut i = skip_ws(text, from);
    let start = i;
    let mut depth = 0i32;
    while i < end {
        match bytes[i] {
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                if depth == 0 {
                    return Ok(i);
                }
                depth -= 1;
                i += 1;
            }
            b'"' => {
                i = skip_string(text, i)?;
            }
            b'\'' => {
                i = skip_char_literal(text, i)?;
            }
            b',' if depth == 0 => return Ok(i),
            _ => i += 1,
        }
    }
    if start == i {
        return Err("empty value".into());
    }
    Ok(i)
}

fn find_match(text: &str, open_at: usize, open: u8, close: u8) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open_at;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => i = skip_string(text, i)?,
            b'\'' => i = skip_char_literal(text, i)?,
            c if c == open => {
                depth += 1;
                i += 1;
            }
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    Err(format!("unbalanced {}", open as char))
}

fn skip_string(text: &str, from: usize) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut i = from + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err("unterminated string".into())
}

fn skip_char_literal(text: &str, from: usize) -> Result<usize, String> {
    let bytes = text.as_bytes();
    let mut i = from + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\'' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err("unterminated char literal".into())
}

fn skip_ws(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            _ => break,
        }
    }
    i
}

fn parse_string(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if !(raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2) {
        return Err(format!("expected quoted string, got {raw}"));
    }
    let mut out = String::new();
    let inner = &raw[1..raw.len() - 1];
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => out.push(other),
                None => return Err("bad escape".into()),
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn parse_f32_array<const N: usize>(raw: &str) -> Result<[f32; N], String> {
    let raw = raw.trim();
    if !(raw.starts_with('(') && raw.ends_with(')')) {
        return Err(format!("expected tuple, got {raw}"));
    }
    let inner = &raw[1..raw.len() - 1];
    let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
    if parts.len() != N {
        return Err(format!("expected {N} numbers, got {}", parts.len()));
    }
    let mut out = [0.0_f32; N];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse().map_err(|e| format!("number {i}: {e}"))?;
    }
    Ok(out)
}

/// Pull the value text of `field: <value>` from a struct body at the
/// start of `text`.
fn extract_field<'a>(text: &'a str, field: &str) -> Option<&'a str> {
    let (body, _) = take_struct_body(text, 0).ok()?;
    let fields = read_fields(text, body).ok()?;
    fields
        .into_iter()
        .find(|(n, _)| *n == field)
        .map(|(_, r)| &text[r])
}

// -------------------------------------------------------------------------
// RON writer
// -------------------------------------------------------------------------

fn serialize_scene(scene: &SceneFile) -> String {
    let mut out = String::new();
    out.push_str("(\n");
    out.push_str(&format!("    version: {},\n", scene.version));
    if !scene.settings.is_empty() {
        out.push_str(&format!("    settings: {},\n", scene.settings));
    }
    out.push_str("    entities: [");
    if scene.entities.is_empty() {
        out.push_str("],\n");
    } else {
        out.push('\n');
        for (i, e) in scene.entities.iter().enumerate() {
            write_entity(&mut out, e, 2);
            if i + 1 < scene.entities.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("    ],\n");
    }
    out.push_str(")\n");
    out
}

fn write_entity(out: &mut String, e: &EntityData, indent: usize) {
    let pad = "    ".repeat(indent);
    let inner_pad = "    ".repeat(indent + 1);
    out.push_str(&pad);
    out.push_str("(\n");
    out.push_str(&format!("{inner_pad}id: {},\n", e.id));
    out.push_str(&format!("{inner_pad}name: {},\n", quote(&e.name)));
    out.push_str(&format!(
        "{inner_pad}position: ({}, {}, {}),\n",
        f(e.position[0]),
        f(e.position[1]),
        f(e.position[2])
    ));
    out.push_str(&format!(
        "{inner_pad}rotation: ({}, {}, {}, {}),\n",
        f(e.rotation[0]),
        f(e.rotation[1]),
        f(e.rotation[2]),
        f(e.rotation[3])
    ));
    if e.scale != [1.0, 1.0, 1.0] {
        out.push_str(&format!(
            "{inner_pad}scale: ({}, {}, {}),\n",
            f(e.scale[0]),
            f(e.scale[1]),
            f(e.scale[2])
        ));
    }
    out.push_str(&format!("{inner_pad}components: ["));
    if e.components.is_empty() {
        out.push_str("],\n");
    } else {
        out.push('\n');
        for (i, c) in e.components.iter().enumerate() {
            out.push_str(&"    ".repeat(indent + 2));
            out.push_str(c);
            if i + 1 < e.components.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str(&inner_pad);
        out.push_str("],\n");
    }
    out.push_str(&format!("{inner_pad}children: ["));
    if e.children.is_empty() {
        out.push_str("],\n");
    } else {
        out.push('\n');
        for (i, child) in e.children.iter().enumerate() {
            write_entity(out, child, indent + 2);
            if i + 1 < e.children.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str(&inner_pad);
        out.push_str("],\n");
    }
    out.push_str(&pad);
    out.push(')');
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn f(n: f32) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{n:.1}")
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: u64, name: &str) -> EntityData {
        EntityData {
            id,
            name: name.into(),
            position: [0.0, 0.0, 0.0],
            rotation: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            components: Vec::new(),
            children: Vec::new(),
        }
    }

    fn with_children(id: u64, name: &str, children: Vec<EntityData>) -> EntityData {
        EntityData {
            children,
            ..leaf(id, name)
        }
    }

    #[test]
    fn an_empty_scene_roundtrips() {
        let scene = SceneFile::empty();
        let text = scene.to_ron().unwrap();
        let again = SceneFile::from_ron(&text).unwrap();
        assert_eq!(again.version, SCENE_FILE_VERSION);
        assert!(again.entities.is_empty());
    }

    #[test]
    fn walk_visits_every_entity_in_the_tree() {
        let scene = SceneFile {
            version: 1,
            settings: String::new(),
            entities: vec![
                with_children(1, "Root", vec![leaf(2, "A"), leaf(3, "B")]),
                leaf(4, "Loose"),
            ],
        };
        let names: Vec<&str> = scene.walk().map(|e| e.name.as_str()).collect();
        assert_eq!(names.len(), 4);
        assert!(names.contains(&"Root"));
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
        assert!(names.contains(&"Loose"));
    }

    #[test]
    fn find_and_mutate_work_on_nested_entities() {
        let mut scene = SceneFile {
            version: 1,
            settings: String::new(),
            entities: vec![with_children(1, "Root", vec![leaf(42, "Deep")])],
        };
        assert_eq!(scene.find(42).unwrap().name, "Deep");
        assert!(scene.with_entity_mut(42, |e| e.position = [1.0, 2.0, 3.0]));
        assert_eq!(scene.find(42).unwrap().position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn remove_takes_the_entity_out_of_the_tree() {
        let mut scene = SceneFile {
            version: 1,
            settings: String::new(),
            entities: vec![with_children(
                1,
                "Root",
                vec![leaf(2, "A"), leaf(3, "B")],
            )],
        };
        let removed = scene.remove(2).unwrap();
        assert_eq!(removed.name, "A");
        assert!(scene.find(2).is_none());
        assert!(scene.find(3).is_some());
    }

    #[test]
    fn add_under_a_parent_nests_the_entity() {
        let mut scene = SceneFile::empty();
        scene.add(leaf(1, "Parent"), None).unwrap();
        scene.add(leaf(2, "Child"), Some(1)).unwrap();
        assert_eq!(scene.entities[0].children.len(), 1);
    }

    #[test]
    fn add_under_an_unknown_parent_fails() {
        let mut scene = SceneFile::empty();
        assert!(scene.add(leaf(1, "A"), Some(42)).is_err());
    }

    #[test]
    fn components_roundtrip_as_raw_ron_preserving_variants() {
        let mut scene = SceneFile::empty();
        scene
            .add(
                EntityData {
                    components: vec![
                        "SkyLight(time_of_day: 0.5, day_length_seconds: 60.0, paused: false, cloud_coverage: 0.3)"
                            .to_string(),
                        "DebugController".to_string(),
                    ],
                    ..leaf(10, "Sun")
                },
                None,
            )
            .unwrap();

        let text = scene.to_ron().unwrap();
        let back = SceneFile::from_ron(&text).unwrap();
        assert_eq!(back.entities[0].components.len(), 2);
        assert!(back.entities[0].components[0].starts_with("SkyLight("));
        assert_eq!(back.entities[0].components[1], "DebugController");
    }

    #[test]
    fn scene_path_rejects_traversal_and_absolute_paths() {
        let root = Path::new("/tmp/project");
        assert!(scene_path(root, "../../etc/passwd").is_err());
        assert!(scene_path(root, "/etc/passwd").is_err());
        let ok = scene_path(root, "scenes/level1.ron").unwrap();
        assert!(ok.ends_with("scenes/level1.ron"));
    }

    #[test]
    fn fresh_id_never_returns_zero_and_counts_up() {
        let a = fresh_id();
        let b = fresh_id();
        assert_ne!(a, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn component_label_pulls_out_the_variant_name() {
        assert_eq!(component_label("SkyLight(time_of_day:0.5)"), "SkyLight");
        assert_eq!(component_label("DebugController"), "DebugController");
        assert_eq!(
            component_label("Script(name: \"Hand\", data: ())"),
            "Script Hand"
        );
    }

    #[test]
    fn parsing_handles_nested_strings_and_brackets() {
        let text = r#"(
            version: 1,
            entities: [
                (
                    id: 7,
                    name: "has, commas and \" quotes",
                    position: (0.0, 0.0, 0.0),
                    rotation: (1.0, 0.0, 0.0, 0.0),
                    components: [
                        Script(name: "complex", data: ("a", "b, c")),
                    ],
                    children: [],
                ),
            ],
        )"#;
        let scene = SceneFile::from_ron(text).unwrap();
        assert_eq!(scene.entities.len(), 1);
        assert_eq!(
            scene.entities[0].name,
            r#"has, commas and " quotes"#
        );
        assert_eq!(scene.entities[0].components.len(), 1);
        assert!(scene.entities[0].components[0].contains("complex"));
    }

    #[test]
    fn save_and_load_roundtrip_to_disk() {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-mcp-scene-{}-{}",
            std::process::id(),
            fresh_id(),
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.ron");

        let mut scene = SceneFile::empty();
        scene
            .add(
                EntityData {
                    position: [1.0, 2.0, 3.0],
                    components: vec!["DebugController".to_string()],
                    ..leaf(1, "Main")
                },
                None,
            )
            .unwrap();
        scene.save(&path).unwrap();

        let loaded = SceneFile::load(&path).unwrap();
        assert_eq!(loaded.entities.len(), 1);
        assert_eq!(loaded.entities[0].name, "Main");
        assert_eq!(loaded.entities[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(loaded.entities[0].components, vec!["DebugController"]);

        let _ = fs::remove_dir_all(&dir);
    }
}
