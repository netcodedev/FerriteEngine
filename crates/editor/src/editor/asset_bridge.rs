//! The MCP bridge's asset management requests: what refers to an asset,
//! deleting, renaming and making assets and folders. Deleting asks for
//! `confirm: true`; without it the answer only says what would go.

use std::path::{Path, PathBuf};

use ferrite::core::assets::{self, AssetDatabase, AssetId, NewAsset, Reference};
use serde_json::{json, Value};

use super::{BrowserRequest, Command, EditorState};

/// An asset by id, source path below the asset folder, or name. A name
/// two assets share is refused, listing both.
fn resolve(database: &AssetDatabase, asset: &str) -> Result<AssetId, String> {
    if let Some(id) = asset.parse::<AssetId>().ok().filter(|id| database.get(*id).is_some()) {
        return Ok(id);
    }
    let relative = asset.trim_start_matches("assets/");
    if let Some(id) = database.find_by_source(&database.root().join(relative)) {
        return Ok(id);
    }
    let named: Vec<_> = database.iter().filter(|e| e.meta.name == asset).collect();
    match named.as_slice() {
        [one] => Ok(one.meta.id),
        [] => Err(format!("no asset '{asset}' (give its id, name or path below assets/)")),
        many => Err(format!(
            "'{asset}' names {} assets; give one of their ids: {}",
            many.len(),
            many.iter()
                .map(|e| format!("{} ({}, {})", e.meta.id, e.meta.kind.label(), relative_path(database, &e.source_path())))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn relative_path(database: &AssetDatabase, path: &Path) -> String {
    let project = database.project_root();
    path.strip_prefix(&project).unwrap_or(path).to_string_lossy().replace('\\', "/").trim_start_matches("./").to_string()
}

fn references_json(database: &AssetDatabase, references: &[Reference]) -> Value {
    references
        .iter()
        .map(|r| match r.asset.and_then(|id| database.get(id)) {
            Some(entry) => json!({
                "file": relative_path(database, &r.file),
                "asset": entry.meta.id.to_string(),
                "name": entry.meta.name,
                "kind": entry.meta.kind.label(),
            }),
            None => json!({"file": relative_path(database, &r.file)}),
        })
        .collect()
}

fn asset_json(database: &AssetDatabase, id: AssetId) -> Value {
    match database.get(id) {
        Some(entry) => json!({
            "asset": id.to_string(),
            "name": entry.meta.name,
            "kind": entry.meta.kind.label(),
            "path": relative_path(database, &entry.source_path()),
        }),
        None => json!({"asset": id.to_string()}),
    }
}

pub fn references(asset: &str) -> Result<Value, String> {
    let database = assets::database();
    let id = resolve(&database, asset)?;
    let references = database.references_to(id);
    let mut answer = asset_json(&database, id);
    answer["references"] = references_json(&database, &references);
    Ok(answer)
}

/// The name of an editor holding unsaved changes to one of `ids`.
fn unsaved_editor(state: &EditorState, ids: &[AssetId]) -> Option<String> {
    state
        .asset_edit
        .as_ref()
        .filter(|e| ids.contains(&e.meta.id) && e.dirty)
        .map(|e| e.meta.name.clone())
        .or_else(|| state.material.as_ref().filter(|e| ids.contains(&e.asset) && e.dirty()).map(|e| e.name.clone()))
        .or_else(|| state.scatter.as_ref().filter(|e| ids.contains(&e.asset) && e.dirty()).map(|e| e.name.clone()))
}

pub fn delete(state: &mut EditorState, asset: &str, confirm: bool) -> Result<Value, String> {
    let (id, mut answer) = {
        let database = assets::database();
        let id = resolve(&database, asset)?;
        let references = database.references_to(id);
        let mut answer = asset_json(&database, id);
        answer["references"] = references_json(&database, &references);
        (id, answer)
    };
    if !confirm {
        answer["deleted"] = json!(false);
        answer["note"] = json!("Nothing was deleted. Call again with confirm: true to move the file and its sidecar to the trash; the files listed under references will miss it.");
        return Ok(answer);
    }
    if let Some(name) = unsaved_editor(state, &[id]) {
        return Err(format!("'{name}' has unsaved changes in the editor: save or revert them first"));
    }
    // Run as the browser's command, which also closes editors open on it.
    state.push(Command::DeleteAsset(id));
    answer["deleted"] = json!(true);
    answer["note"] = json!("Moved to the trash on the editor's next frame; editor_status → last status says how it went.");
    Ok(answer)
}

pub fn rename(state: &mut EditorState, asset: &str, name: &str) -> Result<Value, String> {
    let id = resolve(&assets::database(), asset)?;
    if let Some(open) = unsaved_editor(state, &[id]) {
        return Err(format!("'{open}' has unsaved changes in the editor: save or revert them first"));
    }
    state.push(Command::RenameAsset { id, name: name.to_string() });
    Ok(json!({"asset": id.to_string(), "renaming_to": name, "note": "Renamed on the editor's next frame; editor_status → last status says how it went."}))
}

/// A folder below the asset root, from a path given relative to it.
fn folder_path(database: &AssetDatabase, path: &str) -> Result<PathBuf, String> {
    let relative = path.trim().trim_start_matches("assets/").trim_matches('/');
    if relative.split('/').any(|part| part == ".." || part == ".") {
        return Err(format!("'{path}' leaves the asset folder"));
    }
    Ok(if relative.is_empty() { database.root().to_path_buf() } else { database.root().join(relative) })
}

pub fn create(state: &mut EditorState, kind: &str, folder: Option<&str>, name: Option<&str>) -> Result<Value, String> {
    let kind = NewAsset::from_key(kind).ok_or_else(|| {
        format!("no kind '{kind}'; one of {}", NewAsset::ALL.map(|k| k.key()).join(", "))
    })?;
    let mut database = assets::database();
    let folder = folder.map(|f| folder_path(&database, f)).transpose()?;
    let id = assets::create::create(&mut database, kind, folder.as_deref(), name)?;
    state.asset_reveal = Some(id);
    Ok(json!({"created": id.to_string(), "name": database.get(id).map(|e| e.meta.name.clone()), "path": database.get(id).map(|e| relative_path(&database, &e.source_path()))}))
}

pub fn create_folder(state: &mut EditorState, path: &str) -> Result<Value, String> {
    let mut database = assets::database();
    let target = folder_path(&database, path)?;
    let (parent, name) = match (target.parent(), target.file_name()) {
        (Some(parent), Some(name)) if target != database.root() => (parent.to_path_buf(), name.to_string_lossy().into_owned()),
        _ => return Err("give the new folder's path below assets/".into()),
    };
    std::fs::create_dir_all(&parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let made = database.create_folder(&parent, &name).map_err(|e| e.to_string())?;
    state.asset_folder_reveal = Some(made.clone());
    Ok(json!({"created": relative_path(&database, &made)}))
}

pub fn delete_folder(state: &mut EditorState, path: &str, confirm: bool) -> Result<Value, String> {
    let (folder, inside, mut answer) = {
        let database = assets::database();
        let folder = folder_path(&database, path)?;
        if folder == database.root() || !folder.is_dir() {
            return Err(format!("no folder '{path}' below assets/ (the asset folder itself cannot go)"));
        }
        let inside = database.assets_in(&folder);
        let references = database.references_into(&folder);
        let answer = json!({
            "folder": relative_path(&database, &folder),
            "assets": inside.iter().map(|id| asset_json(&database, *id)).collect::<Vec<_>>(),
            "references_from_outside": references_json(&database, &references),
        });
        (folder, inside, answer)
    };
    if !confirm {
        answer["deleted"] = json!(false);
        answer["note"] = json!("Nothing was deleted. Call again with confirm: true to move the folder and everything in it to the trash.");
        return Ok(answer);
    }
    if let Some(name) = unsaved_editor(state, &inside) {
        return Err(format!("'{name}' has unsaved changes in the editor: save or revert them first"));
    }
    state.push(Command::DeleteFolder(folder));
    answer["deleted"] = json!(true);
    answer["note"] = json!("Moved to the trash on the editor's next frame; editor_status → last status says how it went.");
    Ok(answer)
}

pub fn browser(
    state: &mut EditorState,
    category: Option<String>,
    folder: Option<String>,
    search: Option<String>,
    asset: Option<String>,
    menu: bool,
    expand: Option<bool>,
) -> Result<Value, String> {
    let database = assets::database();
    let asset = asset.map(|a| resolve(&database, &a)).transpose()?;
    let folder = folder.map(|f| folder_path(&database, &f)).transpose()?;
    if let Some(folder) = folder.as_ref().filter(|f| !f.is_dir()) {
        return Err(format!("no folder {}", relative_path(&database, folder)));
    }
    state.asset_browser_request = Some(BrowserRequest { category, folder, search, asset, menu, expand });
    Ok(json!({"view": state.asset_view.0, "folder": state.asset_view.1, "note": "The browser follows on its next update."}))
}
