use std::{f32::consts::TAU, time::Instant};

use ferrite::{
    core::{
        export::Platform,
        primitives::{Offset, Position, Size},
        renderer::{
            plane::{Plane, PlaneBuilder},
            text::{icons, Text},
        },
        scene::Scene,
    },
    terrain::Terrain,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        menu_button::{MenuButton, MenuItem},
    },
    theme, UIElement,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::{send, Command, GizmoMode, Mode, Shared, Workspace};

const BUTTON_HEIGHT: f32 = 26.0;
/// The spinner shown while the terrain is still loading: how many dots,
/// how large, on how large a circle.
const SPINNER_DOTS: usize = 8;
const SPINNER_DOT: f32 = 3.5;
const SPINNER_RADIUS: f32 = 6.5;

/// Top bar: File and Window menus, gizmo tools, play/stop and the latest
/// status message.
pub struct Toolbar {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    background: Plane,
    separator: Plane,
    file_menu: MenuButton,
    window_menu: MenuButton,
    move_button: Button,
    rotate_button: Button,
    scale_button: Button,
    play_button: Button,
    status_icon: Text,
    status: Text,
    /// Where the camera is (world x, y, z), beside the transform buttons.
    coords: Text,
    /// A spinner beside the status while the terrain is busy (planning the
    /// world, loading chunks), and since when it turns.
    spinner: Vec<Plane>,
    spinning: bool,
    started: Instant,
}

/// Window menu entries, checked while the panel is shown.
fn window_items(stats: bool, widgets: bool, assets: bool, console: bool) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Performance").icon(icons::ACTIVITY).checked(stats),
        MenuItem::new("Widget showcase").icon(icons::COMPONENT).checked(widgets),
        MenuItem::new("Assets").icon(icons::PACKAGE).checked(assets),
        MenuItem::new("Console").icon(icons::TERMINAL).checked(console),
    ]
}

fn tool_button(label: &str, icon: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, BUTTON_HEIGHT), on_click);
    button.set_style(ButtonStyle::Ghost);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(BUTTON_HEIGHT);
    button
}

impl Toolbar {
    pub fn new(state: Shared) -> Self {
        let file_state = state.clone();
        let file_menu = MenuButton::with_items(
            "File",
            None,
            vec![
                MenuItem::new("New scene").icon(icons::FILE_PLUS).shortcut("Ctrl+N"),
                MenuItem::new("Open...").icon(icons::FOLDER_OPEN).shortcut("Ctrl+O"),
                MenuItem::new("Save").icon(icons::SAVE).shortcut("Ctrl+S"),
                MenuItem::new("Save as...").icon(icons::SAVE).shortcut("Ctrl+Shift+S"),
                MenuItem::new("New script...").icon(icons::FILE_CODE),
                MenuItem::new("Reload scripts").icon(icons::ROTATE_CW).shortcut("Ctrl+R"),
                MenuItem::new("Export for Linux...").icon(icons::PACKAGE),
                MenuItem::new("Export for Windows...").icon(icons::PACKAGE),
            ],
            Box::new(move |index, _scene| {
                let command = match index {
                    0 => Command::ConfirmDiscard(Box::new(Command::NewScene)),
                    1 => Command::ConfirmDiscard(Box::new(Command::OpenDialog)),
                    2 => Command::Save,
                    3 => Command::SaveAsDialog,
                    4 => Command::NewScriptDialog,
                    5 => Command::ReloadScripts,
                    6 => Command::ExportGame(Platform::Linux),
                    _ => Command::ExportGame(Platform::Windows),
                };
                send(&file_state, command);
            }),
        );

        let window_state = state.clone();
        let window_menu = MenuButton::with_items(
            "Window",
            None,
            window_items(true, false, true, true),
            Box::new(move |index, _scene| {
                let state = window_state.borrow();
                let flag = match index {
                    0 => state.show_stats.clone(),
                    1 => state.show_widgets.clone(),
                    2 => state.show_assets.clone(),
                    _ => state.show_console.clone(),
                };
                flag.write(!flag.read());
            }),
        );

        let move_state = state.clone();
        let move_button = tool_button(
            "Move",
            icons::MOVE_3D,
            Box::new(move |_| move_state.borrow_mut().gizmo = GizmoMode::Translate),
        );
        let rotate_state = state.clone();
        let rotate_button = tool_button(
            "Rotate",
            icons::ROTATE_3D,
            Box::new(move |_| rotate_state.borrow_mut().gizmo = GizmoMode::Rotate),
        );
        let scale_state = state.clone();
        let scale_button = tool_button(
            "Scale",
            icons::SCALE_3D,
            Box::new(move |_| scale_state.borrow_mut().gizmo = GizmoMode::Scale),
        );

        let play_state = state.clone();
        let mut play_button = Button::new(
            Position::default(),
            Size::new(88.0, BUTTON_HEIGHT),
            Box::new(move |_| {
                let mode = play_state.borrow().mode;
                send(
                    &play_state,
                    match mode {
                        Mode::Edit => Command::Play,
                        Mode::Play => Command::Stop,
                    },
                );
            }),
        );
        play_button.set_style(ButtonStyle::Primary);
        play_button.set_icon(icons::PLAY);
        play_button.set_label("Play");

        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::new(1280.0, super::layout::TOOLBAR_HEIGHT),
            background: PlaneBuilder::new()
                .color(theme::BG_HEADER)
                .border_color(theme::BORDER)
                .border_thickness(1.0)
                .build(),
            separator: PlaneBuilder::new().color(theme::BORDER).build(),
            file_menu,
            window_menu,
            move_button,
            rotate_button,
            scale_button,
            play_button,
            status_icon: theme::icon(icons::INFO, 14.0, theme::TEXT_DIM),
            status: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_MUTED),
            coords: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_DIM),
            spinner: (0..SPINNER_DOTS)
                .map(|_| {
                    PlaneBuilder::new()
                        .size(Size::new(SPINNER_DOT, SPINNER_DOT))
                        .color(theme::ACCENT)
                        .border_radius_uniform(SPINNER_DOT / 2.0)
                        .build()
                })
                .collect(),
            spinning: false,
            started: Instant::now(),
        }
    }

    fn widgets_mut(&mut self) -> Vec<&mut dyn UIElement> {
        let scene = self.state.borrow().workspace == Workspace::Scene;
        let mut widgets: Vec<&mut dyn UIElement> = vec![
            &mut self.file_menu,
            &mut self.window_menu,
            &mut self.play_button,
        ];
        if scene {
            widgets.push(&mut self.move_button);
            widgets.push(&mut self.rotate_button);
            widgets.push(&mut self.scale_button);
        }
        widgets
    }

    fn widgets(&self) -> Vec<&dyn UIElement> {
        let scene = self.state.borrow().workspace == Workspace::Scene;
        let mut widgets: Vec<&dyn UIElement> = vec![
            &self.file_menu,
            &self.window_menu,
            &self.play_button,
        ];
        if scene {
            widgets.push(&self.move_button);
            widgets.push(&self.rotate_button);
            widgets.push(&self.scale_button);
        }
        widgets
    }
}

impl UIElement for Toolbar {
    fn update(&mut self, scene: &mut Scene) {
        let (stats_on, widgets_on, assets_on, console_on) = {
            let state = self.state.borrow();
            (
                state.show_stats.read(),
                state.show_widgets.read(),
                state.show_assets.read(),
                state.show_console.read(),
            )
        };
        // Rebuilds its entries only when a check mark actually changes.
        self.window_menu
            .set_menu_items(window_items(stats_on, widgets_on, assets_on, console_on));
        let (rect, viewport, mode, gizmo, status, workspace) = {
            let state = self.state.borrow();
            (
                state.layout.toolbar,
                state.layout.viewport,
                state.mode,
                state.gizmo,
                state.status().map(str::to_owned),
                state.workspace,
            )
        };
        self.size = Size::new(rect.w, rect.h);
        self.background
            .set_position(Position::new(rect.x - 1.0, rect.y, 0.0));
        self.background
            .set_size(Size::new(rect.w + 2.0, rect.h));

        let y = rect.y + ((rect.h - BUTTON_HEIGHT) * 0.5).round();
        let mut x = rect.x + 8.0;
        for widget in [
            &mut self.file_menu as &mut dyn UIElement,
            &mut self.window_menu,
        ] {
            widget.set_offset(Offset { x, y });
            x += widget.get_size().width + 2.0;
        }
        x += 8.0;
        self.separator
            .set_position(Position::new(x, rect.y + 8.0, 1.0));
        self.separator
            .set_size(Size::new(1.0, rect.h - 16.0));
        x += 9.0;
        if workspace == Workspace::Scene {
            for widget in [
                &mut self.move_button,
                &mut self.rotate_button,
                &mut self.scale_button,
            ] {
                widget.set_offset(Offset { x, y });
                x += widget.get_size().width + 4.0;
            }
        }
        // The camera's place, to the metre.
        let eye = scene.get_active_camera().map(|c| c.get_camera().get_eye_position());
        self.coords.set_content(&eye.map_or(String::new(), |p| format!("X {:.0}   Y {:.0}   Z {:.0}", p.x, p.y, p.z)));
        let cy = rect.y + theme::center_y(&self.coords, rect.h);
        self.coords.prepare_render_at(Position::new(x + 12.0, cy, 2.0));
        let coords_end = x + 12.0 + self.coords.measure_width();
        let editing = mode == Mode::Edit;
        self.move_button
            .set_active(gizmo == GizmoMode::Translate && editing);
        self.rotate_button
            .set_active(gizmo == GizmoMode::Rotate && editing);
        self.scale_button
            .set_active(gizmo == GizmoMode::Scale && editing);

        // Play/stop sits over the centre of the viewport.
        self.play_button.set_icon(if editing { icons::PLAY } else { icons::SQUARE });
        self.play_button.set_label(if editing { "Play" } else { "Stop" });
        self.play_button
            .set_style(if editing { ButtonStyle::Primary } else { ButtonStyle::Danger });
        self.play_button.set_active(!editing);
        let play_w = self.play_button.get_size().width;
        let play_x = (viewport.x + viewport.w * 0.5 - play_w * 0.5).max(coords_end + 16.0);
        self.play_button.set_offset(Offset { x: play_x, y });

        // What the terrain is still busy with, unless something else is
        // being said.
        let loading = scene
            .get_component::<Terrain>()
            .and_then(|terrain| terrain.loading_status());
        self.spinning = status.is_none() && loading.is_some();
        let status = status.or(loading).unwrap_or_else(|| match mode {
            Mode::Play => "Playing  -  Esc frees the mouse, Ctrl+P stops".into(),
            Mode::Edit => String::new(),
        });
        self.status.set_content(&status);
        let status_w = self.status.measure_width();
        let status_x = (rect.x + rect.w - status_w - 12.0).max(play_x + play_w + 36.0);
        let sy = rect.y + theme::center_y(&self.status, rect.h);
        self.status
            .prepare_render_at(Position::new(status_x, sy, 2.0));
        let iy = rect.y + theme::center_y(&self.status_icon, rect.h);
        self.status_icon
            .prepare_render_at(Position::new(status_x - 22.0, iy, 2.0));
        self.status_icon
            .set_content(if status.is_empty() || self.spinning { "" } else { icons::INFO });
        // Dots on a circle where the icon would be, the brightest going round.
        let centre = (status_x - 15.0, rect.y + rect.h * 0.5);
        let turn = self.started.elapsed().as_secs_f32() * 1.2;
        for (i, dot) in self.spinner.iter_mut().enumerate() {
            let angle = i as f32 / SPINNER_DOTS as f32 * TAU;
            dot.set_position(Position::new(
                centre.0 + angle.cos() * SPINNER_RADIUS - SPINNER_DOT / 2.0,
                centre.1 + angle.sin() * SPINNER_RADIUS - SPINNER_DOT / 2.0,
                2.0,
            ));
            let behind = (turn - i as f32 / SPINNER_DOTS as f32).rem_euclid(1.0);
            let (r, g, b, _) = theme::ACCENT;
            dot.set_color((r, g, b, 1.0 - behind * 0.85));
        }

        for widget in self.widgets_mut() {
            widget.update(scene);
        }
    }

    fn render(&self) {
        self.background.render();
        self.separator.render();
        for widget in self.widgets().into_iter() {
            widget.render();
        }
        self.coords.render();
        self.status_icon.render();
        self.status.render();
        if self.spinning {
            for dot in &self.spinner {
                dot.render();
            }
        }
    }

    fn render_overlay(&self) {
        for widget in self.widgets() {
            widget.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        // The release that ends an asset drag belongs to the drag layer
        // (after the toolbar in event order), wherever it happens.
        if let WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) = event {
            if self.state.borrow().drag.as_ref().is_some_and(|drag| drag.active) {
                return false;
            }
        }
        let mut handled = false;
        for widget in self.widgets_mut() {
            if widget.handle_events(scene, window, glfw, event) {
                handled = true;
                if !matches!(event, WindowEvent::CursorPos(..)) {
                    return true;
                }
            }
        }
        if handled {
            return true;
        }
        // Clicks on the bar itself must not fall through to the viewport.
        if let WindowEvent::MouseButton(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.state.borrow().layout.toolbar.contains(x as f32, y as f32);
        }
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _parent: UIElementHandle, _child: Box<dyn UIElement>) {}
    fn contains_child(&self, _handle: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.background.set_z_index(z_index);
        for widget in self.widgets_mut() {
            widget.set_z_index(z_index + 1.0);
        }
    }
}
