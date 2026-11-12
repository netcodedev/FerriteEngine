use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        menu_button::{MenuButton, MenuItem},
    },
    theme, UIElement,
};
use glfw::{Glfw, WindowEvent};

use super::{send, Command, GizmoMode, Mode, Shared, Workspace};

const BUTTON_HEIGHT: f32 = 26.0;

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
    scene_tab: Button,
    anim_tab: Button,
    move_button: Button,
    rotate_button: Button,
    play_button: Button,
    status_icon: Text,
    status: Text,
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
            ],
            Box::new(move |index, _scene| {
                let command = match index {
                    0 => Command::ConfirmDiscard(Box::new(Command::NewScene)),
                    1 => Command::ConfirmDiscard(Box::new(Command::OpenDialog)),
                    2 => Command::Save,
                    _ => Command::SaveAsDialog,
                };
                send(&file_state, command);
            }),
        );

        let window_state = state.clone();
        let window_menu = MenuButton::with_items(
            "Window",
            None,
            vec![
                MenuItem::new("Statistics").icon(icons::ACTIVITY),
                MenuItem::new("Widget showcase").icon(icons::COMPONENT),
            ],
            Box::new(move |index, _scene| {
                let state = window_state.borrow();
                let flag = match index {
                    0 => state.show_stats.clone(),
                    _ => state.show_widgets.clone(),
                };
                flag.write(!flag.read());
            }),
        );

        let scene_state = state.clone();
        let scene_tab = tool_button(
            "Scene",
            icons::BOXES,
            Box::new(move |_| send(&scene_state, Command::SetWorkspace(Workspace::Scene))),
        );
        let anim_state = state.clone();
        let anim_tab = tool_button(
            "Animation",
            icons::ACTIVITY,
            Box::new(move |_| send(&anim_state, Command::SetWorkspace(Workspace::Animation))),
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
            scene_tab,
            anim_tab,
            move_button,
            rotate_button,
            play_button,
            status_icon: theme::icon(icons::INFO, 14.0, theme::TEXT_DIM),
            status: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_MUTED),
        }
    }

    fn widgets_mut(&mut self) -> Vec<&mut dyn UIElement> {
        let scene = self.state.borrow().workspace == Workspace::Scene;
        let mut widgets: Vec<&mut dyn UIElement> = vec![
            &mut self.file_menu,
            &mut self.window_menu,
            &mut self.scene_tab,
            &mut self.anim_tab,
            &mut self.play_button,
        ];
        if scene {
            widgets.push(&mut self.move_button);
            widgets.push(&mut self.rotate_button);
        }
        widgets
    }

    fn widgets(&self) -> Vec<&dyn UIElement> {
        let scene = self.state.borrow().workspace == Workspace::Scene;
        let mut widgets: Vec<&dyn UIElement> = vec![
            &self.file_menu,
            &self.window_menu,
            &self.scene_tab,
            &self.anim_tab,
            &self.play_button,
        ];
        if scene {
            widgets.push(&self.move_button);
            widgets.push(&self.rotate_button);
        }
        widgets
    }
}

impl UIElement for Toolbar {
    fn update(&mut self, scene: &mut Scene) {
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
        for widget in [&mut self.scene_tab, &mut self.anim_tab] {
            widget.set_offset(Offset { x, y });
            x += widget.get_size().width + 4.0;
        }
        self.scene_tab.set_active(workspace == Workspace::Scene);
        self.anim_tab.set_active(workspace == Workspace::Animation);
        x += 12.0;
        if workspace == Workspace::Scene {
            for widget in [&mut self.move_button, &mut self.rotate_button] {
                widget.set_offset(Offset { x, y });
                x += widget.get_size().width + 4.0;
            }
        }
        let editing = mode == Mode::Edit;
        self.move_button
            .set_active(gizmo == GizmoMode::Translate && editing);
        self.rotate_button
            .set_active(gizmo == GizmoMode::Rotate && editing);

        // Play/stop sits over the centre of the viewport.
        self.play_button.set_icon(if editing { icons::PLAY } else { icons::SQUARE });
        self.play_button.set_label(if editing { "Play" } else { "Stop" });
        self.play_button
            .set_style(if editing { ButtonStyle::Primary } else { ButtonStyle::Danger });
        self.play_button.set_active(!editing);
        let play_w = self.play_button.get_size().width;
        let play_x = (viewport.x + viewport.w * 0.5 - play_w * 0.5).max(x + 16.0);
        self.play_button.set_offset(Offset { x: play_x, y });

        let status = status.unwrap_or_else(|| match mode {
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
            .set_content(if status.is_empty() { "" } else { icons::INFO });

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
        self.status_icon.render();
        self.status.render();
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
