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
use glfw::{Action, Cursor, Glfw, MouseButton, Platform, StandardCursor, WindowEvent};

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

/// How close to the window's border the cursor has to be to resize it.
const RESIZE_BORDER: f64 = 5.0;

/// A window edge or corner: -1 for the left (top), 1 for the right (bottom)
/// edge, 0 for neither, horizontally and vertically.
type Edge = (i8, i8);

/// A resize by an edge of the window, as it was when it started.
struct Resize {
    edge: Edge,
    /// The cursor in screen coordinates.
    grab: (f64, f64),
    pos: (i32, i32),
    size: (i32, i32),
}

/// The edge or corner at `(x, y)` in a window `width` x `height`. Without
/// `can_move` (Wayland: the window position is the compositor's) only the
/// right and bottom edges resize.
fn edge_at(width: f64, height: f64, x: f64, y: f64, can_move: bool) -> Option<Edge> {
    let side = |at: f64, length: f64| -> i8 {
        if at < RESIZE_BORDER && can_move {
            -1
        } else if at >= length - RESIZE_BORDER {
            1
        } else {
            0
        }
    };
    let edge = (side(x, width), side(y, height));
    (edge != (0, 0)).then_some(edge)
}

/// The window's position and size with the cursor at `cursor` (screen
/// coordinates) during `resize`, no smaller than `min`.
fn resized(resize: &Resize, cursor: (f64, f64), min: (i32, i32)) -> ((i32, i32), (i32, i32)) {
    let axis = |edge: i8, grab: f64, at: f64, pos: i32, size: i32, min: i32| -> (i32, i32) {
        let delta = (at - grab).round() as i32;
        match edge {
            1 => (pos, (size + delta).max(min)),
            -1 => {
                let new_size = (size - delta).max(min);
                // The opposite edge stays where it is.
                (pos + size - new_size, new_size)
            }
            _ => (pos, size),
        }
    };
    let (x, w) = axis(resize.edge.0, resize.grab.0, cursor.0, resize.pos.0, resize.size.0, min.0);
    let (y, h) = axis(resize.edge.1, resize.grab.1, cursor.1, resize.pos.1, resize.size.1, min.1);
    ((x, y), (w, h))
}

fn edge_cursor(edge: Edge) -> StandardCursor {
    // glfw offers no diagonal shapes here: corners show the horizontal one.
    match edge {
        (0, _) => StandardCursor::VResize,
        _ => StandardCursor::HResize,
    }
}

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
    /// The window edge under the cursor, which a press starts resizing.
    resize_hover: Option<Edge>,
    resize: Option<Resize>,
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
            resize_hover: None,
            resize: None,
        }
    }

    /// The window's own position, where the platform tells it.
    fn window_pos(window: &glfw::Window, glfw: &Glfw) -> (i32, i32) {
        // Not available on Wayland (and this glfw version aborts on the
        // resulting error).
        if glfw.get_platform() == Platform::Wayland {
            (0, 0)
        } else {
            window.get_pos()
        }
    }

    /// Resizing by the window's edges (the window has no system frame).
    /// True when the event was taken for that.
    fn handle_resize(&mut self, window: &mut glfw::Window, glfw: &Glfw, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::CursorPos(x, y) => {
                if let Some(resize) = &self.resize {
                    let (wx, wy) = Self::window_pos(window, glfw);
                    let cursor = (wx as f64 + *x, wy as f64 + *y);
                    let min = super::project_settings::MIN_WINDOW;
                    let ((nx, ny), (w, h)) = resized(resize, cursor, (min.0 as i32, min.1 as i32));
                    if (nx, ny) != (wx, wy) {
                        window.set_pos(nx, ny);
                    }
                    window.set_size(w, h);
                    return true;
                }
                let edge = if window.is_maximized() {
                    None
                } else {
                    let (w, h) = window.get_size();
                    let can_move = glfw.get_platform() != Platform::Wayland;
                    edge_at(w as f64, h as f64, *x, *y, can_move)
                };
                if edge != self.resize_hover {
                    window.set_cursor(edge.map(|e| Cursor::standard(edge_cursor(e))));
                    self.resize_hover = edge;
                }
                edge.is_some()
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let Some(edge) = self.resize_hover else {
                    return false;
                };
                let (x, y) = window.get_cursor_pos();
                let pos = Self::window_pos(window, glfw);
                self.resize = Some(Resize {
                    edge,
                    grab: (pos.0 as f64 + x, pos.1 as f64 + y),
                    pos,
                    size: window.get_size(),
                });
                true
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                self.resize.take().is_some()
            }
            _ => false,
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
        if self.drag_from.is_none() && self.handle_resize(window, glfw, event) {
            self.hovered = None;
            return true;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners_are_found_near_the_border() {
        assert_eq!(edge_at(800.0, 600.0, 400.0, 300.0, true), None);
        assert_eq!(edge_at(800.0, 600.0, 1.0, 300.0, true), Some((-1, 0)));
        assert_eq!(edge_at(800.0, 600.0, 798.0, 599.0, true), Some((1, 1)));
        assert_eq!(edge_at(800.0, 600.0, 400.0, 2.0, true), Some((0, -1)));
        // Without moving the window only the right and bottom edges resize.
        assert_eq!(edge_at(800.0, 600.0, 1.0, 2.0, false), None);
        assert_eq!(edge_at(800.0, 600.0, 1.0, 599.0, false), Some((0, 1)));
    }

    #[test]
    fn resizing_keeps_the_opposite_edge_and_a_minimum_size() {
        let resize = Resize {
            edge: (-1, 1),
            grab: (100.0, 700.0),
            pos: (100, 100),
            size: (800, 600),
        };
        // Left edge 50 px to the right, bottom edge 20 px down.
        assert_eq!(resized(&resize, (150.0, 720.0), (640, 360)), ((150, 100), (750, 620)));
        // Too small: the right edge stays put.
        assert_eq!(resized(&resize, (600.0, 200.0), (640, 360)), ((260, 100), (640, 360)));
    }
}
