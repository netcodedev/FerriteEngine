//! The editor itself, once a project is open: its scene, panels and
//! viewport, and the frame loop that drives them.

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use cgmath::{EuclideanSpace, Point3, Vector3};
use ferrite::core::{
    application::Layer,
    assets,
    entity::{
        component::{animation_component::AnimationComponent, camera_component::CameraComponent},
        Entity, EntityHandle,
    },
    mcp_bridge,
    project::Project,
    renderer::line::LineRenderer,
    scene::{
        scene_file::{ComponentData, EntityData, SceneFile, SceneSettings},
        Scene,
    },
    window::Window,
};
use ferrite::terrain::{dual_contouring::DualContouringChunk, Terrain};
use ferrite_ui::ui::{UIElement, UI};
use glfw::{Action, CursorMode, Glfw, WindowEvent};

use crate::editor::{
    anim::{
        canvas::NodeCanvas, graph_inspector::GraphInspector, inputs::InputsPanel,
        preview::PreviewPanel,
    },
    asset_browser::AssetBrowser,
    asset_editor::{AssetInspector, AssetPreview},
    bridge_handler::{self, Dispatch},
    dialogs::{self, ModalHost},
    drag::{DragLayer, DropAction, DropTarget},
    export::Exporter,
    hierarchy::HierarchyPanel,
    inspector::InspectorPanel,
    layout::{Layout, RightColumn},
    outline::Outline,
    project_settings::EditorSettings,
    scripts::{ScriptEvent, Scripts},
    showcase,
    splitter::Splitter,
    stats::{GpuTimer, StatsPanel},
    titlebar::TitleBar,
    toggle_window::ToggleWindow,
    toolbar::Toolbar,
    viewport::{Viewport, ViewportAction},
    Command, EditorState, EntityTemplate, Mode, Shared, Workspace,
};
use crate::{loading::Prepared, project_lock};

mod commands;

/// How often the editor's arrangement is checked and saved to the project.
const SETTINGS_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// Terrain collider triangles are drawn only this close to the camera.
const COLLIDER_MESH_DISTANCE: f32 = 96.0;

pub(crate) struct EditorLayer {
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
    /// Bridge screenshot requests waiting on the next rendered frame.
    pending_screenshots: Vec<(mcp_bridge::PendingRequest, bridge_handler::Shot)>,
    cursor_initialised: bool,
    scripts: Scripts,
    exporter: Exporter,
    project: Project,
    /// The arrangement last saved to the project file.
    saved_settings: EditorSettings,
    settings_checked: Instant,
    /// Window size and state, as last seen by an event (the window is only
    /// reachable there).
    window_state: Option<((u32, u32), bool)>,
    /// The project's window size has been applied to the window.
    window_restored: bool,
}

impl EditorLayer {
    /// The editor for `project`, which `loading` prepared (entered it,
    /// registered its assets, built its scripts).
    pub(super) fn new(project: Project, prepared: Prepared) -> Self {
        let state = EditorState::new();
        let settings = EditorSettings::from_value(&project.file.editor);
        {
            let mut state = state.borrow_mut();
            settings.apply(&mut state);
            state.project_name = project.name().to_string();
        }
        for problem in &prepared.asset_problems {
            log::warn!("assets: {problem}");
        }
        let registered = prepared.registered;
        // The project's scripts, before the first scene is built from them.
        let mut scripts = Scripts::new(project.scripts());
        let loaded = prepared.scripts.and_then(|()| match scripts.load_built() {
            ScriptEvent::Failed(e) => Err(e),
            _ => Ok(()),
        });
        if let Err(e) = loaded {
            state.borrow_mut().set_status(format!("Scripts: {e}"));
        }
        let (scene, path) = Self::startup_scene(&state, &project);
        Self::report_scene_problems(&state, &scene);
        if registered > 0 && state.borrow().status().is_none() {
            state
                .borrow_mut()
                .set_status(format!("Registered {registered} new assets"));
        }
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
        // Also ahead of the panels: it has to see the release that ends a
        // drag before the panel under the cursor does.
        add(&mut ui, Box::new(DragLayer::new(state.clone())));
        let (scene_panels, anim_panels, asset_panels, browser_panel) = {
            let s = state.borrow();
            (
                s.scene_panels.clone(),
                s.anim_panels.clone(),
                s.asset_panels.clone(),
                s.browser_panel.clone(),
            )
        };
        // Scene workspace.
        for panel in [
            Box::new(HierarchyPanel::new(state.clone())) as Box<dyn UIElement>,
            Box::new(InspectorPanel::new(state.clone())),
            Box::new(StatsPanel::new(state.clone())),
        ] {
            add(&mut ui, Box::new(ToggleWindow::new(scene_panels.clone(), panel)));
        }
        // Asset workspace.
        for panel in [
            Box::new(AssetInspector::new(state.clone())) as Box<dyn UIElement>,
            Box::new(AssetPreview::new(state.clone())),
        ] {
            add(&mut ui, Box::new(ToggleWindow::new(asset_panels.clone(), panel)));
        }
        // Shared by the scene and the asset workspace.
        add(
            &mut ui,
            Box::new(ToggleWindow::new(
                browser_panel,
                Box::new(AssetBrowser::new(state.clone())),
            )),
        );
        // Animation workspace.
        for panel in [
            Box::new(NodeCanvas::new(state.clone())) as Box<dyn UIElement>,
            Box::new(InputsPanel::new(state.clone())),
            Box::new(GraphInspector::new(state.clone())),
            Box::new(PreviewPanel::new(state.clone())),
        ] {
            add(&mut ui, Box::new(ToggleWindow::new(anim_panels.clone(), panel)));
        }

        // Widget showcase, docked in the right column while enabled in the
        // Window menu (it hides itself when the layout gives it no room).
        let (widgets, widgets_popup) = showcase::widget_showcase(state.clone());
        let show_widgets = state.borrow().show_widgets.clone();
        add(&mut ui, widgets);
        add(&mut ui, Box::new(ToggleWindow::new(show_widgets, widgets_popup)));
        add(&mut ui, Box::new(ModalHost::new(state.clone())));

        // MCP bridge: let an external AI tool see and change this editor.
        match mcp_bridge::start(project.dir()) {
            Ok(port) => log::info!("MCP bridge listening on 127.0.0.1:{port}"),
            Err(e) => log::warn!("MCP bridge: {e}"),
        }

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
            pending_screenshots: Vec::new(),
            cursor_initialised: false,
            scripts,
            exporter: Exporter::default(),
            project,
            saved_settings: settings,
            settings_checked: Instant::now(),
            window_state: None,
            window_restored: false,
        }
    }

    /// Keep the project file up to date with how the editor is arranged.
    fn save_settings_if_changed(&mut self) {
        if self.settings_checked.elapsed() < SETTINGS_SAVE_INTERVAL {
            return;
        }
        self.settings_checked = Instant::now();
        let (window, maximized) = match self.window_state {
            // A maximized window keeps the size it had before.
            Some((_, true)) => (self.saved_settings.window, true),
            Some((size, false)) => (Some(size), false),
            None => (self.saved_settings.window, self.saved_settings.maximized),
        };
        let settings = EditorSettings::capture(&self.state.borrow(), window, maximized);
        if settings != self.saved_settings {
            self.project.file.editor = settings.to_value();
            if let Err(e) = self.project.save() {
                log::warn!("could not save the project: {e}");
            }
            self.saved_settings = settings;
        }
    }

    /// The project opens with `path` from now on.
    fn remember_scene(&mut self, path: &Path) {
        let relative = path.strip_prefix(self.project.dir()).unwrap_or(path);
        let relative = relative.to_string_lossy().replace('\\', "/");
        if self.project.file.startup_scene.as_deref() != Some(relative.as_str()) {
            self.project.file.startup_scene = Some(relative);
            if let Err(e) = self.project.save() {
                log::warn!("could not save the project: {e}");
            }
        }
    }

    fn on_script_event(&mut self, event: ScriptEvent) {
        let message = match event {
            ScriptEvent::Building => "Building scripts...".to_string(),
            ScriptEvent::Failed(e) => format!("Scripts: {e}"),
            ScriptEvent::Loaded { count } => {
                let problems = self.scene.rebuild_scripts();
                // The scene the editor returns to after play mode is rebuilt
                // from data on stop anyway.
                match problems.first() {
                    Some(first) => format!("Scripts reloaded, but: {first}"),
                    None => format!("Scripts reloaded ({count})"),
                }
            }
        };
        let mut state = self.state.borrow_mut();
        state.scripts_message = Some(message.clone());
        state.set_status(message);
    }

    /// The project's startup scene, or an empty one.
    fn startup_scene(state: &Shared, project: &Project) -> (Scene, Option<PathBuf>) {
        let Some(path) = &project.file.startup_scene else {
            return (Scene::new(), None);
        };
        let path = PathBuf::from(path);
        match Scene::load_from_path(&path) {
            Ok(scene) => (scene, Some(path)),
            Err(e) => {
                state
                    .borrow_mut()
                    .set_status(format!("Could not load {}: {e}", path.display()));
                (Scene::new(), None)
            }
        }
    }

    /// Tell the user about components that could not be built on load.
    fn report_scene_problems(state: &Shared, scene: &Scene) {
        let problems = scene.problems();
        if let Some(first) = problems.first() {
            let more = match problems.len() - 1 {
                0 => String::new(),
                n => format!(" (and {n} more)"),
            };
            state
                .borrow_mut()
                .set_status(format!("Not loaded: {first}{more}"));
        }
    }

    /// Camera + sky light, like a fresh scene in most editors.
    fn new_scene_file(&self) -> SceneFile {
        let (position, yaw, pitch) = self.viewport.camera_pose();
        let entity = |name: &str, components| EntityData {
            id: u64::from(EntityHandle::new()),
            name: name.into(),
            position: [0.0; 3],
            rotation: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0; 3],
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
                        free: true,
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
                    free: true,
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
                vec![ComponentData::bare_terrain(2)],
            ),
            EntityTemplate::Player => ("player", spawn, vec![ComponentData::Player]),
        };
        EntityData {
            id: u64::from(EntityHandle::new()),
            name: name.into(),
            position: [position.x, position.y, position.z],
            rotation: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0; 3],
            components,
            children: Vec::new(),
        }
    }

    fn terrain_mut(&mut self, id: &EntityHandle) -> Option<&mut Terrain<DualContouringChunk>> {
        self.scene
            .get_entity_mut(id)?
            .get_component_mut::<Terrain<DualContouringChunk>>()
    }

    /// Ask for files and import each as an asset.
    fn import_assets(&mut self) {
        let files = dialogs::pick_import_files();
        if files.is_empty() {
            return;
        }
        let mut imported = Vec::new();
        let mut failed = Vec::new();
        {
            let mut database = assets::database();
            for file in &files {
                match database.import(file) {
                    Ok(id) => {
                        if let Some(entry) = database.get(id) {
                            let name = match entry.meta.as_model() {
                                Some(model) if model.scale != 1.0 => format!(
                                    "{} (scale {} from the file's unit)",
                                    entry.meta.name, model.scale
                                ),
                                _ => entry.meta.name.clone(),
                            };
                            imported.push(name);
                        }
                    }
                    Err(e) => failed.push(e.to_string()),
                }
            }
        }
        let message = match (imported.as_slice(), failed.first()) {
            (_, Some(error)) if imported.is_empty() => format!("Import failed: {error}"),
            (names, Some(error)) => {
                format!("Imported {}, but: {error}", names.join(", "))
            }
            (names, None) => format!("Imported {}", names.join(", ")),
        };
        self.state.borrow_mut().set_status(message);
    }

    fn replace_scene(&mut self, scene: Scene) {
        self.scene = scene;
        // The viewport re-installs its camera on the next update.
    }

    /// Drain pending MCP bridge requests and answer them against the
    /// live scene. Screenshots go on a short queue; `service_pending_screenshots`
    /// finishes them after the frame is drawn.
    fn process_bridge_requests(&mut self) {
        for pending in mcp_bridge::drain_pending() {
            let request = pending.request.clone();
            let mut state = self.state.borrow_mut();
            let dispatch = bridge_handler::dispatch(
                request,
                pending,
                bridge_handler::Context {
                    scene: &mut self.scene,
                    state: &mut state,
                    viewport: &mut self.viewport,
                },
            );
            drop(state);
            match dispatch {
                Dispatch::Done => {}
                Dispatch::Screenshot(pending, shot) => {
                    self.pending_screenshots.push((pending, shot));
                }
            }
        }
        // Apply any Play/Stop/Save commands bridge requests queued.
        self.run_commands();
    }

    /// Finish any screenshot requests on the queue by reading the
    /// framebuffer. Runs at the end of `on_update`, so the frame has
    /// already been drawn.
    fn service_pending_screenshots(&mut self, window: &Window) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let viewport = self.state.borrow().layout.viewport_gl();
        let full_width = window.width.max(1);
        let full_height = window.height.max(1);
        for (pending, shot) in std::mem::take(&mut self.pending_screenshots) {
            let (x, y, w, h) = if shot.source == 1 {
                (0, 0, full_width as i32, full_height as i32)
            } else {
                (viewport[0], viewport[1], viewport[2], viewport[3])
            };
            let response = capture_viewport(x, y, w.max(1), h.max(1), &shot);
            pending.respond(response);
        }
    }

    fn save(&mut self, path: PathBuf) {
        match self.scene.save_to_path(&path) {
            Ok(()) => {
                self.remember_scene(&path);
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

impl Drop for EditorLayer {
    /// Take the bridge's lockfile away, so tools do not look for an editor
    /// that is gone, and unlock the project for other editors.
    fn drop(&mut self) {
        mcp_bridge::stop();
        project_lock::release();
    }
}

impl Layer for EditorLayer {
    fn on_update(&mut self, window: &Window, delta_time: f64) {
        if let Some(event) = self.scripts.poll() {
            self.on_script_event(event);
        }
        self.process_bridge_requests();
        if let Some(status) = self.exporter.poll() {
            self.state.borrow_mut().set_status(status);
        }
        // The game asked to end: in the editor that ends play mode, never
        // the editor.
        if ferrite::core::application::take_quit_request()
            && self.state.borrow().mode == Mode::Play
        {
            self.state.borrow_mut().push(Command::Stop);
            self.run_commands();
        }
        // The game asked to start over: play again from the scene as it
        // was when play mode started.
        if ferrite::core::application::take_restart_request()
            && self.state.borrow().mode == Mode::Play
        {
            self.state.borrow_mut().push(Command::Stop);
            self.state.borrow_mut().push(Command::Play);
            self.run_commands();
        }
        self.save_settings_if_changed();
        let (show_stats, show_widgets, show_assets, workspace) = {
            let s = self.state.borrow();
            (
                s.show_stats.read(),
                s.show_widgets.read(),
                s.show_assets.read(),
                s.workspace,
            )
        };
        let right = match workspace {
            Workspace::Animation => RightColumn::Preview,
            Workspace::Scene => RightColumn::Panels {
                stats: show_stats,
                widgets: show_widgets,
            },
            // Settings on the left, the preview takes the rest.
            Workspace::Asset => RightColumn::Panels {
                stats: false,
                widgets: false,
            },
        };
        let sizes = self.state.borrow().panel_sizes;
        let layout = Layout::compute(
            window.width as f32,
            window.height as f32,
            right,
            show_assets && workspace != Workspace::Animation,
            &sizes,
        );
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
        let gpu_timer = self.gpu_timer.get_or_insert_with(|| {
            // Time each of the scene's passes for the statistics, too.
            ferrite::core::renderer::profiler::set_enabled(true);
            GpuTimer::new()
        });
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
        // A model asset dropped on the viewport becomes an entity. Panels
        // registered their (smaller) targets during the update.
        if workspace == Workspace::Scene && mode == Mode::Edit {
            self.state.borrow_mut().drop_targets.push(DropTarget {
                rect: layout.viewport,
                action: DropAction::SpawnModel,
            });
        }
        self.run_commands();
        self.ui.render();
        let ui_ms = ui_start.elapsed().as_secs_f32() * 1000.0;

        self.service_pending_screenshots(window);
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
        if !self.window_restored {
            self.window_restored = true;
            if let Some((width, height)) = self.saved_settings.window {
                window.set_size(width as i32, height as i32);
            }
            if self.saved_settings.maximized {
                window.maximize();
            }
        }
        let (width, height) = window.get_size();
        self.window_state = Some(((width.max(1) as u32, height.max(1) as u32), window.is_maximized()));
        self.apply_pending_cursor(window);

        let mode = self.state.borrow().mode;
        let layout = self.state.borrow().layout;

        // While flying or dragging, the viewport owns the mouse and keys.
        let viewport_busy =
            mode == Mode::Edit && (self.viewport.is_flying() || self.viewport.is_dragging());
        // While the game holds the mouse, its clicks belong to the game: the
        // hidden cursor still has a position, over whatever panel or button.
        let game_has_mouse = mode == Mode::Play
            && window.get_cursor_mode() == CursorMode::Disabled
            && matches!(
                event,
                WindowEvent::MouseButton(..) | WindowEvent::CursorPos(..) | WindowEvent::Scroll(..)
            );
        if !viewport_busy
            && !game_has_mouse
            && self.ui.handle_events(&mut self.scene, window, glfw, event)
        {
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

        if self.state.borrow().workspace != Workspace::Scene {
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

/// Read a sub-rectangle of the default framebuffer and encode it for the
/// bridge response, upright and scaled as `shot` asks.
fn capture_viewport(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    shot: &bridge_handler::Shot,
) -> mcp_bridge::BridgeResponse {
    use ferrite::core::mcp_bridge::BridgeResponse;
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    let mut buffer = vec![0u8; w * h * 3];
    unsafe {
        gl::PixelStorei(gl::PACK_ALIGNMENT, 1);
        gl::ReadBuffer(gl::BACK);
        gl::ReadPixels(
            x,
            y,
            width,
            height,
            gl::RGB,
            gl::UNSIGNED_BYTE,
            buffer.as_mut_ptr() as *mut _,
        );
    }
    // glReadPixels is bottom-left origin; flip rows so the PNG is upright.
    let row = w * 3;
    let mut flipped = vec![0u8; buffer.len()];
    for r in 0..h {
        let src = r * row;
        let dst = (h - 1 - r) * row;
        flipped[dst..dst + row].copy_from_slice(&buffer[src..src + row]);
    }
    match bridge_handler::encode_screenshot(flipped, w as u32, h as u32, shot.max_width, shot.format) {
        Ok((b64, width, height)) => BridgeResponse::data(serde_json::json!({
            "image_base64": b64,
            "mime_type": shot.format.mime(),
            "width": width,
            "height": height,
        })),
        Err(e) => BridgeResponse::err(e),
    }
}
