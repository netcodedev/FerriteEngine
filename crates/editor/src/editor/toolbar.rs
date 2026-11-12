use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{Fonts, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{button::Button, menu_button::MenuButton},
    UIElement,
};
use glfw::{Glfw, WindowEvent};

use super::{send, Command, GizmoMode, Mode, Shared};

/// Top bar: File and Window menus, gizmo tool toggles, play/stop and the
/// scene title with the latest status message.
pub struct Toolbar {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    background: Plane,
    file_menu: MenuButton,
    window_menu: MenuButton,
    move_button: Button,
    rotate_button: Button,
    play_button: Button,
    title: Text,
    status: Text,
}

impl Toolbar {
    pub fn new(state: Shared) -> Self {
        let file_state = state.clone();
        let file_menu = MenuButton::new(
            "File",
            vec![
                "New scene      Ctrl+N".into(),
                "Open...        Ctrl+O".into(),
                "Save           Ctrl+S".into(),
                "Save as...     Ctrl+Shift+S".into(),
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
        let window_menu = MenuButton::new(
            "Window",
            vec!["Widget showcase".into()],
            Box::new(move |_index, _scene| {
                let flag = window_state.borrow().show_widgets.clone();
                flag.write(!flag.read());
            }),
        );

        let move_state = state.clone();
        let mut move_button = Button::new(
            Position::default(),
            Size::new(78.0, 22.0),
            Box::new(move |_| move_state.borrow_mut().gizmo = GizmoMode::Translate),
        );
        move_button.set_label("Move W");
        let rotate_state = state.clone();
        let mut rotate_button = Button::new(
            Position::default(),
            Size::new(88.0, 22.0),
            Box::new(move |_| rotate_state.borrow_mut().gizmo = GizmoMode::Rotate),
        );
        rotate_button.set_label("Rotate E");

        let play_state = state.clone();
        let mut play_button = Button::new(
            Position::default(),
            Size::new(80.0, 22.0),
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
        play_button.set_label("Play");

        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::new(1280.0, super::layout::TOOLBAR_HEIGHT),
            background: PlaneBuilder::new()
                .color((0.12, 0.12, 0.14, 1.0))
                .border_color((0.3, 0.3, 0.35, 1.0))
                .border_thickness(1.0)
                .build(),
            file_menu,
            window_menu,
            move_button,
            rotate_button,
            play_button,
            title: Text::new(Fonts::RobotoMono, 0, 0, 0, 15.0, String::new()),
            status: Text::new(Fonts::RobotoMono, 0, 0, 0, 14.0, String::new()),
        }
    }

    fn widgets_mut(&mut self) -> [&mut dyn UIElement; 5] {
        [
            &mut self.file_menu,
            &mut self.window_menu,
            &mut self.move_button,
            &mut self.rotate_button,
            &mut self.play_button,
        ]
    }

    fn widgets(&self) -> [&dyn UIElement; 5] {
        [
            &self.file_menu,
            &self.window_menu,
            &self.move_button,
            &self.rotate_button,
            &self.play_button,
        ]
    }
}

impl UIElement for Toolbar {
    fn update(&mut self, scene: &mut Scene) {
        let (rect, mode, gizmo, title, status) = {
            let state = self.state.borrow();
            (
                state.layout.toolbar,
                state.mode,
                state.gizmo,
                state.scene_title(),
                state.status().map(str::to_owned),
            )
        };
        self.size = Size::new(rect.w, rect.h);
        self.background
            .set_position(Position::new(rect.x, rect.y, 0.0));
        self.background.set_size(self.size);

        let y = rect.y + 5.0;
        let mut x = rect.x + 6.0;
        for widget in [
            &mut self.file_menu as &mut dyn UIElement,
            &mut self.window_menu,
        ] {
            widget.set_offset(Offset { x, y });
            x += widget.get_size().width + 6.0;
        }
        x += 18.0;
        for widget in [&mut self.move_button, &mut self.rotate_button] {
            widget.set_offset(Offset { x, y: y - 1.0 });
            x += widget.get_size().width + 6.0;
        }
        self.move_button
            .set_active(gizmo == GizmoMode::Translate && mode == Mode::Edit);
        self.rotate_button
            .set_active(gizmo == GizmoMode::Rotate && mode == Mode::Edit);

        // Play/stop sits at the horizontal centre of the viewport.
        let viewport = self.state.borrow().layout.viewport;
        let play_x = (viewport.x + viewport.w * 0.5 - 40.0).max(x + 12.0);
        self.play_button.set_offset(Offset {
            x: play_x,
            y: y - 1.0,
        });
        self.play_button.set_label(match mode {
            Mode::Edit => "Play",
            Mode::Play => "Stop",
        });
        self.play_button.set_active(mode == Mode::Play);

        let after_play = play_x + 92.0;
        self.title.set_content(&title);
        self.title
            .prepare_render_at(Position::new(after_play, y + 2.0, 2.0));
        let status = status.unwrap_or_else(|| match mode {
            Mode::Play => "Playing - Esc frees the mouse, Ctrl+P stops".into(),
            Mode::Edit => String::new(),
        });
        self.status.set_content(&status);
        let status_x = (rect.x + rect.w - self.status.measure_width() - 10.0)
            .max(after_play + self.title.measure_width() + 20.0);
        self.status
            .prepare_render_at(Position::new(status_x, y + 3.0, 2.0));

        for widget in self.widgets_mut() {
            widget.update(scene);
        }
    }

    fn render(&self) {
        self.background.render();
        for widget in self.widgets() {
            widget.render();
        }
        self.title.render();
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
