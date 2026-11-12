use std::time::Instant;

use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, theme, UIElement};
use glfw::{Action, Glfw, MouseButton, Platform, WindowEvent};

use super::{send, Command, Shared};

const BUTTON_WIDTH: f32 = 44.0;
const DOUBLE_CLICK_SECONDS: f32 = 0.35;

#[derive(Clone, Copy, PartialEq)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

const CONTROLS: [Control; 3] = [Control::Minimize, Control::Maximize, Control::Close];

/// Custom window title bar for the undecorated editor window: name and scene
/// on the left, minimize / maximize / close on the right. Dragging moves
/// the window where the platform allows it (not on Wayland, where the
/// compositor owns window placement); double-click toggles maximize.
pub struct TitleBar {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    background: Plane,
    hover_planes: [Plane; 3],
    control_icons: [Text; 3],
    app_icon: Text,
    name: Text,
    title: Text,
    hovered: Option<Control>,
    drag_from: Option<(f64, f64)>,
    last_click: Option<Instant>,
}

impl TitleBar {
    pub fn new(state: Shared) -> Self {
        let hover = || PlaneBuilder::new().color(theme::TRANSPARENT).build();
        let control_icon = |glyph| theme::icon(glyph, 15.0, theme::TEXT_MUTED);
        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::new(1280.0, super::layout::TITLEBAR_HEIGHT),
            background: PlaneBuilder::new()
                .color(theme::BG_APP)
                .border_color(theme::BORDER)
                .build(),
            hover_planes: [hover(), hover(), hover()],
            control_icons: [
                control_icon(icons::MINUS),
                control_icon(icons::SQUARE),
                control_icon(icons::X),
            ],
            app_icon: theme::icon(icons::BOXES, 16.0, theme::ACCENT),
            name: theme::heading("Ferrite Engine", theme::FONT_SIZE, theme::TEXT),
            title: theme::text("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_DIM),
            hovered: None,
            drag_from: None,
            last_click: None,
        }
    }

    fn control_at(&self, x: f32, y: f32) -> Option<Control> {
        let rect = self.state.borrow().layout.titlebar;
        if !rect.contains(x, y) {
            return None;
        }
        let from_right = rect.x + rect.w - x;
        match (from_right / BUTTON_WIDTH) as usize {
            0 => Some(Control::Close),
            1 => Some(Control::Maximize),
            2 => Some(Control::Minimize),
            _ => None,
        }
    }

    fn toggle_maximize(window: &mut glfw::Window) {
        if window.is_maximized() {
            window.restore();
        } else {
            window.maximize();
        }
    }

    fn activate(&mut self, control: Control, window: &mut glfw::Window) {
        match control {
            Control::Minimize => window.iconify(),
            Control::Maximize => Self::toggle_maximize(window),
            // Goes through the unsaved-changes check; the layer closes the
            // window once that is settled.
            Control::Close => send(&self.state, Command::ConfirmDiscard(Box::new(Command::Quit))),
        }
    }
}

impl UIElement for TitleBar {
    fn update(&mut self, _scene: &mut Scene) {
        let (rect, title) = {
            let state = self.state.borrow();
            (state.layout.titlebar, state.scene_title())
        };
        self.size = Size::new(rect.w, rect.h);
        self.background
            .set_position(Position::new(rect.x, rect.y, 0.0));
        self.background.set_size(self.size);

        let icon_y = theme::center_y(&self.app_icon, rect.h);
        self.app_icon
            .prepare_render_at(Position::new(rect.x + 12.0, rect.y + icon_y, 3.0));
        let name_x = rect.x + 36.0;
        let name_y = theme::center_y(&self.name, rect.h);
        self.name
            .prepare_render_at(Position::new(name_x, rect.y + name_y, 3.0));
        self.title.set_content(&title);
        let title_y = theme::center_y(&self.title, rect.h);
        self.title.prepare_render_at(Position::new(
            name_x + self.name.measure_width() + 12.0,
            rect.y + title_y,
            3.0,
        ));

        for (i, control) in CONTROLS.iter().enumerate() {
            let x = rect.x + rect.w - BUTTON_WIDTH * (3 - i) as f32;
            let hovered = self.hovered == Some(*control);
            let plane = &mut self.hover_planes[i];
            plane.set_position(Position::new(x, rect.y, 1.0));
            plane.set_size(Size::new(BUTTON_WIDTH, rect.h));
            plane.set_color(match (hovered, control) {
                (true, Control::Close) => theme::DANGER,
                (true, _) => theme::BG_CONTROL,
                (false, _) => theme::TRANSPARENT,
            });
            let icon = &mut self.control_icons[i];
            icon.set_color(if hovered { theme::TEXT } else { theme::TEXT_MUTED });
            let ix = x + ((BUTTON_WIDTH - icon.measure_width()) * 0.5).round();
            let iy = theme::center_y(icon, rect.h);
            icon.prepare_render_at(Position::new(ix, rect.y + iy, 3.0));
        }
    }

    fn render(&self) {
        self.background.render();
        for plane in &self.hover_planes {
            plane.render();
        }
        for icon in &self.control_icons {
            icon.render();
        }
        self.app_icon.render();
        self.name.render();
        self.title.render();
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            WindowEvent::CursorPos(x, y) => {
                self.hovered = self.control_at(*x as f32, *y as f32);
                if let Some((grab_x, grab_y)) = self.drag_from {
                    // Window position is not available on Wayland (and this
                    // glfw version aborts on the resulting error).
                    if glfw.get_platform() != Platform::Wayland {
                        let (wx, wy) = window.get_pos();
                        window.set_pos(
                            wx + (*x - grab_x) as i32,
                            wy + (*y - grab_y) as i32,
                        );
                    }
                    return true;
                }
                false
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let rect = self.state.borrow().layout.titlebar;
                if !rect.contains(x as f32, y as f32) {
                    return false;
                }
                if let Some(control) = self.control_at(x as f32, y as f32) {
                    self.activate(control, window);
                    return true;
                }
                let now = Instant::now();
                let double = self
                    .last_click
                    .is_some_and(|t| now.duration_since(t).as_secs_f32() < DOUBLE_CLICK_SECONDS);
                if double {
                    self.last_click = None;
                    Self::toggle_maximize(window);
                } else {
                    self.last_click = Some(now);
                    self.drag_from = Some((x, y));
                }
                true
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                self.drag_from.take().is_some()
            }
            _ => false,
        }
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
    fn set_z_index(&mut self, _z_index: f32) {}
}
