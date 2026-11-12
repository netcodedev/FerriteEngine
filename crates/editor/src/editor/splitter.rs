//! Drag handles between the docked panels: left column ↔ viewport, top ↔
//! bottom panel of the left column, viewport ↔ right column.

use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::plane::{Plane, PlaneBuilder},
    scene::Scene,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, theme, UIElement};
use glfw::{Action, Cursor, Glfw, MouseButton, StandardCursor, WindowEvent};

use super::{
    layout::{Layout, Rect, MIN_COLUMN, MIN_PANEL_HEIGHT},
    Shared, Workspace,
};

/// Distance from an edge (px) at which it can be grabbed.
const GRAB: f32 = 4.0;
const MIN_VIEWPORT: f32 = 200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    /// Right edge of the left column.
    Side,
    /// Between the two panels of the left column.
    SideSplit,
    /// Left edge of the right column.
    Right,
    /// Between statistics and the widget showcase in the right column.
    RightSplit,
}

impl Edge {
    fn cursor(self) -> StandardCursor {
        match self {
            Edge::Side | Edge::Right => StandardCursor::HResize,
            Edge::SideSplit | Edge::RightSplit => StandardCursor::VResize,
        }
    }
}

pub struct Splitter {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    hover: Option<Edge>,
    drag: Option<Edge>,
    line: Plane,
}

impl Splitter {
    pub fn new(state: Shared) -> Self {
        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::default(),
            hover: None,
            drag: None,
            line: PlaneBuilder::new().color(theme::ACCENT).build(),
        }
    }

    fn edge_at(layout: &Layout, x: f32, y: f32) -> Option<Edge> {
        let side = layout.hierarchy;
        let below_bars = y >= layout.toolbar.y + layout.toolbar.h;
        if !below_bars {
            return None;
        }
        let split_y = layout.inspector.y;
        if x < side.x + side.w - GRAB && (y - split_y).abs() <= GRAB {
            return Some(Edge::SideSplit);
        }
        if (x - (side.x + side.w)).abs() <= GRAB {
            return Some(Edge::Side);
        }
        let right = layout.right;
        if right.w > 0.0
            && layout.stats.w > 0.0
            && layout.widgets.w > 0.0
            && x > right.x + GRAB
            && (y - layout.widgets.y).abs() <= GRAB
        {
            return Some(Edge::RightSplit);
        }
        if right.w > 0.0 && (x - right.x).abs() <= GRAB {
            return Some(Edge::Right);
        }
        None
    }

    /// Thin highlight showing the grabbed or hovered edge.
    fn line_rect(layout: &Layout, edge: Edge) -> Rect {
        match edge {
            Edge::Side => Rect {
                x: layout.hierarchy.x + layout.hierarchy.w - 1.0,
                y: layout.hierarchy.y,
                w: 2.0,
                h: layout.hierarchy.h + layout.inspector.h,
            },
            Edge::SideSplit => Rect {
                x: layout.inspector.x,
                y: layout.inspector.y - 1.0,
                w: layout.inspector.w,
                h: 2.0,
            },
            Edge::Right => Rect {
                x: layout.right.x - 1.0,
                y: layout.right.y,
                w: 2.0,
                h: layout.right.h,
            },
            Edge::RightSplit => Rect {
                x: layout.right.x,
                y: layout.widgets.y - 1.0,
                w: layout.right.w,
                h: 2.0,
            },
        }
    }

    fn apply_drag(&self, edge: Edge, x: f32, y: f32) {
        let mut state = self.state.borrow_mut();
        let layout = state.layout;
        let right_w = layout.right.w;
        let workspace = state.workspace;
        let sizes = &mut state.panel_sizes;
        match edge {
            Edge::Side => {
                let max = (layout.window_w - right_w - MIN_VIEWPORT).max(MIN_COLUMN);
                sizes.side = x.clamp(MIN_COLUMN, max).round();
            }
            Edge::SideSplit => {
                let body = layout.hierarchy.h + layout.inspector.h;
                let top = (y - layout.hierarchy.y).clamp(MIN_PANEL_HEIGHT, (body - MIN_PANEL_HEIGHT).max(MIN_PANEL_HEIGHT));
                sizes.top_share = top / body.max(1.0);
            }
            Edge::RightSplit => {
                let body = layout.right.h;
                let top = (y - layout.right.y).clamp(MIN_PANEL_HEIGHT, (body - MIN_PANEL_HEIGHT).max(MIN_PANEL_HEIGHT));
                sizes.right_share = top / body.max(1.0);
            }
            Edge::Right => {
                let max = (layout.window_w - layout.hierarchy.w - MIN_VIEWPORT).max(MIN_COLUMN);
                let width = (layout.window_w - x).clamp(MIN_COLUMN, max).round();
                match workspace {
                    Workspace::Scene => sizes.stats = width,
                    Workspace::Animation => sizes.preview = width,
                }
            }
        }
    }
}

impl UIElement for Splitter {
    fn update(&mut self, _scene: &mut Scene) {
        let layout = self.state.borrow().layout;
        match self.drag.or(self.hover) {
            Some(edge) => {
                let r = Self::line_rect(&layout, edge);
                self.line.set_position(Position::new(r.x, r.y, 30.0));
                self.line.set_size(Size::new(r.w, r.h));
            }
            None => self.line.set_size(Size::new(0.0, 0.0)),
        }
    }

    fn render(&self) {}

    fn render_overlay(&self) {
        if self.drag.or(self.hover).is_some() {
            self.line.render();
        }
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        window: &mut glfw::Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            WindowEvent::CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                if let Some(edge) = self.drag {
                    self.apply_drag(edge, x, y);
                    return true;
                }
                let layout = self.state.borrow().layout;
                let edge = Self::edge_at(&layout, x, y);
                if edge != self.hover {
                    window.set_cursor(edge.map(|e| Cursor::standard(e.cursor())));
                    self.hover = edge;
                }
                // Keep panels underneath from claiming the hover (and the
                // cursor shape) while over an edge.
                edge.is_some()
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                if let Some(edge) = self.hover {
                    self.drag = Some(edge);
                    return true;
                }
                false
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                if self.drag.take().is_some() {
                    let (x, y) = window.get_cursor_pos();
                    let layout = self.state.borrow().layout;
                    let edge = Self::edge_at(&layout, x as f32, y as f32);
                    if edge.is_none() {
                        window.set_cursor(None);
                    }
                    self.hover = edge;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _p: UIElementHandle, _c: Box<dyn UIElement>) {}
    fn contains_child(&self, _h: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _h: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _h: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
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
    fn set_z_index(&mut self, z: f32) {
        self.line.set_z_index(z + 30.0);
    }
}
