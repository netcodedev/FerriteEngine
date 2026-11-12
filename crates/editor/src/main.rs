mod editor;

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use cgmath::{EuclideanSpace, Point3, Vector3};
use editor::{
    anim::{
        canvas::NodeCanvas, graph_inspector::GraphInspector, inputs::InputsPanel,
        preview::PreviewPanel, AnimSelection,
    },
    dialogs::{self, ModalHost},
    hierarchy::HierarchyPanel,
    inspector::InspectorPanel,
    layout::{Layout, RightColumn},
    outline::Outline,
    showcase,
    splitter::Splitter,
    stats::{GpuTimer, StatsPanel},
    titlebar::TitleBar,
    toggle_window::ToggleWindow,
    toolbar::Toolbar,
    viewport::{Viewport, ViewportAction},
    Command, EditorState, EntityTemplate, GizmoMode, Mode, Shared, Workspace,
};
use ferrite::core::{
    application::{Application, Layer},
    entity::{
        component::{animation_component::AnimationComponent, camera_component::CameraComponent},
        Entity, EntityHandle,
    },
    renderer::line::LineRenderer,
    scene::{
        scene_file::{default_scene_file, ComponentData, EntityData, SceneFile, SceneSettings},
        Scene,
    },
    window::Window,
};
use ferrite::player::PLAYER_GRAPH_PATH;
use ferrite_ui::ui::{UIElement, UI};
use glfw::{Action, CursorMode, Glfw, Key, Modifiers, WindowEvent};

const STARTUP_SCENE: &str = "scenes/default.ron";
/// Terrain collider triangles are drawn only this close to the camera.
const COLLIDER_MESH_DISTANCE: f32 = 96.0;

fn main() {
    // The editor draws its own title bar.
    let mut application = Application::with_decorations(1280, 720, "Ferrite Editor", false);
    application.set_clear_color(ferrite_ui::ui::theme::BG_APP);
    application.add_layer(Box::new(EditorLayer::new()));
    application.start();
}

struct EditorLayer {
    scene: Scene,
    ui: UI,
    state: Shared,
    viewport: Viewport,
    outline: Outline,
    gpu_timer: Option<GpuTimer>,
    frame_count: u64,
    /// Scene as it was when play mode started; restored on stop.
    play_snapshot: Option<SceneFile>,
    /// Cursor mode to apply on the next event (commands run in `on_update`
    /// have no access to the GLFW window).
    pending_cursor: Option<CursorMode>,
    /// Set by `Command::Quit`; the window is closed on the next event.
    pending_quit: bool,
    cursor_initialised: bool,
}

impl EditorLayer {
    fn new() -> Self {
        let state = EditorState::new();
        let (scene, path) = Self::startup_scene(&state);
        state.borrow_mut().scene_path = path;

        let mut ui = UI::new();
        let add = |ui: &mut UI, mut element: Box<dyn UIElement>| {
            element.set_z_index(0.0);
            ui.add(element);
        };
        add(&mut ui, Box::new(TitleBar::new(state.clone())));
        add(&mut ui, Box::new(Toolbar::new(state.clone())));
        // Before the panels, so it gets first claim on clicks at the edges.
        add(&mut ui, Box::new(Splitter::new(state.clone())));
        let (scene_panels, anim_panels) = {
            let s = state.borrow();
            (s.scene_panels.clone(), s.anim_panels.clone())
        };
        // Scene workspace.
        for panel in [
            Box::new(HierarchyPanel::new(state.clone())) as Box<dyn UIElement>,
            Box::new(InspectorPanel::new(state.clone())),
            Box::new(StatsPanel::new(state.clone())),
        ] {
            add(&mut ui, Box::new(ToggleWindow::new(scene_panels.clone(), panel)));
        }
        // Animation workspace.
        for panel in [
            Box::new(NodeCanvas::new(state.clone())) as Box<dyn UIElement>,
            Box::new(InputsPanel::new(state.clone())),
            Box::new(GraphInspector::new(state.clone())),
            Box::new(PreviewPanel::new(state.clone())),
        ] {
            add(&mut ui, Box::new(ToggleWindow::new(anim_panels.clone(), panel)));
        }

        // Floating windows from the Window menu (hidden by default).
        let (widgets, widgets_popup) = showcase::widget_showcase();
        let show_widgets = state.borrow().show_widgets.clone();
        add(&mut ui, Box::new(ToggleWindow::new(show_widgets.clone(), widgets)));
        add(&mut ui, Box::new(ToggleWindow::new(show_widgets, widgets_popup)));
        add(&mut ui, Box::new(ModalHost::new(state.clone())));

        Self {
            scene,
            ui,
            state,
            viewport: Viewport::new(),
            outline: Outline::new(),
            gpu_timer: None,
            frame_count: 0,
            play_snapshot: None,
            pending_cursor: None,
            pending_quit: false,
            cursor_initialised: false,
        }
    }

    /// `scenes/default.ron` if it exists, otherwise the built-in default
    /// scene (camera, sky, terrain with player, debug controller).
    fn startup_scene(state: &Shared) -> (Scene, Option<PathBuf>) {
        let path = Path::new(STARTUP_SCENE);
        if path.exists() {
            match Scene::load_from_path(path) {
                Ok(scene) => return (scene, Some(path.to_path_buf())),
                Err(e) => state
                    .borrow_mut()
                    .set_status(format!("Could not load {STARTUP_SCENE}: {e}")),
            }
        }
        let scene = Scene::from_file(&default_scene_file()).unwrap_or_else(|e| {
            state
                .borrow_mut()
                .set_status(format!("Default scene failed: {e}"));
            Scene::new()
        });
        (scene, None)
    }

    /// Camera + sky light, like a fresh scene in most editors.
    fn new_scene_file(&self) -> SceneFile {
        let (position, yaw, pitch) = self.viewport.camera_pose();
        let entity = |name: &str, components| EntityData {
            id: u64::from(EntityHandle::new()),
            name: name.into(),
            position: [0.0; 3],
            rotation: [1.0, 0.0, 0.0, 0.0],
            components,
            children: Vec::new(),
        };
        SceneFile {
            version: ferrite::core::scene::scene_file::SCENE_FILE_VERSION,
            settings: SceneSettings {
                shadow_map_size: Some((4096, 4096)),
                debug_map_size: Some((1280, 720)),
            },
            entities: vec![
                entity(
                    "camera",
                    vec![ComponentData::Camera {
                        position: [position.x, position.y, position.z],
                        yaw_deg: yaw.0,
                        pitch_deg: pitch.0,
                        fov_deg: 64.0,
                        near: 0.1,
                        far: 1000.0,
                        speed: 10.0,
                    }],
                ),
                entity("sky light", vec![ComponentData::default_sky_light()]),
            ],
        }
    }

    fn template_data(&self, template: EntityTemplate) -> EntityData {
        let spawn = self.viewport.spawn_point();
        let (camera_position, yaw, pitch) = self.viewport.camera_pose();
        let (name, position, components) = match template {
            EntityTemplate::Empty => ("entity", spawn, vec![]),
            EntityTemplate::Camera => (
                "camera",
                Point3::origin(),
                vec![ComponentData::Camera {
                    position: [camera_position.x, camera_position.y, camera_position.z],
                    yaw_deg: yaw.0,
                    pitch_deg: pitch.0,
                    fov_deg: 64.0,
                    near: 0.1,
                    far: 1000.0,
                    speed: 10.0,
                }],
            ),
            EntityTemplate::SkyLight => (
                "sky light",
                Point3::origin(),
                vec![ComponentData::default_sky_light()],
            ),
            EntityTemplate::Terrain => (
                "terrain",
                Point3::origin(),
                vec![ComponentData::Terrain { seed: 2 }],
            ),
            EntityTemplate::Player => ("player", spawn, vec![ComponentData::Player]),
        };
        EntityData {
            id: u64::from(EntityHandle::new()),
            name: name.into(),
            position: [position.x, position.y, position.z],
            rotation: [1.0, 0.0, 0.0, 0.0],
            components,
            children: Vec::new(),
        }
    }

    fn replace_scene(&mut self, scene: Scene) {
        self.scene = scene;
        // The viewport re-installs its camera on the next update.
    }

    fn run_commands(&mut self) {
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
                    let mut state = self.state.borrow_mut();
                    state.set_status(format!("Opened {}", path.display()));
                    state.scene_path = Some(path);
                    state.dirty = false;
                    state.select(None);
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
                if self.state.borrow().workspace == Workspace::Animation {
                    self.run(Command::SetWorkspace(Workspace::Scene));
                }
                self.play_snapshot = Some(self.scene.to_file());
                self.scene.set_editor_camera(None);
                self.scene.set_physics_enabled(true);
                self.state.borrow_mut().mode = Mode::Play;
                self.pending_cursor = Some(CursorMode::Disabled);
            }
            Command::Stop => {
                if mode == Mode::Edit {
                    return;
                }
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

    fn save(&mut self, path: PathBuf) {
        match self.scene.save_to_path(&path) {
            Ok(()) => {
                let mut state = self.state.borrow_mut();
                state.set_status(format!("Saved {}", path.display()));
                state.scene_path = Some(path);
                state.dirty = false;
            }
            Err(e) => self
                .state
                .borrow_mut()
                .set_status(format!("Could not save {}: {e}", path.display())),
        }
    }

    /// Keyboard shortcuts that are not handled by a focused widget.
    fn shortcut(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let ctrl = modifiers.contains(Modifiers::Control);
        let shift = modifiers.contains(Modifiers::Shift);
        let mode = self.state.borrow().mode;
        let selected = self.state.borrow().selected();
        let workspace = self.state.borrow().workspace;
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

    /// Wireframes for every collider whose "show collider" toggle is on.
    /// Primitive shapes draw on top (cyan), terrain meshes depth-tested
    /// (green) and only near the camera.
    fn render_colliders(&self) {
        let Some(camera) = self.scene.get_active_camera() else {
            return;
        };
        let vp = camera.get_view_projection();
        let eye = camera.get_camera().get_eye_position();
        let (shapes, meshes) = self.scene.visible_collider_lines(eye, COLLIDER_MESH_DISTANCE);
        if !meshes.is_empty() {
            LineRenderer::render_lines(&vp, &meshes, Vector3::new(0.2, 0.85, 0.3), false);
        }
        if !shapes.is_empty() {
            LineRenderer::render_lines(&vp, &shapes, Vector3::new(0.0, 0.9, 1.0), true);
        }
    }

    fn apply_viewport_aspect(&mut self, layout: &Layout) {
        let aspect = layout.viewport.w / layout.viewport.h.max(1.0);
        if let Some(camera) = self.scene.get_component_mut::<CameraComponent>() {
            camera.get_projection_mut().set_aspect(aspect);
        }
    }
}

impl Layer for EditorLayer {
    fn on_update(&mut self, window: &Window, delta_time: f64) {
        let (show_stats, workspace) = {
            let s = self.state.borrow();
            (s.show_stats.read(), s.workspace)
        };
        let right = match (workspace, show_stats) {
            (Workspace::Animation, _) => RightColumn::Preview,
            (Workspace::Scene, true) => RightColumn::Stats,
            (Workspace::Scene, false) => RightColumn::None,
        };
        let sizes = self.state.borrow().panel_sizes;
        let layout = Layout::compute(window.width as f32, window.height as f32, right, &sizes);
        self.state.borrow_mut().layout = layout;
        let mode = self.state.borrow().mode;

        // Drop a selection whose entity no longer exists.
        let selected = self.state.borrow().selected();
        if let Some(id) = selected {
            if self.scene.get_entity(&id).is_none() {
                self.state.borrow().select(None);
            }
        }

        let update_start = Instant::now();
        match mode {
            Mode::Edit => {
                self.scene.set_physics_enabled(false);
                self.viewport.update(&mut self.scene, layout.viewport, delta_time);
                self.scene.update_in_editor(delta_time);
            }
            Mode::Play => {
                self.scene.update(delta_time);
            }
        }
        self.apply_viewport_aspect(&layout);
        let update_ms = update_start.elapsed().as_secs_f32() * 1000.0;

        // Scene into the docked viewport, then gizmos on top of it.
        let render_start = Instant::now();
        let gpu_timer = self.gpu_timer.get_or_insert_with(GpuTimer::new);
        gpu_timer.begin();
        let scene_visible = workspace == Workspace::Scene;
        if scene_visible {
            self.scene.set_viewport(Some(layout.viewport_gl()));
            self.scene.render(window);
            self.render_colliders();
        }
        if mode == Mode::Edit && scene_visible {
            let (selected, gizmo) = {
                let state = self.state.borrow();
                (state.selected(), state.gizmo)
            };
            if let Some(id) = selected {
                self.outline
                    .render(&self.scene, id, layout.viewport_gl());
            }
            self.viewport
                .render(&self.scene, layout.viewport, selected, gizmo);
        }
        let gpu_timer = self.gpu_timer.as_mut().expect("created above");
        gpu_timer.end();
        let gpu_ms = gpu_timer.last_ms();
        let render_ms = render_start.elapsed().as_secs_f32() * 1000.0;
        window.reset_viewport();
        // UI depth must not be tested against the 3D scene.
        window.clear_mask(gl::DEPTH_BUFFER_BIT);

        let ui_start = Instant::now();
        self.ui.update(&mut self.scene);
        self.run_commands();
        self.ui.render();
        let ui_ms = ui_start.elapsed().as_secs_f32() * 1000.0;

        self.capture_if_requested(window);

        let mut state = self.state.borrow_mut();
        state.stats.record_frame(delta_time);
        state
            .stats
            .record_phases(update_ms, render_ms, ui_ms, gpu_ms);
        state.stats.refresh_slow(&self.scene);
    }

    fn on_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        if !self.cursor_initialised {
            window.set_cursor_mode(CursorMode::Normal);
            self.cursor_initialised = true;
        }
        self.apply_pending_cursor(window);

        let mode = self.state.borrow().mode;
        let layout = self.state.borrow().layout;

        // While flying or dragging, the viewport owns the mouse and keys.
        let viewport_busy =
            mode == Mode::Edit && (self.viewport.is_flying() || self.viewport.is_dragging());
        if !viewport_busy && self.ui.handle_events(&mut self.scene, window, glfw, event) {
            self.run_commands();
            self.apply_pending_cursor(window);
            return;
        }

        if let WindowEvent::Key(key, _, Action::Press, modifiers) = event {
            if !self.viewport.is_flying() && self.shortcut(*key, *modifiers) {
                self.run_commands();
                self.apply_pending_cursor(window);
                return;
            }
        }

        if self.state.borrow().workspace == Workspace::Animation {
            return;
        }
        match mode {
            Mode::Edit => {
                let (selected, gizmo) = {
                    let state = self.state.borrow();
                    (state.selected(), state.gizmo)
                };
                match self.viewport.handle_event(
                    &mut self.scene,
                    window,
                    event,
                    layout.viewport,
                    selected,
                    gizmo,
                ) {
                    ViewportAction::Select(id) => self.state.borrow().select(id),
                    ViewportAction::Edited => self.state.borrow_mut().dirty = true,
                    ViewportAction::Consumed | ViewportAction::None => {}
                }
                // Resize events still reach cameras and render targets.
                if let WindowEvent::FramebufferSize(..) = event {
                    self.scene.handle_event(glfw, window, event);
                }
            }
            Mode::Play => self.scene.handle_event(glfw, window, event),
        }
    }

    fn get_name(&self) -> &str {
        "Editor"
    }
}

impl EditorLayer {
    /// Development aid: with `FERRITE_CAPTURE=<file.png>` (and optionally
    /// `FERRITE_CAPTURE_FRAME=<n>`, default 300) the editor saves its own
    /// framebuffer once after that many frames.
    fn capture_if_requested(&mut self, window: &Window) {
        self.frame_count += 1;
        let Ok(path) = std::env::var("FERRITE_CAPTURE") else {
            return;
        };
        let frame = std::env::var("FERRITE_CAPTURE_FRAME")
            .ok()
            .and_then(|f| f.parse().ok())
            .unwrap_or(300u64);
        if self.frame_count == frame {
            match window.save_screenshot(Path::new(&path)) {
                Ok(()) => eprintln!("captured {path}"),
                Err(e) => eprintln!("capture failed: {e}"),
            }
        }
    }

    fn apply_pending_cursor(&mut self, window: &mut glfw::Window) {
        if let Some(mode) = self.pending_cursor.take() {
            self.viewport.reset_interaction(window);
            window.set_cursor_mode(mode);
        }
        if self.pending_quit {
            window.set_should_close(true);
        }
    }
}

/// Reload `path` into every animation component that uses it, so saved
/// graph edits show up without reloading the scene.
fn apply_saved_graph(entities: &mut [Entity], path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let mut count = 0;
    for entity in entities.iter_mut() {
        if let Some(animation) = entity.get_component_mut::<AnimationComponent>() {
            if animation.graph_path() == Some(path) {
                animation.load_graph(path)?;
                count += 1;
            }
        }
        count += apply_saved_graph(entity.get_children_mut(), path)?;
    }
    Ok(count)
}
