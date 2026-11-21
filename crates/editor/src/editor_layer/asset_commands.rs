//! What the editor does for the asset browser's commands: making,
//! renaming, duplicating and deleting assets and folders. See
//! `docs/asset-browser.md`.

use std::path::Path;

use ferrite::core::assets::{self, create, AssetId, Removal};

use crate::editor::{dialogs, Command, Workspace};

use super::*;

impl EditorLayer {
    /// Run one of the asset browser's commands; `false` if it is not one.
    pub(super) fn run_asset_command(&mut self, command: &Command) -> bool {
        match command {
            Command::NewAssetDialog { kind, folder } => dialogs::new_asset(&self.state, *kind, folder.clone()),
            Command::NewAsset { kind, folder, name } => {
                let made = create::create(&mut assets::database(), *kind, folder.as_deref(), name.as_deref());
                match made {
                    Ok(id) => {
                        let mut state = self.state.borrow_mut();
                        state.asset_reveal = Some(id);
                        state.set_status(format!("Made {} '{}'", kind.label().to_lowercase(), name_of(id)));
                    }
                    Err(e) => self
                        .state
                        .borrow_mut()
                        .set_status(format!("Could not make the {}: {e}", kind.label().to_lowercase())),
                }
            }
            Command::NewFolderDialog(parent) => dialogs::new_folder(&self.state, parent.clone()),
            Command::NewFolder { parent, name } => {
                let made = assets::database().create_folder(parent, name);
                let mut state = self.state.borrow_mut();
                match made {
                    Ok(path) => {
                        state.asset_folder_reveal = Some(path);
                        state.set_status(format!("Made folder '{name}'"));
                    }
                    Err(e) => state.set_status(format!("Could not make the folder: {e}")),
                }
            }
            Command::RenameAssetDialog(id) => {
                let name = assets::database().get(*id).map(|e| e.meta.name.clone());
                if let Some(name) = name {
                    dialogs::rename_asset(&self.state, *id, &name);
                }
            }
            Command::RenameAsset { id, name } => {
                let Some(reopen) = self.release_editors(&[*id]) else {
                    return true;
                };
                let renamed = assets::database().rename(*id, name);
                match renamed {
                    Ok(()) => {
                        self.state.borrow_mut().asset_reveal = Some(*id);
                        self.state.borrow_mut().set_status(format!("Renamed to '{name}'"));
                    }
                    Err(e) => self.state.borrow_mut().set_status(format!("Could not rename: {e}")),
                }
                if let Some(workspace) = reopen {
                    self.run(Command::EditAsset { id: *id, variant: None });
                    self.run(Command::SetWorkspace(workspace));
                }
            }
            Command::RenameFolderDialog(path) => dialogs::rename_folder(&self.state, path.clone()),
            Command::RenameFolder { path, name } => {
                let inside = assets::database().assets_in(path);
                if self.release_editors(&inside).is_none() {
                    return true;
                }
                let renamed = assets::database().rename_folder(path, name);
                let mut state = self.state.borrow_mut();
                match renamed {
                    Ok(to) => {
                        state.asset_folder_reveal = Some(to);
                        state.set_status(format!("Renamed folder to '{name}'"));
                    }
                    Err(e) => state.set_status(format!("Could not rename the folder: {e}")),
                }
            }
            Command::DuplicateAsset(id) => {
                let made = assets::database().duplicate(*id);
                let mut state = self.state.borrow_mut();
                match made {
                    Ok(copy) => {
                        state.asset_reveal = Some(copy);
                        state.set_status(format!("Made '{}'", name_of(copy)));
                    }
                    Err(e) => state.set_status(format!("Could not duplicate: {e}")),
                }
            }
            Command::DeleteAssetDialog(id) => dialogs::delete_asset(&self.state, *id),
            Command::DeleteAsset(id) => {
                if self.release_editors(&[*id]).is_none() {
                    return true;
                }
                let name = name_of(*id);
                let deleted = assets::database().delete(*id, Removal::Trash);
                let mut state = self.state.borrow_mut();
                match deleted {
                    Ok(()) => {
                        if state.asset_selection.as_ref().is_some_and(|s| s.id == *id) {
                            state.asset_selection = None;
                        }
                        state.set_status(format!("Moved '{name}' to the trash"));
                    }
                    Err(e) => state.set_status(format!("Could not delete '{name}': {e}")),
                }
            }
            Command::DeleteFolderDialog(path) => dialogs::delete_folder(&self.state, path.clone()),
            Command::DeleteFolder(path) => {
                let inside = assets::database().assets_in(path);
                if self.release_editors(&inside).is_none() {
                    return true;
                }
                let deleted = assets::database().delete_folder(path, Removal::Trash);
                let mut state = self.state.borrow_mut();
                match deleted {
                    Ok(()) => {
                        if state.asset_selection.as_ref().is_some_and(|s| inside.contains(&s.id)) {
                            state.asset_selection = None;
                        }
                        state.set_status(format!("Moved folder '{}' to the trash", folder_name(path)));
                    }
                    Err(e) => state.set_status(format!("Could not delete the folder: {e}")),
                }
            }
            _ => return false,
        }
        true
    }

    /// Close the editors open on any of `ids`, so their files can be
    /// renamed or removed. `None` (with a status message) if one has unsaved
    /// changes; otherwise the workspace to go back to if the current one
    /// showed one of them.
    fn release_editors(&mut self, ids: &[AssetId]) -> Option<Option<Workspace>> {
        let mut state = self.state.borrow_mut();
        let unsaved = state
            .asset_edit
            .as_ref()
            .filter(|e| ids.contains(&e.meta.id) && e.dirty)
            .map(|e| e.meta.name.clone())
            .or_else(|| state.material.as_ref().filter(|e| ids.contains(&e.asset) && e.dirty()).map(|e| e.name.clone()))
            .or_else(|| state.scatter.as_ref().filter(|e| ids.contains(&e.asset) && e.dirty()).map(|e| e.name.clone()));
        if let Some(name) = unsaved {
            state.set_status(format!("'{name}' has unsaved changes: save or revert them first"));
            return None;
        }
        let mut closed = Vec::new();
        if state.asset_edit.as_ref().is_some_and(|e| ids.contains(&e.meta.id)) {
            state.asset_edit = None;
            closed.push(Workspace::Asset);
        }
        if state.material.as_ref().is_some_and(|e| ids.contains(&e.asset)) {
            state.material = None;
            closed.push(Workspace::Material);
        }
        if state.scatter.as_ref().is_some_and(|e| ids.contains(&e.asset)) {
            state.scatter = None;
            closed.push(Workspace::ScatterGraph);
        }
        let current = state.workspace;
        drop(state);
        if closed.contains(&current) {
            self.run(Command::SetWorkspace(Workspace::Scene));
            return Some(Some(current));
        }
        Some(None)
    }
}

fn name_of(id: AssetId) -> String {
    assets::database().get(id).map(|e| e.meta.name.clone()).unwrap_or_default()
}

pub(crate) fn folder_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}
