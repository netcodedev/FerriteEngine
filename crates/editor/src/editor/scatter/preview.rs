//! The scatter graph workspace's preview in the right column, switchable:
//! the world in 3D from the editor camera (the editor draws the scene into
//! `view_rect`; the terrain plants the graph as it is being edited), or a
//! map of the cell under the camera with a dot per model of each output.

use cgmath::{InnerSpace, Matrix4, Point3, Vector3};
use ferrite::{
    core::{
        primitives::{Offset, Position, Size},
        renderer::{
            line::{Line, LineRenderer},
            plane::{Plane, PlaneBuilder},
            text::{icons, Text},
        },
        scene::Scene,
    },
    terrain::scatter::detail::{cell_rect, DETAIL_CELL},
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::button::{Button, ButtonStyle},
    theme, UIElement,
};
use glfw::{Glfw, WindowEvent};

use super::super::{
    layout::{Layout, Rect},
    Shared,
};

const HEADER_HEIGHT: f32 = 36.0;
const MARGIN: f32 = 6.0;
const LEGEND_ROWS: usize = 6;
/// Kept free below the view for the legend's first rows and the note.
const LEGEND_SPACE: f32 = 3.0 * 18.0 + 30.0;
const Z: f32 = 2.0;

/// Where the 3D view (and the map) goes: a square at the top of the right
/// column, below the header; the legend and notes below it.
pub fn view_rect(layout: &Layout) -> Rect {
    let r = layout.stats;
    let w = (r.w - 2.0 * MARGIN).max(0.0);
    // Room below for the legend (three groups at most, as a rule) and a note.
    let h = (r.h - HEADER_HEIGHT - MARGIN - LEGEND_SPACE).max(0.0);
    Rect { x: r.x + MARGIN, y: r.y + HEADER_HEIGHT, w, h: h.min(w) }
}

pub struct ScatterPreview {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    header: Plane,
    frame: Plane,
    map_bg: Plane,
    title: Text,
    note: Text,
    view_3d: Button,
    view_map: Button,
    legend: Vec<Text>,
    showing_3d: Option<bool>,
}

fn toggle(label: &str, icon: &str, state: &Shared, to_3d: bool) -> Button {
    let state = state.clone();
    let mut button = Button::new(
        Position::default(),
        Size::new(10.0, 24.0),
        Box::new(move |_| state.borrow_mut().scatter_preview_3d = to_3d),
    );
    button.set_icon(icon);
    button.set_label(label);
    button.fit(24.0);
    button
}

impl ScatterPreview {
    pub fn new(state: Shared) -> Self {
        let view_3d = toggle("3D", icons::BOX, &state, true);
        let view_map = toggle("Map", icons::LAYOUT_GRID, &state, false);
        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::default(),
            header: PlaneBuilder::new().color(theme::BG_PANEL).border_color(theme::BORDER).border_thickness(1.0).build(),
            frame: PlaneBuilder::new()
                .color((0.0, 0.0, 0.0, 0.0))
                .border_color(theme::BORDER_STRONG)
                .border_thickness(1.0)
                .build(),
            map_bg: PlaneBuilder::new().color((0.067, 0.071, 0.086, 1.0)).build(),
            title: theme::heading("Preview", theme::FONT_SIZE, theme::TEXT),
            note: theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT_DIM),
            view_3d,
            view_map,
            legend: (0..LEGEND_ROWS).map(|_| theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT)).collect(),
            showing_3d: None,
        }
    }

    /// The map's square on screen, and metres to pixels.
    fn map_square(&self) -> (Rect, f32) {
        let v = view_rect(&self.state.borrow().layout);
        let side = v.w.min(v.h).max(10.0);
        (Rect { x: v.x + (v.w - side) * 0.5, y: v.y, w: side, h: side }, side / DETAIL_CELL)
    }
}

fn seg(a: (f32, f32), b: (f32, f32)) -> Line {
    let d = Vector3::new(b.0 - a.0, b.1 - a.1, 0.0);
    let len = d.magnitude();
    Line::new(Point3::new(a.0, a.1, 0.0), if len > 1e-4 { d / len } else { Vector3::unit_x() }, len)
}

fn draw_lines(window: (f32, f32), lines: &Vec<Line>, color: (f32, f32, f32)) {
    if lines.is_empty() {
        return;
    }
    let ortho: Matrix4<f32> = cgmath::ortho(0.0, window.0, window.1, 0.0, -100.0, 100.0);
    LineRenderer::render_lines(&ortho, lines, Vector3::new(color.0, color.1, color.2), true);
}

impl UIElement for ScatterPreview {
    fn update(&mut self, scene: &mut Scene) {
        let (r, view, showing_3d) = {
            let s = self.state.borrow();
            (s.layout.stats, view_rect(&s.layout), s.scatter_preview_3d)
        };
        self.size = Size::new(r.w, r.h);
        self.header.set_position(Position::new(r.x, r.y, Z));
        self.header.set_size(Size::new(r.w, HEADER_HEIGHT));
        self.title.prepare_render_at(Position::new(r.x + 10.0, r.y + theme::center_y(&self.title, HEADER_HEIGHT), Z + 1.0));
        let by = r.y + (HEADER_HEIGHT - 24.0) * 0.5;
        let mut bx = r.x + r.w - 8.0;
        for button in [&mut self.view_map, &mut self.view_3d] {
            bx -= button.get_size().width;
            button.set_offset(Offset { x: bx, y: by });
            bx -= 4.0;
        }
        if self.showing_3d != Some(showing_3d) {
            self.view_3d.set_style(if showing_3d { ButtonStyle::Primary } else { ButtonStyle::Ghost });
            self.view_map.set_style(if showing_3d { ButtonStyle::Ghost } else { ButtonStyle::Primary });
            self.showing_3d = Some(showing_3d);
        }
        // A frame round the 3D view (drawn by the editor under the panels).
        self.frame.set_position(Position::new(view.x - 1.0, view.y - 1.0, Z));
        self.frame.set_size(Size::new(view.w + 2.0, view.h + 2.0));
        self.map_bg.set_position(Position::new(view.x, view.y, Z - 0.5));
        self.map_bg.set_size(Size::new(view.w, view.h));

        // The legend: each group shown, its colour and how many points.
        let groups: Vec<(String, usize, (f32, f32, f32))> = {
            let s = self.state.borrow();
            s.scatter
                .as_ref()
                .map(|edit| edit.shown_points().into_iter().map(|g| (g.label, g.points.len(), g.color)).collect())
                .unwrap_or_default()
        };
        let (square, _) = self.map_square();
        let mut y = square.y + square.h + 8.0;
        for (row, text) in self.legend.iter_mut().enumerate() {
            match groups.get(row) {
                Some((label, count, c)) => {
                    text.set_content(&format!("\u{25cf} {label}: {count}"));
                    text.set_color((c.0, c.1, c.2, 1.0));
                }
                None => text.set_content(""),
            }
            text.prepare_render_at(Position::new(square.x, y, Z + 1.0));
            text.set_z_index(Z + 1.0);
            y += 18.0;
        }
        let note = if showing_3d {
            "Right mouse + WASD: fly. Points: the cell under the camera"
        } else {
            "The 32 m cell under the camera (x)"
        };
        self.note.set_content(note);
        self.note.prepare_render_at(Position::new(view.x, y + 4.0, Z + 1.0));
        self.view_3d.update(scene);
        self.view_map.update(scene);
    }

    fn render(&self) {
        let (showing_3d, window, cell, groups) = {
            let s = self.state.borrow();
            (
                s.scatter_preview_3d,
                (s.layout.window_w, s.layout.window_h),
                s.scatter.as_ref().and_then(|e| e.trace.as_ref().map(|(cell, _)| *cell)),
                s.scatter.as_ref().map(|e| e.shown_points()).unwrap_or_default(),
            )
        };
        self.header.render();
        self.title.render();
        self.view_3d.render();
        self.view_map.render();
        for text in &self.legend {
            text.render();
        }
        self.note.render();
        if showing_3d {
            self.frame.render();
            return;
        }
        self.map_bg.render();
        let (square, scale) = self.map_square();
        let Some(cell) = cell else {
            return;
        };
        let origin = cell_rect(cell);
        let to_screen = |x: f32, z: f32| (square.x + (x - origin.x0) * scale, square.y + (z - origin.z0) * scale);
        // The cell's edge.
        let (a, b) = ((square.x, square.y), (square.x + square.w, square.y + square.h));
        draw_lines(window, &vec![seg(a, (b.0, a.1)), seg((b.0, a.1), b), seg(b, (a.0, b.1)), seg((a.0, b.1), a)], (0.35, 0.37, 0.42));
        // A small cross per point, by group.
        let dot = (scale * 0.15).clamp(1.0, 3.0);
        for group in &groups {
            let lines: Vec<Line> = group
                .points
                .iter()
                .flat_map(|p| {
                    let (x, y) = to_screen(p[0], p[2]);
                    [seg((x - dot, y), (x + dot, y)), seg((x, y - dot), (x, y + dot))]
                })
                .collect();
            draw_lines(window, &lines, group.color);
        }
        // The camera.
        if let Some(eye) = self.state.borrow().scatter_camera {
            if eye[0] >= origin.x0 && eye[0] < origin.x1 && eye[1] >= origin.z0 && eye[1] < origin.z1 {
                let (x, y) = to_screen(eye[0], eye[1]);
                draw_lines(window, &vec![seg((x - 6.0, y - 6.0), (x + 6.0, y + 6.0)), seg((x - 6.0, y + 6.0), (x + 6.0, y - 6.0))], (1.0, 1.0, 1.0));
            }
        }
    }

    fn render_overlay(&self) {
        self.view_3d.render_overlay();
        self.view_map.render_overlay();
    }

    fn handle_events(&mut self, scene: &mut Scene, window: &mut glfw::Window, glfw: &mut Glfw, event: &WindowEvent) -> bool {
        for button in [&mut self.view_3d, &mut self.view_map] {
            if button.handle_events(scene, window, glfw, event) && !matches!(event, WindowEvent::CursorPos(..)) {
                return true;
            }
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            let state = self.state.borrow();
            // Over the 3D view, the camera's (the editor flies it).
            if state.scatter_preview_3d && view_rect(&state.layout).contains(x as f32, y as f32) {
                return false;
            }
            return state.layout.stats.contains(x as f32, y as f32);
        }
        false
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
    fn set_z_index(&mut self, _z: f32) {
        self.view_3d.set_z_index(Z + 2.0);
        self.view_map.set_z_index(Z + 2.0);
    }
}

/// Each shown point as a short upright tick in the 3D preview, over the
/// world just drawn there (the GL viewport is still the preview's).
pub fn draw_points_3d(state: &super::super::Shared, scene: &Scene) {
    let Some(view_projection) = scene.get_active_camera().map(|c| c.get_view_projection()) else {
        return;
    };
    let groups = match state.borrow().scatter.as_ref() {
        Some(edit) => edit.shown_points(),
        None => return,
    };
    for group in groups {
        if group.points.is_empty() {
            continue;
        }
        let lines: Vec<Line> = group
            .points
            .iter()
            .map(|p| Line::new(Point3::new(p[0], p[1], p[2]), Vector3::unit_y(), TICK))
            .collect();
        LineRenderer::render_lines(&view_projection, &lines, Vector3::new(group.color.0, group.color.1, group.color.2), true);
    }
}

/// Height of a point's tick in the 3D preview, metres.
const TICK: f32 = 0.6;
