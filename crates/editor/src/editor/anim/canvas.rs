//! Node canvas of the animation workspace: states as nodes, transitions as
//! arrows. Pan, zoom, move nodes, connect them, select for the inspector.

use std::time::Instant;

use cgmath::{InnerSpace, Matrix4, Point3, Vector2, Vector3};
use ferrite::core::{
    model::animation_graph::graph_file::{StateDef, TransitionDef},
    primitives::{Offset, Position, Size},
    renderer::{
        line::{Line, LineRenderer},
        plane::{Plane, PlaneBuilder},
        text::{icons, Fonts, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{
    clip,
    element_handle::UIElementHandle,
    elements::button::{Button, ButtonStyle},
    theme, UIElement,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use super::{
    super::{layout::Rect, send, Command, Shared},
    AnimSelection,
};

const HEADER_HEIGHT: f32 = 40.0;
const NODE_W: f32 = 190.0;
const NODE_H: f32 = 58.0;
const HANDLE_RADIUS: f32 = 7.0;
const GRID: f32 = 32.0;
const MIN_ZOOM: f32 = 0.35;
const MAX_ZOOM: f32 = 2.0;
const DOUBLE_CLICK: f32 = 0.35;
const Z: f32 = 2.0;

struct NodeVisual {
    body: Plane,
    accent: Plane,
    handle: Plane,
    progress: Plane,
    title: Text,
    subtitle: Text,
    badge: Text,
}

enum Drag {
    Pan { last: (f32, f32) },
    Move { index: usize, grab: (f32, f32), moved: bool },
    Link { from: usize },
}

pub struct NodeCanvas {
    handle: UIElementHandle,
    state: Shared,
    offset: Offset,
    size: Size,
    pan: (f32, f32),
    zoom: f32,
    zoom_applied: f32,
    fitted: bool,
    background: Plane,
    header_bg: Plane,
    title: Text,
    hint: Text,
    back_button: Button,
    add_button: Button,
    fit_button: Button,
    revert_button: Button,
    save_button: Button,
    nodes: Vec<NodeVisual>,
    nodes_structure: Option<u64>,
    /// Graph file the view was fitted for; opening another one refits.
    fitted_path: Option<std::path::PathBuf>,
    drag: Option<Drag>,
    hover_node: Option<usize>,
    hover_handle: Option<usize>,
    hover_transition: Option<usize>,
    cursor: (f32, f32),
    last_click: Option<Instant>,
}

fn header_button(label: &str, icon: &str, style: ButtonStyle, on_click: Box<dyn Fn(&mut Scene)>) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, 28.0), on_click);
    button.set_style(style);
    button.set_icon(icon);
    if !label.is_empty() {
        button.set_label(label);
    }
    button.fit(28.0);
    button
}

impl NodeCanvas {
    pub fn new(state: Shared) -> Self {
        let add_state = state.clone();
        let add_button = header_button(
            "Add state",
            icons::PLUS,
            ButtonStyle::Secondary,
            Box::new(move |_| {
                let mut s = add_state.borrow_mut();
                let rect = s.layout.viewport;
                let center = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
                let pending = s.anim_canvas_center.unwrap_or((center.0, center.1));
                let anim = &mut s.anim;
                let name = anim.unique_state_name("state");
                anim.file.states.push(StateDef {
                    name: name.clone(),
                    clips: Vec::new(),
                    sync: false,
                    position: (pending.0 - NODE_W * 0.5, pending.1 - NODE_H * 0.5),
                });
                if anim.file.default_state.is_empty() {
                    anim.file.default_state = name.clone();
                }
                anim.selection = AnimSelection::State(name);
                anim.touch_structure();
            }),
        );
        let fit_button = header_button("", icons::SCAN, ButtonStyle::Ghost, Box::new(|_| {}));
        let back_state = state.clone();
        let back_button = header_button(
            "Back to scene",
            icons::ARROW_LEFT,
            ButtonStyle::Ghost,
            Box::new(move |_| send(&back_state, Command::CloseGraphEditor)),
        );
        let revert_state = state.clone();
        let revert_button = header_button(
            "Revert",
            icons::ROTATE_CW,
            ButtonStyle::Ghost,
            Box::new(move |_| send(&revert_state, Command::RevertGraph)),
        );
        let save_state = state.clone();
        let save_button = header_button(
            "Save",
            icons::SAVE,
            ButtonStyle::Primary,
            Box::new(move |_| send(&save_state, Command::SaveGraph)),
        );
        Self {
            handle: UIElementHandle::new(),
            state,
            offset: Offset::default(),
            size: Size::default(),
            pan: (0.0, 0.0),
            zoom: 1.0,
            zoom_applied: 0.0,
            fitted: false,
            background: PlaneBuilder::new().color((0.067, 0.071, 0.086, 1.0)).build(),
            header_bg: PlaneBuilder::new()
                .color(theme::BG_PANEL)
                .border_color(theme::BORDER)
                .border_thickness(1.0)
                .build(),
            title: theme::heading("", theme::FONT_SIZE, theme::TEXT),
            hint: theme::text(
                "Drag a node to move it  \u{b7}  drag its handle onto another node to add a transition  \u{b7}  double-click to add a state  \u{b7}  scroll to zoom",
                theme::FONT_SIZE_SMALL,
                theme::TEXT_DIM,
            ),
            back_button,
            add_button,
            fit_button,
            revert_button,
            save_button,
            nodes: Vec::new(),
            nodes_structure: None,
            fitted_path: None,
            drag: None,
            hover_node: None,
            hover_handle: None,
            hover_transition: None,
            cursor: (0.0, 0.0),
            last_click: None,
        }
    }

    fn rect(&self) -> Rect {
        self.state.borrow().layout.viewport
    }

    fn area(&self) -> Rect {
        let r = self.rect();
        Rect {
            x: r.x,
            y: r.y + HEADER_HEIGHT,
            w: r.w,
            h: (r.h - HEADER_HEIGHT).max(1.0),
        }
    }

    fn to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        let a = self.area();
        (a.x + self.pan.0 + p.0 * self.zoom, a.y + self.pan.1 + p.1 * self.zoom)
    }

    fn to_graph(&self, p: (f32, f32)) -> (f32, f32) {
        let a = self.area();
        ((p.0 - a.x - self.pan.0) / self.zoom, (p.1 - a.y - self.pan.1) / self.zoom)
    }

    /// Screen rect of node `i`.
    fn node_rect(&self, position: (f32, f32)) -> Rect {
        let (x, y) = self.to_screen(position);
        Rect {
            x,
            y,
            w: NODE_W * self.zoom,
            h: NODE_H * self.zoom,
        }
    }

    fn handle_center(&self, r: &Rect) -> (f32, f32) {
        (r.x + r.w, r.y + r.h * 0.5)
    }

    fn fit_view(&mut self) {
        let states = self.state.borrow().anim.file.states.clone();
        let area = self.area();
        if states.is_empty() || area.w < 10.0 {
            self.pan = (40.0, 40.0);
            self.zoom = 1.0;
            return;
        }
        let min_x = states.iter().map(|s| s.position.0).fold(f32::MAX, f32::min);
        let min_y = states.iter().map(|s| s.position.1).fold(f32::MAX, f32::min);
        let max_x = states.iter().map(|s| s.position.0 + NODE_W).fold(f32::MIN, f32::max);
        let max_y = states.iter().map(|s| s.position.1 + NODE_H).fold(f32::MIN, f32::max);
        let margin = 80.0;
        let zoom = ((area.w - margin * 2.0) / (max_x - min_x))
            .min((area.h - margin * 2.0) / (max_y - min_y))
            .clamp(MIN_ZOOM, 1.25);
        self.zoom = zoom;
        self.pan = (
            (area.w - (max_x - min_x) * zoom) * 0.5 - min_x * zoom,
            (area.h - (max_y - min_y) * zoom) * 0.5 - min_y * zoom,
        );
    }

    fn rebuild_nodes(&mut self, count: usize) {
        self.nodes = (0..count)
            .map(|_| NodeVisual {
                body: PlaneBuilder::new()
                    .color(theme::BG_CONTROL)
                    .border_color(theme::BORDER_STRONG)
                    .border_thickness(1.0)
                    .border_radius_uniform(8.0)
                    .build(),
                accent: PlaneBuilder::new()
                    .color(theme::ACCENT)
                    .border_radius((8.0, 0.0, 8.0, 0.0))
                    .build(),
                handle: PlaneBuilder::new()
                    .color(theme::BG_FIELD)
                    .border_color(theme::BORDER_STRONG)
                    .border_thickness(1.5)
                    .border_radius_uniform(HANDLE_RADIUS)
                    .build(),
                progress: PlaneBuilder::new()
                    .color(theme::SUCCESS)
                    .border_radius_uniform(1.5)
                    .build(),
                title: Text::new(Fonts::InterSemiBold, 0, 0, 0, 13.0, String::new()),
                subtitle: theme::text("", 12.0, theme::TEXT_MUTED),
                badge: theme::text("", 11.0, theme::ACCENT_TEXT),
            })
            .collect();
        self.zoom_applied = 0.0;
    }

    fn transition_segments(&self) -> Vec<(usize, (f32, f32), (f32, f32))> {
        let state = self.state.borrow();
        let file = &state.anim.file;
        let mut out = Vec::new();
        for (i, t) in file.transitions.iter().enumerate() {
            let (Some(a), Some(b)) = (file.state(&t.from), file.state(&t.to)) else {
                continue;
            };
            if t.from == t.to {
                continue;
            }
            let ra = self.node_rect(a.position);
            let rb = self.node_rect(b.position);
            let ca = Vector2::new(ra.x + ra.w * 0.5, ra.y + ra.h * 0.5);
            let cb = Vector2::new(rb.x + rb.w * 0.5, rb.y + rb.h * 0.5);
            let dir = cb - ca;
            if dir.magnitude2() < 1.0 {
                continue;
            }
            let n = dir.normalize();
            // Both directions present: offset each to its own side.
            let reverse = file.transitions.iter().any(|o| o.from == t.to && o.to == t.from);
            let side = if reverse {
                Vector2::new(-n.y, n.x) * 7.0 * self.zoom.max(0.6)
            } else {
                Vector2::new(0.0, 0.0)
            };
            let start = clip_to_rect(ca + side, n, &ra);
            let end = clip_to_rect(cb + side, -n, &rb);
            out.push((i, (start.x, start.y), (end.x, end.y)));
        }
        out
    }

    fn node_at(&self, x: f32, y: f32) -> Option<usize> {
        let state = self.state.borrow();
        // Topmost (last drawn) first.
        state
            .anim
            .file
            .states
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| self.node_rect(s.position).contains(x, y))
            .map(|(i, _)| i)
    }

    fn handle_at(&self, x: f32, y: f32) -> Option<usize> {
        let state = self.state.borrow();
        state
            .anim
            .file
            .states
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| {
                let (hx, hy) = self.handle_center(&self.node_rect(s.position));
                let r = (HANDLE_RADIUS + 3.0) * self.zoom.max(0.7);
                (x - hx).powi(2) + (y - hy).powi(2) <= r * r
            })
            .map(|(i, _)| i)
    }

    fn transition_at(&self, x: f32, y: f32) -> Option<usize> {
        let p = Vector2::new(x, y);
        self.transition_segments()
            .into_iter()
            .map(|(i, a, b)| (i, distance_to_segment(p, a.into(), b.into())))
            .filter(|(_, d)| *d < 6.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn header_widgets_mut(&mut self) -> [&mut Button; 5] {
        [
            &mut self.back_button,
            &mut self.add_button,
            &mut self.fit_button,
            &mut self.revert_button,
            &mut self.save_button,
        ]
    }
}

/// Point where a ray from inside `r` (starting at `from`, heading `dir`)
/// leaves the rectangle.
fn clip_to_rect(from: Vector2<f32>, dir: Vector2<f32>, r: &Rect) -> Vector2<f32> {
    let mut t = f32::MAX;
    if dir.x.abs() > 1e-5 {
        let edge = if dir.x > 0.0 { r.x + r.w } else { r.x };
        t = t.min((edge - from.x) / dir.x);
    }
    if dir.y.abs() > 1e-5 {
        let edge = if dir.y > 0.0 { r.y + r.h } else { r.y };
        t = t.min((edge - from.y) / dir.y);
    }
    from + dir * t.max(0.0)
}

fn distance_to_segment(p: Vector2<f32>, a: Vector2<f32>, b: Vector2<f32>) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.magnitude2().max(1e-5)).clamp(0.0, 1.0);
    (a + ab * t - p).magnitude()
}

fn seg(a: (f32, f32), b: (f32, f32)) -> Line {
    let d = Vector3::new(b.0 - a.0, b.1 - a.1, 0.0);
    let len = d.magnitude();
    Line::new(
        Point3::new(a.0, a.1, 0.0),
        if len > 1e-4 { d / len } else { Vector3::unit_x() },
        len,
    )
}

/// Arrow from `a` to `b` with a head at `b`.
fn arrow(a: (f32, f32), b: (f32, f32), head: f32) -> Vec<Line> {
    let d = Vector2::new(b.0 - a.0, b.1 - a.1);
    if d.magnitude2() < 1.0 {
        return Vec::new();
    }
    let n = d.normalize();
    let side = Vector2::new(-n.y, n.x);
    let back = Vector2::new(b.0, b.1) - n * head;
    let l = back + side * head * 0.5;
    let r = back - side * head * 0.5;
    vec![seg(a, b), seg((l.x, l.y), b), seg((r.x, r.y), b)]
}

fn color3(c: theme::Color) -> Vector3<f32> {
    Vector3::new(c.0, c.1, c.2)
}

/// Lines in window pixels, `width` px thick (see the viewport gizmos).
fn draw_lines(window: (f32, f32), lines: &Vec<Line>, color: Vector3<f32>, width: i32) {
    if lines.is_empty() {
        return;
    }
    let ortho = cgmath::ortho(0.0, window.0, window.1, 0.0, -100.0, 100.0);
    let half = width / 2;
    let extra = if width % 2 == 0 { 0 } else { half };
    for dy in -half..=extra {
        for dx in -half..=extra {
            let shift = Matrix4::from_translation(Vector3::new(
                dx as f32 * 2.0 / window.0,
                dy as f32 * 2.0 / window.1,
                0.0,
            ));
            LineRenderer::render_lines(&(shift * ortho), lines, color, true);
        }
    }
}

impl UIElement for NodeCanvas {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        self.size = Size::new(rect.w, rect.h);
        let path = self.state.borrow().anim.path.clone();
        if self.fitted_path.as_ref() != Some(&path) {
            self.fitted = false;
            self.fitted_path = Some(path);
        }
        if !self.fitted && rect.w > 10.0 {
            self.fit_view();
            self.fitted = true;
        }
        let area = self.area();
        self.state.borrow_mut().anim_canvas_center = Some((
            (area.w * 0.5 - self.pan.0) / self.zoom,
            (area.h * 0.5 - self.pan.1) / self.zoom,
        ));

        // Header.
        self.header_bg
            .set_position(Position::new(rect.x - 1.0, rect.y, Z));
        self.header_bg
            .set_size(Size::new(rect.w + 2.0, HEADER_HEIGHT));
        let by = rect.y + ((HEADER_HEIGHT - 28.0) * 0.5).round();
        self.back_button.set_offset(Offset { x: rect.x + 8.0, y: by });
        let title = format!("Animation graph  \u{b7}  {}", self.state.borrow().anim.file_name());
        self.title.set_content(&title);
        let ty = theme::center_y(&self.title, HEADER_HEIGHT);
        let title_x = rect.x + 8.0 + self.back_button.get_size().width + 12.0;
        self.title
            .prepare_render_at(Position::new(title_x, rect.y + ty, Z + 2.0));
        let mut bx = rect.x + rect.w - 10.0;
        for button in [&mut self.save_button, &mut self.revert_button, &mut self.fit_button]
            .into_iter()
        {
            bx -= button.get_size().width;
            button.set_offset(Offset { x: bx, y: by });
            bx -= 6.0;
        }
        bx -= 10.0;
        bx -= self.add_button.get_size().width;
        self.add_button.set_offset(Offset { x: bx, y: by });
        let hy = rect.y + rect.h - 24.0;
        self.hint
            .prepare_render_at(Position::new(rect.x + 14.0, hy, Z + 2.0));
        self.background
            .set_position(Position::new(area.x, area.y, 0.5));
        self.background.set_size(Size::new(area.w, area.h));

        // Nodes.
        let (structure, states, default_state, selection, status) = {
            let s = self.state.borrow();
            (
                s.anim.structure,
                s.anim.file.states.clone(),
                s.anim.file.default_state.clone(),
                s.anim.selection.clone(),
                s.anim.preview.clone(),
            )
        };
        if self.nodes_structure != Some(structure) || self.nodes.len() != states.len() {
            self.rebuild_nodes(states.len());
            self.nodes_structure = Some(structure);
        }
        let zoom_changed = (self.zoom - self.zoom_applied).abs() > 1e-3;
        let zoom = self.zoom;
        for (i, def) in states.iter().enumerate() {
            let r = self.node_rect(def.position);
            let selected = selection == AnimSelection::State(def.name.clone());
            let active = status.current == def.name;
            let leaving = status.previous.as_deref() == Some(def.name.as_str());
            let hovered = self.hover_node == Some(i);
            let (hx, hy) = self.handle_center(&r);
            let handle_hovered = self.hover_handle == Some(i);
            let node = &mut self.nodes[i];
            node.body.set_position(Position::new(r.x, r.y, Z + 1.0));
            node.body.set_size(Size::new(r.w, r.h));
            node.body.border_color = if selected {
                theme::ACCENT
            } else if active {
                theme::SUCCESS
            } else if hovered {
                theme::BORDER_STRONG
            } else {
                theme::BORDER
            };
            node.body.border_thickness = if selected || active { 2.0 } else { 1.0 };
            node.body.set_color(if hovered { theme::BG_CONTROL_HOVER } else { theme::BG_CONTROL });
            let is_default = def.name == default_state;
            node.accent
                .set_position(Position::new(r.x, r.y, Z + 2.0));
            node.accent.set_size(Size::new(
                if is_default { 4.0 * zoom.max(0.75) } else { 0.0 },
                r.h,
            ));
            let hr = HANDLE_RADIUS * zoom.max(0.7);
            node.handle
                .set_position(Position::new(hx - hr, hy - hr, Z + 3.0));
            node.handle.set_size(Size::new(hr * 2.0, hr * 2.0));
            node.handle.set_color(if handle_hovered { theme::ACCENT } else { theme::BG_FIELD });
            node.handle.border_color = if handle_hovered { theme::ACCENT_HOVER } else { theme::BORDER_STRONG };

            // Blend progress bar along the bottom of the active node.
            let blend = if active && status.previous.is_some() {
                status.progress.clamp(0.0, 1.0)
            } else if leaving {
                (1.0 - status.progress).clamp(0.0, 1.0)
            } else {
                0.0
            };
            node.progress.set_position(Position::new(
                r.x + 8.0 * zoom,
                r.y + r.h - 6.0 * zoom,
                Z + 3.0,
            ));
            node.progress
                .set_size(Size::new((r.w - 16.0 * zoom) * blend, 3.0 * zoom.max(0.7)));

            if zoom_changed {
                node.title
                    .set_font(Fonts::InterSemiBold, (13.5 * zoom).clamp(7.0, 26.0));
                node.subtitle.set_font(Fonts::Inter, (12.0 * zoom).clamp(6.0, 22.0));
                node.badge.set_font(Fonts::Inter, (11.0 * zoom).clamp(6.0, 20.0));
            }
            node.title.set_content(&def.name);
            node.title.set_color(if selected { theme::ACCENT_TEXT } else { theme::TEXT });
            let clips = if def.clips.is_empty() {
                "no clips".to_owned()
            } else {
                def.clips.iter().map(|c| c.file.trim_end_matches(".fbx")).collect::<Vec<_>>().join(" + ")
            };
            node.subtitle.set_content(&clips);
            let pad = 14.0 * zoom;
            node.title
                .prepare_render_at(Position::new(r.x + pad, r.y + 9.0 * zoom, Z + 4.0));
            node.subtitle
                .prepare_render_at(Position::new(r.x + pad, r.y + 31.0 * zoom, Z + 4.0));
            node.badge.set_content(if is_default {
                "default"
            } else if active {
                "active"
            } else {
                ""
            });
            node.badge.set_color(if is_default { theme::ACCENT_TEXT } else { theme::SUCCESS });
            let bw = node.badge.measure_width();
            node.badge.prepare_render_at(Position::new(
                r.x + r.w - bw - 14.0 * zoom,
                r.y + 10.0 * zoom,
                Z + 4.0,
            ));
            for text in [&mut node.title, &mut node.subtitle, &mut node.badge] {
                text.set_z_index(Z + 4.0);
            }
        }
        if zoom_changed {
            self.zoom_applied = self.zoom;
        }

        for button in self.header_widgets_mut() {
            button.update(scene);
        }
    }

    fn render(&self) {
        let (window, selection, status, transitions_from) = {
            let s = self.state.borrow();
            (
                (s.layout.window_w, s.layout.window_h),
                s.anim.selection.clone(),
                s.anim.preview.clone(),
                s.anim
                    .file
                    .transitions
                    .iter()
                    .map(|t| (t.from.clone(), t.to.clone()))
                    .collect::<Vec<_>>(),
            )
        };
        let area = self.area();
        self.background.render();
        clip::push(&self.background);

        // Grid.
        let step = GRID * self.zoom;
        if step >= 8.0 {
            let mut grid = Vec::new();
            let start_x = area.x + self.pan.0.rem_euclid(step);
            let mut x = start_x;
            while x < area.x + area.w {
                grid.push(seg((x, area.y), (x, area.y + area.h)));
                x += step;
            }
            let mut y = area.y + self.pan.1.rem_euclid(step);
            while y < area.y + area.h {
                grid.push(seg((area.x, y), (area.x + area.w, y)));
                y += step;
            }
            draw_lines(window, &grid, Vector3::new(0.095, 0.1, 0.118), 1);
        }

        // Transitions.
        let head = (10.0 * self.zoom).clamp(6.0, 16.0);
        let mut normal = Vec::new();
        let mut highlighted = Vec::new();
        let mut active = Vec::new();
        for (i, a, b) in self.transition_segments() {
            let (from, to) = &transitions_from[i];
            let lines = arrow(a, b, head);
            if selection == AnimSelection::Transition(i) {
                highlighted.extend(lines);
            } else if status.previous.as_deref() == Some(from.as_str()) && status.current == *to {
                active.extend(lines);
            } else if self.hover_transition == Some(i) {
                highlighted.extend(lines);
            } else {
                normal.extend(lines);
            }
        }
        draw_lines(window, &normal, Vector3::new(0.42, 0.44, 0.5), 2);
        draw_lines(window, &active, color3(theme::SUCCESS), 3);
        draw_lines(window, &highlighted, color3(theme::ACCENT), 3);
        if let Some(Drag::Link { from }) = &self.drag {
            let state = self.state.borrow();
            if let Some(def) = state.anim.file.states.get(*from) {
                let start = self.handle_center(&self.node_rect(def.position));
                draw_lines(window, &arrow(start, self.cursor, head), color3(theme::ACCENT_HOVER), 2);
            }
        }

        for node in &self.nodes {
            node.body.render();
            node.accent.render();
            node.progress.render();
            node.handle.render();
            node.title.render();
            node.subtitle.render();
            node.badge.render();
        }
        clip::pop(&self.background);

        self.header_bg.render();
        self.title.render();
        self.hint.render();
        for button in [
            &self.back_button,
            &self.add_button,
            &self.fit_button,
            &self.revert_button,
            &self.save_button,
        ] {
            button.render();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        // Header buttons first (the fit button is handled here directly).
        if let WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) = event {
            let (x, y) = window.get_cursor_pos();
            let r = self.fit_button.get_size();
            let o = *self.fit_button.get_offset();
            if x as f32 >= o.x && x as f32 <= o.x + r.width && y as f32 >= o.y && y as f32 <= o.y + r.height {
                self.fit_view();
                return true;
            }
        }
        for button in self.header_widgets_mut() {
            if button.handle_events(scene, window, glfw, event) && !matches!(event, WindowEvent::CursorPos(..)) {
                return true;
            }
        }

        let area = self.area();
        let rect = self.rect();
        match event {
            WindowEvent::CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                let previous = self.cursor;
                self.cursor = (x, y);
                match &mut self.drag {
                    Some(Drag::Pan { last }) => {
                        self.pan.0 += x - last.0;
                        self.pan.1 += y - last.1;
                        *last = (x, y);
                        return true;
                    }
                    Some(Drag::Move { index, grab, moved }) => {
                        if (x - previous.0).abs() + (y - previous.1).abs() > 0.0 {
                            *moved = true;
                        }
                        let (index, grab) = (*index, *grab);
                        let g = self.to_graph((x, y));
                        let mut s = self.state.borrow_mut();
                        if let Some(def) = s.anim.file.states.get_mut(index) {
                            // Snap to a quarter grid for tidy layouts.
                            let snap = GRID * 0.25;
                            def.position = (
                                ((g.0 - grab.0) / snap).round() * snap,
                                ((g.1 - grab.1) / snap).round() * snap,
                            );
                        }
                        s.anim.touch();
                        return true;
                    }
                    Some(Drag::Link { .. }) => return true,
                    None => {}
                }
                if area.contains(x, y) {
                    self.hover_handle = self.handle_at(x, y);
                    self.hover_node = self.node_at(x, y);
                    self.hover_transition = if self.hover_node.is_none() {
                        self.transition_at(x, y)
                    } else {
                        None
                    };
                } else {
                    self.hover_handle = None;
                    self.hover_node = None;
                    self.hover_transition = None;
                }
                false
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = self.cursor;
                if !area.contains(x, y) {
                    return rect.contains(x, y);
                }
                let now = Instant::now();
                let double = self
                    .last_click
                    .is_some_and(|t| now.duration_since(t).as_secs_f32() < DOUBLE_CLICK);
                self.last_click = Some(now);

                if let Some(i) = self.handle_at(x, y) {
                    self.drag = Some(Drag::Link { from: i });
                    return true;
                }
                if let Some(i) = self.node_at(x, y) {
                    // Layout lookups borrow the state too: compute first.
                    let g = self.to_graph((x, y));
                    let mut s = self.state.borrow_mut();
                    let def = &s.anim.file.states[i];
                    let grab = (g.0 - def.position.0, g.1 - def.position.1);
                    let name = def.name.clone();
                    s.anim.selection = AnimSelection::State(name);
                    drop(s);
                    self.drag = Some(Drag::Move { index: i, grab, moved: false });
                    return true;
                }
                if let Some(t) = self.transition_at(x, y) {
                    self.state.borrow_mut().anim.selection = AnimSelection::Transition(t);
                    return true;
                }
                if double {
                    let g = self.to_graph((x, y));
                    let mut s = self.state.borrow_mut();
                    let name = s.anim.unique_state_name("state");
                    s.anim.file.states.push(StateDef {
                        name: name.clone(),
                        clips: Vec::new(),
                        sync: false,
                        position: (g.0 - NODE_W * 0.5, g.1 - NODE_H * 0.5),
                    });
                    if s.anim.file.default_state.is_empty() {
                        s.anim.file.default_state = name.clone();
                    }
                    s.anim.selection = AnimSelection::State(name);
                    s.anim.touch_structure();
                    return true;
                }
                self.state.borrow_mut().anim.selection = AnimSelection::None;
                self.drag = Some(Drag::Pan { last: (x, y) });
                true
            }
            WindowEvent::MouseButton(MouseButton::Button2 | MouseButton::Button3, Action::Press, _) => {
                let (x, y) = self.cursor;
                if area.contains(x, y) {
                    self.drag = Some(Drag::Pan { last: (x, y) });
                    return true;
                }
                false
            }
            WindowEvent::MouseButton(_, Action::Release, _) => {
                let Some(drag) = self.drag.take() else {
                    return false;
                };
                if let Drag::Link { from } = drag {
                    let (x, y) = self.cursor;
                    if let Some(to) = self.node_at(x, y).filter(|to| *to != from) {
                        let mut s = self.state.borrow_mut();
                        let from_name = s.anim.file.states[from].name.clone();
                        let to_name = s.anim.file.states[to].name.clone();
                        let existing = s
                            .anim
                            .file
                            .transitions
                            .iter()
                            .position(|t| t.from == from_name && t.to == to_name);
                        let index = match existing {
                            Some(i) => i,
                            None => {
                                s.anim.file.transitions.push(TransitionDef {
                                    from: from_name,
                                    to: to_name,
                                    blend: 0.25,
                                    conditions: Vec::new(),
                                });
                                s.anim.touch_structure();
                                s.anim.file.transitions.len() - 1
                            }
                        };
                        s.anim.selection = AnimSelection::Transition(index);
                    }
                }
                true
            }
            WindowEvent::Scroll(_, dy) => {
                let (x, y) = self.cursor;
                if !area.contains(x, y) {
                    return false;
                }
                let before = self.to_graph((x, y));
                self.zoom = (self.zoom * (1.0 + *dy as f32 * 0.12)).clamp(MIN_ZOOM, MAX_ZOOM);
                // Keep the point under the cursor fixed while zooming.
                let after = self.to_screen(before);
                self.pan.0 += x - after.0;
                self.pan.1 += y - after.1;
                true
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
        self.background.set_z_index(z);
        for button in self.header_widgets_mut() {
            button.set_z_index(Z + 3.0);
        }
    }
}
