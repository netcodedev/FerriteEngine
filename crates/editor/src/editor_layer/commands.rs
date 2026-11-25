//! What the editor does for a `Command` from its panels, menus and
//! shortcuts.

use cgmath::{EuclideanSpace, Point3};
use ferrite::core::{
    assets::{self, AssetId, AssetKind},
    memory,
    entity::{
        component::{animation_component::AnimationComponent, model_component::ModelComponent},
        EntityHandle,
    },
    scene::{
        scene_file::{ComponentData, EntityData},
        Scene,
    },
};
use ferrite::player::PLAYER_GRAPH_PATH;
use ferrite::terrain::scatter::graph::NodeKind;
use glfw::{CursorMode, Key, Modifiers};

use crate::editor::{
    anim::AnimSelection,
    asset_editor::AssetEdit,
    clip_editor::{self, ClipRequest},
    dialogs,
    export::{self, Exporter},
    material::MaterialEdit,
    scatter::ScatterEdit,
    Command, GizmoMode, Mode, Workspace,
};
use crate::new_script::new_script_file;

use super::*;

impl EditorLayer {
    /// Export the game for `platform` into `out` (the folder the dialog
    /// gave), unless it lies in a copy of the project.
    pub(super) fn export_into(&mut self, out: std::path::PathBuf, platform: ferrite::core::export::Platform) {
        if self.exporter.is_running() {
            self.state.borrow_mut().set_status(tr!("commands.export_running"));
            return;
        }
        match export::copied_folder_containing(self.project.dir(), &out) {
            Some(folder) => self.state.borrow_mut().set_status(tr!("commands.export_into_copied", folder = folder)),
            None => self.exporter.start(self.project.clone(), out, platform),
        }
    }

    pub(super) fn run_commands(&mut self) {
        loop {
            let commands = std::mem::take(&mut self.state.borrow_mut().commands);
            if commands.is_empty() {
                break;
            }
            for command in commands {
                self.run(command);
            }
        }
    }

    pub(super) fn run(&mut self, command: Command) {
        if self.run_asset_command(&command) {
            return;
        }
        let mode = self.state.borrow().mode;
        // Closing during play: leave play mode first, then ask about unsaved
        // edits as usual.
        if mode == Mode::Play {
            if let Command::ConfirmDiscard(then) = &command {
                if matches!(**then, Command::Quit) {
                    self.run(Command::Stop);
                    self.run(command);
                    return;
                }
            }
        }
        let edits_scene = matches!(
            command,
            Command::NewScene
                | Command::OpenDialog
                | Command::Open(_)
                | Command::AddEntity { .. }
                | Command::AddComponent(..)
                | Command::RemoveComponent(..)
                | Command::PickScatterAsset { .. }
                | Command::SetScatterAsset { .. }
                | Command::PickModelAsset(_)
                | Command::PickScript(_)
                | Command::SetEntityModel { .. }
                | Command::SpawnModel(_)
                | Command::RemoveScatterModel(..)
                | Command::Delete(_)
                | Command::Duplicate(_)
                | Command::ConfirmDiscard(_)
                | Command::EditAnimationGraph(_)
        );
        if mode == Mode::Play && edits_scene {
            self.state
                .borrow_mut()
                .set_status(tr!("commands.stop_play_first"));
            return;
        }

        match command {
            // Done by `run_asset_command` above.
            Command::NewAssetDialog { .. }
            | Command::NewAsset { .. }
            | Command::NewFolderDialog(_)
            | Command::NewFolder { .. }
            | Command::RenameAssetDialog(_)
            | Command::RenameAsset { .. }
            | Command::RenameFolderDialog(_)
            | Command::RenameFolder { .. }
            | Command::DuplicateAsset(_)
            | Command::DeleteAssetDialog(_)
            | Command::DeleteAsset(_)
            | Command::DeleteFolderDialog(_)
            | Command::DeleteFolder(_) => {}
            Command::NewScene => match Scene::from_file(&self.new_scene_file()) {
                Ok(scene) => {
                    self.replace_scene(scene);
                    let mut state = self.state.borrow_mut();
                    state.scene_path = None;
                    state.dirty = false;
                    state.select(None);
                    state.set_status(tr!("commands.new_scene"));
                }
                Err(e) => self.state.borrow_mut().set_status(tr!("commands.new_scene_failed", error = e)),
            },
            Command::OpenDialog => dialogs::open(&self.state),
            Command::Open(path) => match Scene::load_from_path(&path) {
                Ok(scene) => {
                    self.replace_scene(scene);
                    self.remember_scene(&path);
                    {
                        let mut state = self.state.borrow_mut();
                        state.set_status(tr!("commands.opened", path = path.display()));
                        state.scene_path = Some(path);
                        state.dirty = false;
                        state.select(None);
                    }
                    Self::report_scene_problems(&self.state, &self.scene);
                }
                Err(e) => self
                    .state
                    .borrow_mut()
                    .set_status(tr!("commands.open_failed", path = path.display(), error = e)),
            },
            // The scene in play mode is the game's, not the one to save.
            Command::Save | Command::SaveAsDialog | Command::SaveAs { .. } | Command::SaveFirst(_)
                if mode == Mode::Play =>
            {
                self.state
                    .borrow_mut()
                    .set_status(tr!("commands.stop_play_before_saving"));
            }
            Command::Save => {
                let path = self.state.borrow().scene_path.clone();
                match path {
                    Some(path) => {
                        self.save(path);
                    }
                    None => dialogs::save_as(&self.state, None),
                }
            }
            Command::SaveAsDialog => dialogs::save_as(&self.state, None),
            Command::SaveAs { path, then } => {
                if self.save(path) {
                    if let Some(then) = then {
                        self.state.borrow_mut().push(*then);
                    }
                }
            }
            Command::ConfirmDiscard(then) => {
                let unsaved = self.unsaved_for(&then);
                if unsaved.any() {
                    dialogs::confirm_discard(&self.state, *then, unsaved, None);
                } else {
                    self.state.borrow_mut().push(*then);
                }
            }
            Command::SaveFirst(then) => {
                let unsaved = self.unsaved_for(&then);
                if unsaved.asset && self.state.borrow().material.as_ref().is_some_and(|e| e.dirty()) {
                    self.run(Command::SaveMaterial);
                    if self.state.borrow().material.as_ref().is_some_and(|e| e.dirty()) {
                        return;
                    }
                }
                if unsaved.asset && self.state.borrow().scatter.as_ref().is_some_and(|e| e.dirty()) {
                    self.run(Command::SaveScatter);
                    if self.state.borrow().scatter.as_ref().is_some_and(|e| e.dirty()) {
                        return;
                    }
                }
                if unsaved.graph {
                    self.run(Command::SaveGraph);
                    if self.state.borrow().anim.dirty {
                        return;
                    }
                }
                if unsaved.asset {
                    self.run(Command::SaveAsset);
                    if self.unsaved_for(&then).asset {
                        return;
                    }
                }
                if unsaved.scene {
                    let path = self.state.borrow().scene_path.clone();
                    match path {
                        Some(path) => {
                            if !self.save(path) {
                                return;
                            }
                        }
                        // Goes on once the user picked where to save it.
                        None => {
                            dialogs::save_as(&self.state, Some(*then));
                            return;
                        }
                    }
                }
                self.state.borrow_mut().push(*then);
            }
            Command::Play => {
                if mode == Mode::Play {
                    return;
                }
                if self.state.borrow().workspace != Workspace::Scene {
                    self.run(Command::SetWorkspace(Workspace::Scene));
                }
                log::info!("Play mode started");
                self.play_snapshot = Some(self.scene.to_file());
                self.dirty_before_play = self.state.borrow().dirty;
                self.scene.set_editor_camera(None);
                self.scene.set_physics_enabled(true);
                self.state.borrow_mut().mode = Mode::Play;
                self.pending_cursor = Some(CursorMode::Disabled);
                if self.scene.get_active_camera().is_none() {
                    self.state.borrow_mut().set_status(tr!("commands.no_camera"));
                }
            }
            Command::Stop => {
                if mode == Mode::Edit {
                    return;
                }
                log::info!("Play mode stopped");
                // A game played on a server leaves it with play mode.
                ferrite::core::net::disconnect();
                if let Some(snapshot) = self.play_snapshot.take() {
                    match Scene::from_file(&snapshot) {
                        Ok(scene) => {
                            self.replace_scene(scene);
                            self.state.borrow_mut().dirty = self.dirty_before_play;
                        }
                        Err(e) => self
                            .state
                            .borrow_mut()
                            .set_status(tr!("commands.restore_failed", error = e)),
                    }
                }
                self.scene.set_physics_enabled(false);
                self.state.borrow_mut().mode = Mode::Edit;
                self.pending_cursor = Some(CursorMode::Normal);
            }
            Command::AddEntity { parent, template } => {
                let data = self.template_data(template);
                match self.scene.instantiate(&data, true) {
                    Ok(mut entity) => {
                        // Children are placed relative to their parent.
                        if parent.is_some() {
                            entity.set_local_position(Point3::origin());
                        }
                        let id = entity.id;
                        if self.scene.add_entity_to(parent, entity).is_ok() {
                            let mut state = self.state.borrow_mut();
                            state.select(Some(id));
                            state.dirty = true;
                        }
                    }
                    Err(e) => self
                        .state
                        .borrow_mut()
                        .set_status(tr!("commands.create_entity_failed", error = e)),
                }
            }
            Command::AddComponent(id, data) => {
                if let Some(entity) = self.scene.get_entity_mut(&id) {
                    data.attach(entity);
                    self.state.borrow_mut().dirty = true;
                }
            }
            Command::RemoveComponent(id, index) => {
                let removed = self
                    .scene
                    .get_entity_mut(&id)
                    .and_then(|entity| entity.remove_component_at(index));
                if let Some(mut component) = removed {
                    component.on_remove(&mut self.scene.physics_engine);
                    self.state.borrow_mut().dirty = true;
                }
            }
            Command::ImportAssets => self.import_assets(),
            Command::ReloadScripts => {
                if let Some(event) = self.scripts.rebuild() {
                    self.on_script_event(event);
                }
            }
            Command::MemorySnapshot => {
                let taken = crate::editor::stats::memory_snapshot(&self.scene, &mut self.state.borrow_mut().stats, "");
                let message = match taken {
                    Ok((_, Some(diff))) => {
                        let resident = diff
                            .changes
                            .iter()
                            .find(|c| c.group == memory::PROCESS && c.name == "resident (RSS)")
                            .map_or(0, |c| c.delta());
                        let top = diff
                            .top_growth(1)
                            .next()
                            .map(|c| format!("{} {} {}", c.group, c.name, memory::signed(c.delta())))
                            .unwrap_or_else(|| "-".into());
                        tr!(
                            "commands.memory_snapshot_diff",
                            change = memory::signed(resident),
                            minutes = diff.seconds / 60,
                            top = top
                        )
                        .to_string()
                    }
                    Ok((path, None)) => {
                        tr!("commands.memory_snapshot_first", path = path.display().to_string()).to_string()
                    }
                    Err(error) => tr!("commands.memory_snapshot_failed", error = error).to_string(),
                };
                self.state.borrow_mut().set_status(message);
            }
            Command::ExportGame(platform) => {
                // The export packs what is on disk.
                let unsaved = self.unsaved_for(&Command::ExportGame(platform));
                if unsaved.any() && !self.exporter.is_running() {
                    dialogs::confirm_discard(
                        &self.state,
                        Command::ExportGame(platform),
                        unsaved,
                        Some((tr!("commands.export_anyway"), Command::ExportAnyway(platform))),
                    );
                } else {
                    self.run(Command::ExportAnyway(platform));
                }
            }
            Command::ExportAnyway(platform) => {
                if self.exporter.is_running() {
                    self.state.borrow_mut().set_status(tr!("commands.export_running"));
                } else {
                    // The folder is asked for on the dialog's own thread.
                    self.open_file_dialog(move || super::DialogAnswer::ExportFolder(Exporter::pick_folder(), platform));
                }
            }
            Command::RefreshAssets => {
                let (count, problems) = {
                    let mut database = assets::database();
                    let mut problems = database.scan();
                    // Files added by hand since the last look become assets.
                    problems.extend(database.register_new_files().1);
                    problems.extend(ferrite::core::material::asset::ensure_all_model_materials(&mut database).1);
                    (database.len(), problems)
                };
                let mut state = self.state.borrow_mut();
                match problems.first() {
                    Some(problem) => state.set_status(tr!("commands.assets_problem", problem = problem)),
                    None => state.set_status(tr!("commands.assets_count", count = count)),
                }
            }
            Command::PickScatterAsset { entity, layer } => {
                dialogs::pick_scatter_asset(&self.state, entity, layer)
            }
            Command::SetScatterAsset {
                entity,
                layer,
                asset,
            } => {
                // Terrains scatter scatter graphs only.
                if !Self::is_scatter_graph(asset.id) {
                    self.state
                        .borrow_mut()
                        .set_status(tr!("commands.scatter_graphs_only"));
                    return;
                }
                if let Some(terrain) = self.terrain_mut(&entity) {
                    match layer {
                        Some(layer) => terrain.scatter_mut().set_asset(layer, asset),
                        None => terrain.scatter_mut().add_layer(asset),
                    }
                    self.state.borrow_mut().dirty = true;
                }
            }
            Command::PickModelAsset(entity) => dialogs::pick_entity_model(&self.state, entity),
            Command::PickScript(entity) => dialogs::pick_script(&self.state, entity),
            Command::NewScriptDialog => dialogs::new_script(&self.state),
            Command::NewScript(name) => {
                let message = match new_script_file(self.scripts.folder(), &name) {
                    Ok(file) => tr!("commands.script_created", file = file),
                    Err(e) => tr!("commands.script_failed", error = e),
                };
                self.state.borrow_mut().set_status(message);
            }
            Command::SetEntityModel { entity, asset } => {
                let Some(target) = self.scene.get_entity_mut(&entity) else {
                    return;
                };
                let result = match target.get_component_mut::<ModelComponent>() {
                    Some(model) => model.set_asset(asset),
                    None => {
                        // Reports a failure through a placeholder component.
                        ComponentData::Model { asset }.attach(target);
                        Ok(())
                    }
                };
                let mut state = self.state.borrow_mut();
                match result {
                    Ok(()) => state.dirty = true,
                    Err(e) => state.set_status(tr!("commands.model_load_failed", error = e)),
                }
            }
            Command::SpawnModel(asset) => {
                let name = assets::database()
                    .display_name(&asset)
                    .unwrap_or_else(|| "model".into());
                let spawn = self.viewport.spawn_point();
                let data = EntityData {
                    id: u64::from(EntityHandle::new()),
                    name,
                    position: [spawn.x, spawn.y, spawn.z],
                    rotation: [1.0, 0.0, 0.0, 0.0],
                    scale: [1.0; 3],
                    components: vec![ComponentData::Model { asset }],
                    children: Vec::new(),
                };
                match self.scene.instantiate(&data, true) {
                    Ok(entity) => {
                        let id = entity.id;
                        self.scene.add_entity(entity);
                        let mut state = self.state.borrow_mut();
                        state.select(Some(id));
                        state.dirty = true;
                    }
                    Err(e) => self
                        .state
                        .borrow_mut()
                        .set_status(tr!("commands.create_entity_failed", error = e)),
                }
            }
            Command::RemoveScatterModel(entity, layer) => {
                if let Some(terrain) = self.terrain_mut(&entity) {
                    terrain.scatter_mut().remove_layer(layer);
                    self.state.borrow_mut().dirty = true;
                }
            }
            Command::Delete(id) => {
                if self.scene.remove_entity(&id) {
                    let mut state = self.state.borrow_mut();
                    state.select(None);
                    state.dirty = true;
                }
            }
            Command::Duplicate(id) => {
                let Some(data) = self.scene.get_entity(&id).and_then(|e| e.to_data()) else {
                    return;
                };
                let parent = self
                    .scene
                    .get_ancestors(&id)
                    .and_then(|ancestors| ancestors.last().copied());
                match self.scene.instantiate(&data, true) {
                    Ok(mut entity) => {
                        entity.set_name(format!("{} copy", data.name));
                        let new_id = entity.id;
                        if self.scene.add_entity_to(parent, entity).is_ok() {
                            let mut state = self.state.borrow_mut();
                            state.select(Some(new_id));
                            state.dirty = true;
                        }
                    }
                    Err(e) => self
                        .state
                        .borrow_mut()
                        .set_status(tr!("commands.duplicate_failed", error = e)),
                }
            }
            Command::Focus(id) => self.viewport.focus(&self.scene, id),
            Command::Quit => self.pending_quit = true,
            Command::SettingsDialog => crate::editor::settings_dialog::show(&self.state),
            Command::SetWorkspace(workspace) => {
                if mode == Mode::Play && workspace == Workspace::Animation {
                    self.state
                        .borrow_mut()
                        .set_status(tr!("commands.stop_play_before_anim"));
                    return;
                }
                let mut state = self.state.borrow_mut();
                state.workspace = workspace;
                state.scene_panels.write(workspace == Workspace::Scene);
                state.anim_panels.write(workspace == Workspace::Animation);
                state.asset_panels.write(workspace == Workspace::Asset);
                state.scatter_panels.write(workspace == Workspace::ScatterGraph);
                state.material_panels.write(workspace == Workspace::Material);
                state.browser_panel.write(workspace != Workspace::Animation);
            }
            Command::CloseScatterEditor => {
                // The world plants the saved graph again (unsaved edits stay
                // in the editor's copy).
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.clear_preview(&mut self.scene);
                }
                self.run(Command::SetWorkspace(Workspace::Scene));
            }
            Command::SaveScatter => {
                let result = self.state.borrow_mut().scatter.as_mut().map(|edit| edit.save().map(|_| edit.name.clone()));
                let message = match result {
                    Some(Ok(name)) => tr!("commands.scatter_saved", name = name),
                    Some(Err(e)) => tr!("commands.scatter_save_failed", error = e),
                    None => return,
                };
                self.state.borrow_mut().set_status(message);
            }
            Command::RevertScatter => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.revert();
                }
            }
            Command::ScatterAddMenu => self.state.borrow_mut().scatter_open_menu = true,
            Command::ScatterFit => self.state.borrow_mut().scatter_fit = true,
            Command::ScatterUndo => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.undo();
                }
            }
            Command::ScatterRedo => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.redo();
                }
            }
            Command::ScatterDelete => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.delete_selection();
                }
            }
            Command::ScatterCopy => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.copy();
                }
            }
            Command::ScatterPaste => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.paste();
                }
            }
            Command::PickScatterModel(node) => dialogs::pick_scatter_model(&self.state, node),
            Command::SetScatterModel { node, asset } => {
                if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                    edit.edit(|graph| {
                        if let Some(NodeKind::SpawnMesh(spawn)) = graph.node_mut(node).map(|n| &mut n.kind) {
                            spawn.asset = Some(asset);
                            spawn.model.clear();
                        }
                    });
                }
            }
            Command::EditAsset { id, .. } if MaterialEdit::handles(id) => {
                let open = self.state.borrow().material.as_ref().is_some_and(|edit| edit.asset == id);
                if !open {
                    let unsaved = self
                        .state
                        .borrow()
                        .material
                        .as_ref()
                        .filter(|edit| edit.dirty())
                        .map(|edit| edit.name.clone());
                    if let Some(name) = unsaved {
                        self.state.borrow_mut().set_status(tr!("commands.save_or_revert_first", name = name));
                        self.run(Command::SetWorkspace(Workspace::Material));
                        return;
                    }
                    match MaterialEdit::open(id) {
                        Ok(edit) => self.state.borrow_mut().material = Some(edit),
                        Err(e) => {
                            self.state.borrow_mut().set_status(e);
                            return;
                        }
                    }
                }
                self.run(Command::SetWorkspace(Workspace::Material));
            }
            Command::CloseMaterialEditor => self.run(Command::SetWorkspace(Workspace::Scene)),
            Command::SaveMaterial => {
                let result = self.state.borrow_mut().material.as_mut().map(|edit| edit.save().map(|_| edit.name.clone()));
                let message = match result {
                    Some(Ok(name)) => tr!("commands.material_saved", name = name),
                    Some(Err(e)) => tr!("commands.material_save_failed", error = e),
                    None => return,
                };
                self.state.borrow_mut().set_status(message);
            }
            Command::RevertMaterial => {
                if let Some(edit) = self.state.borrow_mut().material.as_mut() {
                    edit.revert();
                }
            }
            Command::MaterialAddMenu => self.state.borrow_mut().material_open_menu = true,
            Command::MaterialFit => self.state.borrow_mut().material_fit = true,
            Command::MaterialUndo => {
                if let Some(edit) = self.state.borrow_mut().material.as_mut() {
                    edit.undo();
                }
            }
            Command::MaterialRedo => {
                if let Some(edit) = self.state.borrow_mut().material.as_mut() {
                    edit.redo();
                }
            }
            Command::MaterialDelete => {
                if let Some(edit) = self.state.borrow_mut().material.as_mut() {
                    edit.delete_selection();
                }
            }
            Command::MaterialAddParam => {
                let mut state = self.state.borrow_mut();
                if let Some(edit) = state.material.as_mut() {
                    let taken: Vec<String> = edit.graph().map(|g| g.params.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
                    let name = (1..).map(|n| format!("param_{n}")).find(|n| !taken.contains(n)).unwrap_or_default();
                    let op = serde_json::json!({"op": "add_param", "name": name, "type": "float"});
                    if let Err(e) = edit.apply_ops(&[op]) {
                        state.set_status(e);
                    }
                }
            }
            Command::MaterialRemoveParam(name) => {
                if let Some(edit) = self.state.borrow_mut().material.as_mut() {
                    let _ = edit.apply_ops(&[serde_json::json!({"op": "remove_param", "name": name})]);
                }
            }
            Command::PickMaterialTexture { param } => dialogs::pick_material_texture(&self.state, param),
            Command::SetMaterialTexture { param, texture } => {
                let mut state = self.state.borrow_mut();
                if let Some(edit) = state.material.as_mut() {
                    let value = match texture {
                        ferrite::core::material::asset::TextureSetting::Auto => "auto".to_string(),
                        ferrite::core::material::asset::TextureSetting::None => "none".to_string(),
                        ferrite::core::material::asset::TextureSetting::Asset(id) => id.to_string(),
                    };
                    if let Err(e) = edit.set_material_property(
                        &param,
                        &ferrite::core::entity::component::PropertyValue::Text(value),
                    ) {
                        state.set_status(e);
                    }
                }
            }
            Command::PickModelMaterial { material } => dialogs::pick_model_material(&self.state, material),
            Command::SetModelMaterial { material, asset } => {
                if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
                    edit.set_material(material, asset);
                }
            }
            Command::EditAsset { id, .. } if Self::is_scatter_graph(id) => {
                let open = self.state.borrow().scatter.as_ref().is_some_and(|edit| edit.asset == id);
                if !open {
                    let unsaved = self
                        .state
                        .borrow()
                        .scatter
                        .as_ref()
                        .filter(|edit| edit.dirty())
                        .map(|edit| edit.name.clone());
                    if let Some(name) = unsaved {
                        self.state
                            .borrow_mut()
                            .set_status(tr!("commands.save_or_revert_first", name = name));
                        self.run(Command::SetWorkspace(Workspace::ScatterGraph));
                        return;
                    }
                    match ScatterEdit::open(id) {
                        Ok(edit) => {
                            let mut state = self.state.borrow_mut();
                            if let Some(old) = state.scatter.as_mut() {
                                old.clear_preview(&mut self.scene);
                            }
                            state.scatter = Some(edit);
                        }
                        Err(e) => {
                            self.state.borrow_mut().set_status(e);
                            return;
                        }
                    }
                }
                self.run(Command::SetWorkspace(Workspace::ScatterGraph));
            }
            Command::EditAsset { id, variant } => {
                let unsaved = self
                    .state
                    .borrow()
                    .asset_edit
                    .as_ref()
                    .filter(|edit| edit.dirty && edit.meta.id != id)
                    .map(|edit| edit.meta.name.clone());
                if let Some(name) = unsaved {
                    self.state
                        .borrow_mut()
                        .set_status(tr!("commands.save_or_revert_first", name = name));
                    self.run(Command::SetWorkspace(Workspace::Asset));
                    return;
                }
                let already_open = self
                    .state
                    .borrow()
                    .asset_edit
                    .as_ref()
                    .is_some_and(|edit| edit.meta.id == id);
                if already_open {
                    // Keep its unsaved edits; just show the asked variant.
                    let mut state = self.state.borrow_mut();
                    if let (Some(edit), Some(variant)) = (state.asset_edit.as_mut(), variant) {
                        let index = edit
                            .meta
                            .as_model()
                            .and_then(|model| model.variants.iter().position(|v| v.name == variant));
                        if let Some(index) = index.filter(|index| *index != edit.variant) {
                            edit.variant = index;
                            edit.lod = None;
                            edit.structure += 1;
                            edit.preview += 1;
                        }
                    }
                } else {
                    match AssetEdit::open(id, variant.as_deref()) {
                        Ok(edit) => self.state.borrow_mut().asset_edit = Some(edit),
                        Err(e) => {
                            self.state.borrow_mut().set_status(e);
                            return;
                        }
                    }
                }
                self.run(Command::SetWorkspace(Workspace::Asset));
            }
            Command::SaveAsset => {
                let mut state = self.state.borrow_mut();
                let result = state
                    .asset_edit
                    .as_mut()
                    .map(|edit| edit.save().map(|()| edit.meta.name.clone()));
                match result {
                    Some(Ok(name)) => state.set_status(tr!("commands.asset_saved", name = name)),
                    Some(Err(e)) => state.set_status(tr!("commands.asset_save_failed", error = e)),
                    None => {}
                }
            }
            Command::RevertAsset => {
                let mut state = self.state.borrow_mut();
                let result = state.asset_edit.as_mut().map(|edit| edit.revert());
                match result {
                    Some(Ok(())) => state.set_status(tr!("commands.asset_reverted")),
                    Some(Err(e)) => state.set_status(e),
                    None => {}
                }
            }
            Command::CloseAssetEditor => self.run(Command::SetWorkspace(Workspace::Scene)),
            Command::NewClipDialog(model) => dialogs::new_clip(&self.state, model),
            Command::NewClip { model, name } => match clip_editor::create_clip(&name, model) {
                Ok(id) => {
                    self.state
                        .borrow_mut()
                        .set_status(tr!("commands.clip_made", name = name));
                    self.run(Command::EditAsset { id, variant: None });
                }
                Err(e) => self
                    .state
                    .borrow_mut()
                    .set_status(tr!("commands.clip_failed", error = e)),
            },
            Command::Clip(request) => {
                if let Some(clip) = self
                    .state
                    .borrow_mut()
                    .asset_edit
                    .as_mut()
                    .and_then(|edit| edit.clip.as_mut())
                {
                    clip.requests.push(request);
                }
            }
            Command::SaveGraph => {
                let result = self.state.borrow_mut().anim.save();
                match result {
                    Ok(()) => {
                        let path = self.state.borrow().anim.path.clone();
                        let applied = apply_saved_graph(
                            self.scene.get_entities_mut(),
                            &path.to_string_lossy(),
                        );
                        let message = match applied {
                            Ok(0) => tr!("commands.graph_saved", path = path.display()),
                            Ok(1) => tr!("commands.graph_saved_applied_one", path = path.display()),
                            Ok(n) => tr!("commands.graph_saved_applied", path = path.display(), count = n),
                            Err(e) => tr!("commands.graph_saved_apply_failed", path = path.display(), error = e),
                        };
                        self.state.borrow_mut().set_status(message);
                    }
                    Err(e) => self
                        .state
                        .borrow_mut()
                        .set_status(tr!("commands.graph_save_failed", error = e)),
                }
            }
            Command::RevertGraph => {
                let result = self.state.borrow_mut().anim.reload();
                let mut state = self.state.borrow_mut();
                match result {
                    Ok(()) => state.set_status(tr!("commands.graph_reverted")),
                    Err(e) => state.set_status(tr!("commands.graph_reload_failed", error = e)),
                }
            }
            Command::EditAnimationGraph(id) => {
                let path = self
                    .scene
                    .get_entity(&id)
                    .and_then(|e| e.get_own_component::<AnimationComponent>())
                    .and_then(|a| a.graph_path().map(str::to_owned))
                    .unwrap_or_else(|| PLAYER_GRAPH_PATH.to_owned());
                let (current, dirty) = {
                    let s = self.state.borrow();
                    (s.anim.path.to_string_lossy().into_owned(), s.anim.dirty)
                };
                if current != path {
                    if dirty {
                        dialogs::confirm_graph_switch(&self.state, &current, Command::EditAnimationGraph(id));
                        return;
                    }
                    let opened = self.state.borrow_mut().anim.open(&path);
                    if let Err(e) = opened {
                        self.state
                            .borrow_mut()
                            .set_status(tr!("commands.graph_open_failed", path = path, error = e));
                        return;
                    }
                }
                self.run(Command::SetWorkspace(Workspace::Animation));
            }
            Command::CloseGraphEditor => self.run(Command::SetWorkspace(Workspace::Scene)),
            Command::DeleteGraphSelection => {
                let mut state = self.state.borrow_mut();
                let anim = &mut state.anim;
                match anim.selection.clone() {
                    AnimSelection::State(name) => anim.file.remove_state(&name),
                    AnimSelection::Transition(index) => {
                        if index < anim.file.transitions.len() {
                            anim.file.transitions.remove(index);
                        }
                    }
                    AnimSelection::None => return,
                }
                anim.selection = AnimSelection::None;
                anim.touch_structure();
            }
        }
    }

    /// What `then` would lose (quitting) or leave out (an export) of the
    /// edits not saved yet.
    /// Whether asset `id` is a scatter graph (opened in its own workspace).
    fn is_scatter_graph(id: AssetId) -> bool {
        assets::database()
            .get(id)
            .is_some_and(|entry| matches!(entry.meta.kind, AssetKind::ScatterGraph(_)))
    }

    fn unsaved_for(&self, then: &Command) -> dialogs::Unsaved {
        let state = self.state.borrow();
        let graph = state.anim.dirty;
        // An unsaved scatter graph counts as an unsaved asset.
        let asset = state.asset_edit.as_ref().is_some_and(|edit| edit.dirty)
            || state.scatter.as_ref().is_some_and(|edit| edit.dirty())
            || state.material.as_ref().is_some_and(|edit| edit.dirty());
        match then {
            Command::Quit | Command::ExportGame(_) => dialogs::Unsaved {
                scene: state.dirty,
                graph,
                asset,
            },
            // Switching graphs only replaces the open graph.
            Command::EditAnimationGraph(_) => dialogs::Unsaved {
                graph,
                ..Default::default()
            },
            // Opening another scene keeps the graph and asset edits in
            // memory.
            _ => dialogs::Unsaved {
                scene: state.dirty,
                ..Default::default()
            },
        }
    }

    /// Keyboard shortcuts that are not handled by a focused widget.
    pub(super) fn shortcut(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let ctrl = modifiers.contains(Modifiers::Control);
        let shift = modifiers.contains(Modifiers::Shift);
        let mode = self.state.borrow().mode;
        let selected = self.state.borrow().selected();
        let workspace = self.state.borrow().workspace;
        if workspace == Workspace::Asset {
            // Scene shortcuts (delete, gizmos, ...) do not apply here.
            let clip_open = self
                .state
                .borrow()
                .asset_edit
                .as_ref()
                .is_some_and(|edit| edit.clip.is_some());
            let command = match (key, ctrl) {
                (Key::S, true) => Some(Command::SaveAsset),
                (Key::Escape, false) => Some(Command::CloseAssetEditor),
                (Key::Space, false) if clip_open => Some(Command::Clip(ClipRequest::TogglePlay)),
                (Key::K, false) if clip_open => Some(Command::Clip(ClipRequest::Key)),
                (Key::Delete, false) if clip_open => Some(Command::Clip(ClipRequest::DeleteKey)),
                (Key::Home, false) if clip_open => Some(Command::Clip(ClipRequest::ToStart)),
                _ => None,
            };
            return match command {
                Some(command) => {
                    self.state.borrow_mut().push(command);
                    true
                }
                None => false,
            };
        }
        if workspace == Workspace::ScatterGraph {
            let command = match (key, ctrl, shift) {
                (Key::S, true, _) => Some(Command::SaveScatter),
                (Key::Z, true, false) => Some(Command::ScatterUndo),
                (Key::Z, true, true) | (Key::Y, true, _) => Some(Command::ScatterRedo),
                (Key::C, true, _) => Some(Command::ScatterCopy),
                (Key::V, true, _) => Some(Command::ScatterPaste),
                (Key::Delete | Key::Backspace, false, _) => Some(Command::ScatterDelete),
                (Key::Space, false, _) | (Key::A, false, true) => Some(Command::ScatterAddMenu),
                (Key::F, false, _) | (Key::Home, false, _) => Some(Command::ScatterFit),
                (Key::Escape, false, _) => {
                    if let Some(edit) = self.state.borrow_mut().scatter.as_mut() {
                        edit.select(Vec::new());
                    }
                    return true;
                }
                _ => None,
            };
            return match command {
                Some(command) => {
                    self.state.borrow_mut().push(command);
                    true
                }
                None => false,
            };
        }
        if workspace == Workspace::Material {
            let graph = self.state.borrow().material.as_ref().is_some_and(|e| e.is_graph());
            let command = match (key, ctrl, shift) {
                (Key::S, true, _) => Some(Command::SaveMaterial),
                (Key::Z, true, false) => Some(Command::MaterialUndo),
                (Key::Z, true, true) | (Key::Y, true, _) => Some(Command::MaterialRedo),
                (Key::Delete | Key::Backspace, false, _) if graph => Some(Command::MaterialDelete),
                (Key::Space, false, _) | (Key::A, false, true) if graph => Some(Command::MaterialAddMenu),
                (Key::F, false, _) | (Key::Home, false, _) if graph => Some(Command::MaterialFit),
                (Key::Escape, false, _) => {
                    if let Some(edit) = self.state.borrow_mut().material.as_mut() {
                        edit.select(Vec::new());
                    }
                    return true;
                }
                _ => None,
            };
            return match command {
                Some(command) => {
                    self.state.borrow_mut().push(command);
                    true
                }
                None => false,
            };
        }
        if workspace == Workspace::Animation {
            let command = match (key, ctrl) {
                (Key::S, true) => Some(Command::SaveGraph),
                (Key::Delete, false) => Some(Command::DeleteGraphSelection),
                (Key::P, true) => {
                    self.state.borrow_mut().push(Command::SetWorkspace(Workspace::Scene));
                    Some(Command::Play)
                }
                (Key::Escape, false) => {
                    self.state.borrow_mut().anim.selection = AnimSelection::None;
                    return true;
                }
                _ => None,
            };
            return match command {
                Some(command) => {
                    self.state.borrow_mut().push(command);
                    true
                }
                None => false,
            };
        }
        let command = match (key, ctrl, shift) {
            (Key::R, true, _) => Some(Command::ReloadScripts),
            (Key::M, true, true) => Some(Command::MemorySnapshot),
            (Key::P, true, _) => Some(match mode {
                Mode::Edit => Command::Play,
                Mode::Play => Command::Stop,
            }),
            _ if mode == Mode::Play => None,
            (Key::S, true, true) => Some(Command::SaveAsDialog),
            (Key::S, true, false) => Some(Command::Save),
            (Key::O, true, _) => Some(Command::ConfirmDiscard(Box::new(Command::OpenDialog))),
            (Key::N, true, _) => Some(Command::ConfirmDiscard(Box::new(Command::NewScene))),
            (Key::D, true, _) => selected.map(Command::Duplicate),
            (Key::Delete, false, _) => selected.map(Command::Delete),
            (Key::F, false, _) => selected.map(Command::Focus),
            (Key::W, false, _) => {
                self.state.borrow_mut().gizmo = GizmoMode::Translate;
                return true;
            }
            (Key::E, false, _) => {
                self.state.borrow_mut().gizmo = GizmoMode::Rotate;
                return true;
            }
            (Key::R, false, _) => {
                self.state.borrow_mut().gizmo = GizmoMode::Scale;
                return true;
            }
            (Key::Escape, false, _) => {
                self.state.borrow().select(None);
                return true;
            }
            _ => None,
        };
        match command {
            Some(command) => {
                self.state.borrow_mut().push(command);
                true
            }
            None => false,
        }
    }
}
