//! What the editor does for a `Command` from its panels, menus and
//! shortcuts.

use cgmath::{EuclideanSpace, Point3};
use ferrite::core::{
    assets,
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
use ferrite::terrain::scatter::DEFAULT_DENSITY;
use glfw::{CursorMode, Key, Modifiers};

use crate::editor::{
    anim::AnimSelection,
    asset_editor::AssetEdit,
    clip_editor::{self, ClipRequest},
    dialogs,
    export::Exporter,
    Command, GizmoMode, Mode, Workspace,
};
use crate::new_script::new_script_file;

use super::*;

impl EditorLayer {
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

    fn run(&mut self, command: Command) {
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
                .set_status("Stop play mode first (Ctrl+P)");
            return;
        }

        match command {
            Command::NewScene => match Scene::from_file(&self.new_scene_file()) {
                Ok(scene) => {
                    self.replace_scene(scene);
                    let mut state = self.state.borrow_mut();
                    state.scene_path = None;
                    state.dirty = false;
                    state.select(None);
                    state.set_status("New scene");
                }
                Err(e) => self.state.borrow_mut().set_status(format!("New scene failed: {e}")),
            },
            Command::OpenDialog => dialogs::open(&self.state),
            Command::Open(path) => match Scene::load_from_path(&path) {
                Ok(scene) => {
                    self.replace_scene(scene);
                    self.remember_scene(&path);
                    {
                        let mut state = self.state.borrow_mut();
                        state.set_status(format!("Opened {}", path.display()));
                        state.scene_path = Some(path);
                        state.dirty = false;
                        state.select(None);
                    }
                    Self::report_scene_problems(&self.state, &self.scene);
                }
                Err(e) => self
                    .state
                    .borrow_mut()
                    .set_status(format!("Could not open {}: {e}", path.display())),
            },
            Command::Save => {
                if mode == Mode::Play {
                    self.state
                        .borrow_mut()
                        .set_status("Stop play mode before saving");
                    return;
                }
                let path = self.state.borrow().scene_path.clone();
                match path {
                    Some(path) => self.save(path),
                    None => dialogs::save_as(&self.state),
                }
            }
            Command::SaveAsDialog => dialogs::save_as(&self.state),
            Command::SaveAs(path) => self.save(path),
            Command::ConfirmDiscard(then) => {
                let (scene_dirty, graph_dirty) = {
                    let s = self.state.borrow();
                    (s.dirty, s.anim.dirty && matches!(*then, Command::Quit))
                };
                if scene_dirty || graph_dirty {
                    dialogs::confirm_discard(&self.state, *then, graph_dirty);
                } else {
                    self.state.borrow_mut().push(*then);
                }
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
                self.scene.set_editor_camera(None);
                self.scene.set_physics_enabled(true);
                self.state.borrow_mut().mode = Mode::Play;
                self.pending_cursor = Some(CursorMode::Disabled);
                if self.scene.get_active_camera().is_none() {
                    self.state.borrow_mut().set_status(
                        "Nothing to see: the scene has no camera. Add a Camera component \
                         (with \"free fly\" on, it flies with WASD and the mouse).",
                    );
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
                        Ok(scene) => self.replace_scene(scene),
                        Err(e) => self
                            .state
                            .borrow_mut()
                            .set_status(format!("Restoring the scene failed: {e}")),
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
                        .set_status(format!("Could not create entity: {e}")),
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
            Command::ExportGame(platform) => {
                if self.exporter.is_running() {
                    self.state.borrow_mut().set_status("An export is already running");
                } else if let Some(out) = Exporter::pick_folder() {
                    self.exporter.start(self.project.clone(), out, platform);
                }
            }
            Command::RefreshAssets => {
                let (count, problems) = {
                    let mut database = assets::database();
                    let mut problems = database.scan();
                    // Files added by hand since the last look become assets.
                    problems.extend(database.register_new_files().1);
                    (database.len(), problems)
                };
                let mut state = self.state.borrow_mut();
                match problems.first() {
                    Some(problem) => state.set_status(format!("Assets: {problem}")),
                    None => state.set_status(format!("{count} assets")),
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
                if let Some(terrain) = self.terrain_mut(&entity) {
                    match layer {
                        Some(layer) => terrain.scatter_mut().set_asset(layer, asset),
                        None => terrain.scatter_mut().add_asset_layer(asset, DEFAULT_DENSITY),
                    }
                    self.state.borrow_mut().dirty = true;
                }
            }
            Command::PickModelAsset(entity) => dialogs::pick_entity_model(&self.state, entity),
            Command::PickScript(entity) => dialogs::pick_script(&self.state, entity),
            Command::NewScriptDialog => dialogs::new_script(&self.state),
            Command::NewScript(name) => {
                let message = match new_script_file(self.scripts.folder(), &name) {
                    Ok(file) => format!("Created scripts/{file}; it is built now"),
                    Err(e) => format!("Could not create the script: {e}"),
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
                    Err(e) => state.set_status(format!("Could not load the model: {e}")),
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
                        .set_status(format!("Could not create entity: {e}")),
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
                        .set_status(format!("Could not duplicate: {e}")),
                }
            }
            Command::Focus(id) => self.viewport.focus(&self.scene, id),
            Command::Quit => self.pending_quit = true,
            Command::SetWorkspace(workspace) => {
                if mode == Mode::Play && workspace == Workspace::Animation {
                    self.state
                        .borrow_mut()
                        .set_status("Stop play mode before editing animations");
                    return;
                }
                let mut state = self.state.borrow_mut();
                state.workspace = workspace;
                state.scene_panels.write(workspace == Workspace::Scene);
                state.anim_panels.write(workspace == Workspace::Animation);
                state.asset_panels.write(workspace == Workspace::Asset);
                state.browser_panel.write(workspace != Workspace::Animation);
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
                        .set_status(format!("Save or revert '{name}' first"));
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
                    Some(Ok(name)) => state.set_status(format!("Saved asset '{name}'")),
                    Some(Err(e)) => state.set_status(format!("Could not save the asset: {e}")),
                    None => {}
                }
            }
            Command::RevertAsset => {
                let mut state = self.state.borrow_mut();
                let result = state.asset_edit.as_mut().map(|edit| edit.revert());
                match result {
                    Some(Ok(())) => state.set_status("Asset reverted"),
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
                        .set_status(format!("Made the clip '{name}'"));
                    self.run(Command::EditAsset { id, variant: None });
                }
                Err(e) => self
                    .state
                    .borrow_mut()
                    .set_status(format!("Could not make the clip: {e}")),
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
            Command::PickAssetTexture { material, slot } => {
                dialogs::pick_asset_texture(&self.state, material, slot)
            }
            Command::SetAssetTexture {
                material,
                slot,
                texture,
            } => {
                if let Some(edit) = self.state.borrow_mut().asset_edit.as_mut() {
                    edit.set_texture(material, slot, texture);
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
                            Ok(0) => format!("Saved {}", path.display()),
                            Ok(n) => format!(
                                "Saved {} and applied it to {n} entit{}",
                                path.display(),
                                if n == 1 { "y" } else { "ies" }
                            ),
                            Err(e) => format!("Saved {}, but applying it failed: {e}", path.display()),
                        };
                        self.state.borrow_mut().set_status(message);
                    }
                    Err(e) => self
                        .state
                        .borrow_mut()
                        .set_status(format!("Could not save the animation graph: {e}")),
                }
            }
            Command::RevertGraph => {
                let result = self.state.borrow_mut().anim.reload();
                let mut state = self.state.borrow_mut();
                match result {
                    Ok(()) => state.set_status("Animation graph reverted"),
                    Err(e) => state.set_status(format!("Could not reload the graph: {e}")),
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
                            .set_status(format!("Could not open {path}: {e}"));
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
