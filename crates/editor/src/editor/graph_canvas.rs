//! A node graph's canvas, shared by the scatter graph and material graph
//! workspaces: nodes as boxes with their pins, wires as curves from an
//! output to an input. Pan, zoom, select, move, wire, and an "add node"
//! menu at the cursor. Keys (delete, undo, copy, ...) come as commands from
//! the editor's shortcuts, so typing in the inspector does not reach them.
//!
//! What it shows and edits comes through `CanvasDoc`, which each workspace
//! implements for its open graph; `CanvasHost` says where that is and which
//! commands the header buttons send.

use std::time::Instant;

use cgmath::{InnerSpace, Matrix4, Point3, Vector2, Vector3};
use ferrite::{
    core::{
        primitives::{Offset, Position, Size},
        renderer::{
            line::{Line, LineRenderer},
            plane::{Plane, PlaneBuilder},
            text::{icons, Fonts, Text},
        },
        scene::Scene,
    },
};
use ferrite_ui::ui::{
    clip,
    element_handle::UIElementHandle,
    elements::button::{Button, ButtonStyle},
    theme, UIElement,
};
use glfw::{Action, Glfw, Key, Modifiers, MouseButton, WindowEvent};

use super::{layout::Rect, send, Command, EditorState, Shared};

/// A node's id in a canvas.
pub type NodeId = u32;

/// One pin of a node: inputs and outputs are counted from 0 each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PinRef {
    pub node: NodeId,
    pub pin: u8,
}

/// A wire from an output pin to an input pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wire {
    pub from: PinRef,
    pub to: PinRef,
}

/// How a node is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    pub id: NodeId,
    pub position: [f32; 2],
    pub title: String,
    pub summary: String,
    /// A line under the summary (a point count), and its colour.
    pub note: (String, theme::Color),
    /// The title bar's colour.
    pub color: theme::Color,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Something is wrong with it (drawn with a red frame).
    pub problem: bool,
}

/// A kind of node the add menu offers.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    pub label: String,
    pub description: String,
}

/// The graph a canvas shows and edits.
pub trait CanvasDoc {
    fn asset(&self) -> ferrite::core::assets::AssetId;
    /// The header: what it is and its name (and whether unsaved).
    fn heading(&self) -> String;
    /// The footer line and its colour.
    fn footer(&self) -> (String, theme::Color);
    /// Goes up when nodes come or go or the selection changes.
    fn structure(&self) -> u64;
    fn nodes(&self) -> Vec<NodeView>;
    fn wires(&self) -> Vec<Wire>;
    fn selection(&self) -> Vec<NodeId>;
    fn selected_wire(&self) -> Option<Wire>;
    fn can_undo(&self) -> bool;
    fn can_redo(&self) -> bool;
    fn catalog(&self) -> Vec<CatalogEntry>;
    /// A node's tooltip: its kind's name and what it does.
    fn tip(&self, node: NodeId) -> Option<CatalogEntry>;

    /// Add the catalog's kind `index` at `at` (graph units) and select it.
    fn add(&mut self, index: usize, at: [f32; 2]);
    fn select(&mut self, nodes: Vec<NodeId>);
    fn select_wire(&mut self, wire: Option<Wire>);
    /// Moving the selection starts (one undo step for the whole drag).
    fn begin_move(&mut self);
    fn move_selection(&mut self, by: [f32; 2]);
    /// Take the wire out of input `to`, if there is one; returns where it
    /// came from.
    fn unwire_into(&mut self, to: PinRef) -> Option<PinRef>;
    /// Wire `from` into `to`, or say why not.
    fn wire(&mut self, from: PinRef, to: PinRef) -> Result<(), String>;
}

/// Where a canvas finds its graph and what its buttons send.
#[derive(Clone, Copy)]
pub struct CanvasHost {
    pub doc: fn(&EditorState) -> Option<&dyn CanvasDoc>,
    pub doc_mut: fn(&mut EditorState) -> Option<&mut dyn CanvasDoc>,
    /// Requests from shortcuts and buttons: open the add menu, fit the view.
    pub take_requests: fn(&mut EditorState) -> (bool, bool),
    /// Each frame while the canvas shows a graph (follow the file, work
    /// out previews).
    pub tick: fn(&mut EditorState, &mut Scene),
    pub close: fn() -> Command,
    pub add_menu: fn() -> Command,
    pub fit: fn() -> Command,
    pub undo: fn() -> Command,
    pub redo: fn() -> Command,
    pub revert: fn() -> Command,
    pub save: fn() -> Command,
}

const HEADER_HEIGHT: f32 = 40.0;
const FOOTER_HEIGHT: f32 = 30.0;
const NODE_W: f32 = 210.0;
const TITLE_H: f32 = 26.0;
/// Pins' centres below the node's top: the first, then one row apart.
const PIN_Y0: f32 = 44.0;
const PIN_ROW: f32 = 22.0;
/// Nodes of up to two pin rows: the summary and note beside the pins.
const SMALL_NODE_H: f32 = 84.0;

/// Pin rows of a node.
fn rows(node: &NodeView) -> usize {
    node.inputs.len().max(node.outputs.len())
}

/// A node's height: room for its pins, then (with more than two rows) the
/// summary and note below them.
fn node_h(rows: usize) -> f32 {
    if rows <= 2 {
        SMALL_NODE_H
    } else {
        PIN_Y0 + (rows - 1) as f32 * PIN_ROW + 18.0 + 40.0
    }
}

/// Where the summary and the note go below a node's top.
fn text_y(rows: usize) -> (f32, f32) {
    if rows <= 2 {
        (34.0, 56.0)
    } else {
        let below = PIN_Y0 + (rows - 1) as f32 * PIN_ROW + 14.0;
        (below, below + 20.0)
    }
}
const PIN_RADIUS: f32 = 6.0;
const GRID: f32 = 32.0;
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 2.0;
const DOUBLE_CLICK: f32 = 0.35;
const Z: f32 = 2.0;
/// How many of the topmost nodes get depth levels of their own (see
/// `stacking`); the ones below share the lowest. Each takes two whole
/// levels (text keeps whole depths only), all of them under the overlays.
const OWN_LEVELS: usize = 8;
/// The add menu: rows shown at most, and their height.
const MENU_ROWS: usize = 14;
const MENU_ROW_H: f32 = 22.0;
const MENU_W: f32 = 220.0;
/// Over the panels, under the editor's own popups.
const OVERLAY_Z: f32 = 30.0;

/// How long the cursor rests on a node before its tooltip shows.
const TIP_DELAY: f32 = 0.5;
const TIP_W: f32 = 280.0;
const TIP_LINES: usize = 7;

/// A tooltip of several lines: a node kind's name and what it does.
struct Tip {
    background: Plane,
    title: Text,
    lines: Vec<Text>,
    shown: bool,
}

impl Tip {
    fn new() -> Self {
        Self {
            background: PlaneBuilder::new()
                .color(theme::BG_MENU)
                .border_color(theme::BORDER_STRONG)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS)
                .build(),
            title: theme::heading("", theme::FONT_SIZE_SMALL + 0.5, theme::TEXT),
            lines: (0..TIP_LINES).map(|_| theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT_MUTED)).collect(),
            shown: false,
        }
    }

    /// Show `entry` near `at`, kept inside `bounds`.
    fn show(&mut self, entry: &CatalogEntry, at: (f32, f32), bounds: &Rect) {
        self.title.set_content(&entry.label);
        let text = entry.description.replace('`', "");
        // Word wrap to the tooltip's width.
        let inner = TIP_W - 20.0;
        let mut wrapped: Vec<String> = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && self.lines[0].measure_str(&candidate) > inner {
                wrapped.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        if !line.is_empty() {
            wrapped.push(line);
        }
        wrapped.truncate(TIP_LINES);
        let line_h = 17.0;
        let h = 12.0 + 20.0 + wrapped.len() as f32 * line_h + 8.0;
        let x = (at.0 + 16.0).min(bounds.x + bounds.w - TIP_W - 4.0).max(bounds.x + 4.0);
        let y = (at.1 + 18.0).min(bounds.y + bounds.h - h - 4.0).max(bounds.y + 4.0);
        self.background.set_position(Position::new(x, y, OVERLAY_Z + 4.0));
        self.background.set_size(Size::new(TIP_W, h));
        self.title.prepare_render_at(Position::new(x + 10.0, y + 8.0, OVERLAY_Z + 5.0));
        self.title.set_z_index(OVERLAY_Z + 5.0);
        for (i, text) in self.lines.iter_mut().enumerate() {
            text.set_content(wrapped.get(i).map_or("", String::as_str));
            text.prepare_render_at(Position::new(x + 10.0, y + 30.0 + i as f32 * line_h, OVERLAY_Z + 5.0));
            text.set_z_index(OVERLAY_Z + 5.0);
        }
        self.shown = true;
    }

    fn render(&self) {
        if !self.shown {
            return;
        }
        self.background.render();
        self.title.render();
        for text in &self.lines {
            text.render();
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    In,
    Out,
}

struct NodeVisual {
    body: Plane,
    bar: Plane,
    pins: Vec<(Side, u8, Plane, Option<Text>)>,
    title: Text,
    summary: Text,
    count: Text,
}

enum Drag {
    Pan { last: (f32, f32) },
    /// The selected nodes, from where the drag began (graph units).
    Move { start: (f32, f32), recorded: bool },
    Wire { from: PinRef },
    Select { start: (f32, f32) },
}

struct AddMenu {
    /// Everything that can be added (the filter picks from it).
    catalog: Vec<CatalogEntry>,
    /// Screen and graph point it was opened at.
    at: (f32, f32),
    at_graph: (f32, f32),
    filter: String,
    index: usize,
    background: Plane,
    highlight: Plane,
    filter_text: Text,
    rows: Vec<Text>,
}

impl AddMenu {
    fn new(catalog: Vec<CatalogEntry>, at: (f32, f32), at_graph: (f32, f32)) -> Self {
        let mut menu = Self {
            catalog,
            at,
            at_graph,
            filter: String::new(),
            index: 0,
            background: PlaneBuilder::new()
                .color(theme::BG_PANEL)
                .border_color(theme::BORDER_STRONG)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS)
                .build(),
            highlight: PlaneBuilder::new().color(theme::ACCENT).border_radius_uniform(4.0).build(),
            filter_text: theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT),
            rows: (0..MENU_ROWS).map(|_| theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT)).collect(),
        };
        menu.layout();
        menu
    }

    /// The kinds the filter lets through: their catalog index and entry.
    fn choices(&self) -> Vec<(usize, CatalogEntry)> {
        let filter = self.filter.to_lowercase();
        self.catalog
            .iter()
            .enumerate()
            .filter(|(_, k)| filter.is_empty() || k.label.to_lowercase().contains(&filter))
            .map(|(i, k)| (i, k.clone()))
            .collect()
    }

    fn row_rect(&self, row: usize) -> Rect {
        Rect {
            x: self.at.0 + 4.0,
            y: self.at.1 + 30.0 + row as f32 * MENU_ROW_H,
            w: MENU_W - 8.0,
            h: MENU_ROW_H,
        }
    }

    fn rect(&self) -> Rect {
        let rows = self.choices().len().clamp(1, MENU_ROWS);
        Rect { x: self.at.0, y: self.at.1, w: MENU_W, h: 34.0 + rows as f32 * MENU_ROW_H }
    }

    fn layout(&mut self) {
        let choices = self.choices();
        self.index = self.index.min(choices.len().saturating_sub(1));
        let r = self.rect();
        self.background.set_position(Position::new(r.x, r.y, OVERLAY_Z));
        self.background.set_size(Size::new(r.w, r.h));
        let typed = if self.filter.is_empty() { tr!("graph_canvas.type_to_search").to_string() } else { self.filter.clone() };
        self.filter_text.set_content(&typed);
        self.filter_text
            .set_color(if self.filter.is_empty() { theme::TEXT_DIM } else { theme::TEXT });
        self.filter_text
            .prepare_render_at(Position::new(r.x + 10.0, r.y + 7.0, OVERLAY_Z + 2.0));
        for (row, text) in self.rows.iter_mut().enumerate() {
            let label = choices.get(row).map_or("", |(_, k)| k.label.as_str());
            text.set_content(label);
            text.set_color(if row == self.index { theme::ACCENT_TEXT } else { theme::TEXT });
        }
        for row in 0..MENU_ROWS {
            let rr = self.row_rect(row);
            self.rows[row].prepare_render_at(Position::new(rr.x + 8.0, rr.y + 3.0, OVERLAY_Z + 2.0));
            self.rows[row].set_z_index(OVERLAY_Z + 2.0);
        }
        self.filter_text.set_z_index(OVERLAY_Z + 2.0);
        let hr = self.row_rect(self.index);
        self.highlight.set_position(Position::new(hr.x, hr.y, OVERLAY_Z + 1.0));
        self.highlight
            .set_size(Size::new(hr.w, if choices.is_empty() { 0.0 } else { hr.h }));
        self.highlight.set_color((theme::ACCENT.0, theme::ACCENT.1, theme::ACCENT.2, 0.35));
    }

    fn render(&self) {
        self.background.render();
        self.highlight.render();
        self.filter_text.render();
        let shown = self.choices().len().min(MENU_ROWS);
        for text in &self.rows[..shown] {
            text.render();
        }
    }
}

pub struct GraphCanvas {
    handle: UIElementHandle,
    state: Shared,
    host: CanvasHost,
    /// What the graph's nodes looked like when last laid out.
    views: Vec<NodeView>,
    offset: Offset,
    size: Size,
    pan: (f32, f32),
    zoom: f32,
    zoom_applied: f32,
    /// The asset the view was fitted for; opening another refits.
    fitted_for: Option<ferrite::core::assets::AssetId>,
    background: Plane,
    header_bg: Plane,
    footer_bg: Plane,
    title: Text,
    footer: Text,
    buttons: Vec<Button>,
    nodes: Vec<NodeVisual>,
    nodes_built_for: Option<(u64, usize)>,
    drag: Option<Drag>,
    menu: Option<AddMenu>,
    hover_node: Option<NodeId>,
    hover_pin: Option<(NodeId, Side, u8)>,
    hover_wire: Option<Wire>,
    /// The node under the cursor and since when (for its tooltip).
    hover_since: Option<(NodeId, Instant)>,
    /// Node indices from bottom to top, as last laid out.
    stacking: Vec<usize>,
    tip: Tip,
    cursor: (f32, f32),
    last_click: Option<Instant>,
}

fn header_button(label: &str, icon: &str, style: ButtonStyle, tooltip: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, 28.0), on_click);
    button.set_style(style);
    if !icon.is_empty() {
        button.set_icon(icon);
    }
    if !label.is_empty() {
        button.set_label(label);
    }
    if !tooltip.is_empty() {
        button.set_tooltip(tooltip);
    }
    button.fit(28.0);
    button
}

/// Header buttons, left to right after the title is placed: back first,
/// the rest from the right edge.
const BACK: usize = 0;

impl GraphCanvas {
    pub fn new(state: Shared, host: CanvasHost) -> Self {
        let command = |state: &Shared, command: fn() -> Command| {
            let state = state.clone();
            Box::new(move |_: &mut Scene| send(&state, command())) as Box<dyn Fn(&mut Scene)>
        };
        let buttons = vec![
            header_button(tr!("graph_canvas.back_to_scene"), icons::ARROW_LEFT, ButtonStyle::Ghost, "", command(&state, host.close)),
            header_button(tr!("common.add"), icons::PLUS, ButtonStyle::Secondary, tr!("graph_canvas.add_tip"), command(&state, host.add_menu)),
            header_button("", icons::SCAN, ButtonStyle::Ghost, tr!("graph_canvas.fit_tip"), command(&state, host.fit)),
            header_button(tr!("graph_canvas.undo"), "", ButtonStyle::Ghost, tr!("graph_canvas.undo_tip"), command(&state, host.undo)),
            header_button(tr!("graph_canvas.redo"), "", ButtonStyle::Ghost, tr!("graph_canvas.redo_tip"), command(&state, host.redo)),
            header_button(tr!("common.revert"), icons::ROTATE_CW, ButtonStyle::Ghost, tr!("graph_canvas.revert_tip"), command(&state, host.revert)),
            header_button(tr!("common.save"), icons::SAVE, ButtonStyle::Primary, tr!("graph_canvas.save_tip"), command(&state, host.save)),
        ];
        Self {
            handle: UIElementHandle::new(),
            state,
            host,
            views: Vec::new(),
            offset: Offset::default(),
            size: Size::default(),
            pan: (40.0, 40.0),
            zoom: 1.0,
            zoom_applied: 0.0,
            fitted_for: None,
            background: PlaneBuilder::new().color((0.067, 0.071, 0.086, 1.0)).build(),
            header_bg: PlaneBuilder::new()
                .color(theme::BG_PANEL)
                .border_color(theme::BORDER)
                .border_thickness(1.0)
                .build(),
            footer_bg: PlaneBuilder::new().color(theme::BG_PANEL).build(),
            title: theme::heading("", theme::FONT_SIZE, theme::TEXT),
            footer: theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT_DIM),
            buttons,
            nodes: Vec::new(),
            nodes_built_for: None,
            drag: None,
            menu: None,
            hover_node: None,
            hover_pin: None,
            hover_wire: None,
            hover_since: None,
            stacking: Vec::new(),
            tip: Tip::new(),
            cursor: (0.0, 0.0),
            last_click: None,
        }
    }

    fn rect(&self) -> Rect {
        self.state.borrow().layout.viewport
    }

    /// Where the graph is drawn: between header and footer.
    fn area(&self) -> Rect {
        let r = self.rect();
        Rect { x: r.x, y: r.y + HEADER_HEIGHT, w: r.w, h: (r.h - HEADER_HEIGHT - FOOTER_HEIGHT).max(1.0) }
    }

    fn to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        let a = self.area();
        (a.x + self.pan.0 + p.0 * self.zoom, a.y + self.pan.1 + p.1 * self.zoom)
    }

    fn to_graph(&self, p: (f32, f32)) -> (f32, f32) {
        let a = self.area();
        ((p.0 - a.x - self.pan.0) / self.zoom, (p.1 - a.y - self.pan.1) / self.zoom)
    }

    fn node_rect(&self, node: &NodeView) -> Rect {
        let (x, y) = self.to_screen((node.position[0], node.position[1]));
        Rect { x, y, w: NODE_W * self.zoom, h: node_h(rows(node)) * self.zoom }
    }

    /// Screen centre of a pin of `node`.
    fn pin_center(&self, node: &NodeView, side: Side, pin: u8) -> (f32, f32) {
        let x = match side {
            Side::In => node.position[0],
            Side::Out => node.position[0] + NODE_W,
        };
        self.to_screen((x, node.position[1] + PIN_Y0 + pin as f32 * PIN_ROW))
    }

    fn view(&self, id: NodeId) -> Option<&NodeView> {
        self.views.iter().find(|n| n.id == id)
    }

    fn with_edit<R>(&self, f: impl FnOnce(&dyn CanvasDoc) -> R) -> Option<R> {
        (self.host.doc)(&self.state.borrow()).map(f)
    }

    fn with_edit_mut<R>(&self, f: impl FnOnce(&mut dyn CanvasDoc) -> R) -> Option<R> {
        (self.host.doc_mut)(&mut self.state.borrow_mut()).map(f)
    }

    fn fit_view(&mut self) {
        let Some(nodes) = self.with_edit(|e| e.nodes()) else {
            return;
        };
        let positions: Vec<[f32; 2]> = nodes.iter().map(|n| n.position).collect();
        let area = self.area();
        if positions.is_empty() || area.w < 10.0 {
            self.pan = (40.0, 40.0);
            self.zoom = 1.0;
            return;
        }
        let min_x = positions.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        let min_y = positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        let max_x = positions.iter().map(|p| p[0] + NODE_W).fold(f32::MIN, f32::max);
        let max_y = nodes.iter().map(|n| n.position[1] + node_h(rows(n))).fold(f32::MIN, f32::max);
        let margin = 60.0;
        self.zoom = ((area.w - margin * 2.0) / (max_x - min_x))
            .min((area.h - margin * 2.0) / (max_y - min_y))
            .clamp(MIN_ZOOM, 1.25);
        self.pan = (
            (area.w - (max_x - min_x) * self.zoom) * 0.5 - min_x * self.zoom,
            (area.h - (max_y - min_y) * self.zoom) * 0.5 - min_y * self.zoom,
        );
    }

    /// The nodes from bottom to top: in graph order, the selected ones
    /// last (on top).
    fn stacking(nodes: &[NodeView], selection: &[NodeId]) -> Vec<usize> {
        let (selected, rest): (Vec<usize>, Vec<usize>) = (0..nodes.len()).partition(|&i| selection.contains(&nodes[i].id));
        rest.into_iter().chain(selected).collect()
    }

    fn node_at(&self, x: f32, y: f32) -> Option<NodeId> {
        self.stacking
            .iter()
            .rev()
            .filter_map(|&i| self.views.get(i))
            .find(|n| self.node_rect(n).contains(x, y))
            .map(|n| n.id)
    }

    fn pin_at(&self, x: f32, y: f32) -> Option<(NodeId, Side, u8)> {
        let reach = (PIN_RADIUS + 4.0) * self.zoom.max(0.7);
        for node in self.stacking.iter().rev().filter_map(|&i| self.views.get(i)) {
            let pins = (0..node.inputs.len())
                .map(|p| (Side::In, p as u8))
                .chain((0..node.outputs.len()).map(|p| (Side::Out, p as u8)));
            for (side, pin) in pins {
                let (px, py) = self.pin_center(node, side, pin);
                if (x - px).powi(2) + (y - py).powi(2) <= reach * reach {
                    return Some((node.id, side, pin));
                }
            }
        }
        None
    }

    /// Each wire's curve on screen.
    fn wire_curves(&self) -> Vec<(Wire, Vec<(f32, f32)>)> {
        let wires = self.with_edit(|e| e.wires()).unwrap_or_default();
        wires
            .into_iter()
            .filter_map(|w| {
                let from = self.view(w.from.node)?;
                let to = self.view(w.to.node)?;
                let a = self.pin_center(from, Side::Out, w.from.pin);
                let b = self.pin_center(to, Side::In, w.to.pin);
                Some((w, curve(a, b, self.zoom)))
            })
            .collect()
    }

    fn wire_at(&self, x: f32, y: f32) -> Option<Wire> {
        let p = Vector2::new(x, y);
        self.wire_curves()
            .into_iter()
            .map(|(w, points)| {
                let d = points
                    .windows(2)
                    .map(|s| distance_to_segment(p, s[0].into(), s[1].into()))
                    .fold(f32::MAX, f32::min);
                (w, d)
            })
            .filter(|(_, d)| *d < 6.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(w, _)| w)
    }

    fn open_menu(&mut self, at: (f32, f32)) {
        let area = self.area();
        // Kept inside the canvas.
        let x = at.0.min(area.x + area.w - MENU_W - 4.0).max(area.x + 4.0);
        let y = at.1.min(area.y + area.h - 34.0 - MENU_ROWS as f32 * MENU_ROW_H).max(area.y + 4.0);
        let catalog = self.with_edit(|e| e.catalog()).unwrap_or_default();
        self.menu = Some(AddMenu::new(catalog, (x, y), self.to_graph(at)));
    }

    /// Add the menu's chosen kind where the menu was opened.
    fn pick(&mut self, index: usize) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        let Some((kind, _)) = menu.choices().into_iter().nth(index) else {
            return;
        };
        let at = [menu.at_graph.0 - NODE_W * 0.5, menu.at_graph.1 - TITLE_H * 0.5];
        self.with_edit_mut(|e| e.add(kind, at));
    }

    fn rebuild_nodes(&mut self) {
        let Some(nodes) = self.with_edit(|e| {
            e.nodes().into_iter().map(|n| (n.inputs, n.outputs)).collect::<Vec<_>>()
        }) else {
            self.nodes.clear();
            return;
        };
        let pin = |side: Side, index: usize, name: &str, named: bool| {
            let plane = PlaneBuilder::new()
                .color(theme::BG_FIELD)
                .border_color(theme::BORDER_STRONG)
                .border_thickness(1.5)
                .border_radius_uniform(PIN_RADIUS)
                .build();
            let label = named.then(|| theme::text(name, 11.0, theme::TEXT_MUTED));
            (side, index as u8, plane, label)
        };
        self.nodes = nodes
            .into_iter()
            .map(|(inputs, outputs)| {
                let mut pins = Vec::new();
                for (i, name) in inputs.iter().enumerate() {
                    pins.push(pin(Side::In, i, name, inputs.len() > 1));
                }
                for (i, name) in outputs.iter().enumerate() {
                    pins.push(pin(Side::Out, i, name, outputs.len() > 1));
                }
                NodeVisual {
                    body: PlaneBuilder::new()
                        .color(theme::BG_CONTROL)
                        .border_color(theme::BORDER_STRONG)
                        .border_thickness(1.0)
                        .border_radius_uniform(8.0)
                        .build(),
                    bar: PlaneBuilder::new().color(theme::ACCENT).border_radius((8.0, 8.0, 0.0, 0.0)).build(),
                    pins,
                    title: Text::new(Fonts::InterSemiBold, 0, 0, 0, 13.0, String::new()),
                    summary: theme::text("", 12.0, theme::TEXT_MUTED),
                    count: theme::text("", 12.0, theme::TEXT),
                }
            })
            .collect();
        self.zoom_applied = 0.0;
    }
}

/// A wire's curve from output `a` to input `b`, as points on screen.
fn curve(a: (f32, f32), b: (f32, f32), zoom: f32) -> Vec<(f32, f32)> {
    let reach = ((b.0 - a.0).abs() * 0.5).max(50.0 * zoom);
    let c1 = (a.0 + reach, a.1);
    let c2 = (b.0 - reach, b.1);
    (0..=28)
        .map(|i| {
            let t = i as f32 / 28.0;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            (
                w[0] * a.0 + w[1] * c1.0 + w[2] * c2.0 + w[3] * b.0,
                w[0] * a.1 + w[1] * c1.1 + w[2] * c2.1 + w[3] * b.1,
            )
        })
        .collect()
}

fn polyline(points: &[(f32, f32)]) -> Vec<Line> {
    points.windows(2).map(|s| seg(s[0], s[1])).collect()
}

fn distance_to_segment(p: Vector2<f32>, a: Vector2<f32>, b: Vector2<f32>) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.magnitude2().max(1e-5)).clamp(0.0, 1.0);
    (a + ab * t - p).magnitude()
}

fn seg(a: (f32, f32), b: (f32, f32)) -> Line {
    let d = Vector3::new(b.0 - a.0, b.1 - a.1, 0.0);
    let len = d.magnitude();
    Line::new(Point3::new(a.0, a.1, 0.0), if len > 1e-4 { d / len } else { Vector3::unit_x() }, len)
}

fn color3(c: theme::Color) -> Vector3<f32> {
    Vector3::new(c.0, c.1, c.2)
}

/// Lines in window pixels, `width` px thick (see `anim::canvas`).
fn draw_lines(window: (f32, f32), lines: &Vec<Line>, color: Vector3<f32>, width: i32) {
    if lines.is_empty() {
        return;
    }
    let ortho = cgmath::ortho(0.0, window.0, window.1, 0.0, -100.0, 100.0);
    let half = width / 2;
    let extra = if width % 2 == 0 { 0 } else { half };
    for dy in -half..=extra {
        for dx in -half..=extra {
            let shift = Matrix4::from_translation(Vector3::new(dx as f32 * 2.0 / window.0, dy as f32 * 2.0 / window.1, 0.0));
            LineRenderer::render_lines(&(shift * ortho), lines, color, true);
        }
    }
}

impl UIElement for GraphCanvas {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        self.size = Size::new(rect.w, rect.h);
        let Some((asset, heading, structure, (footer, footer_color), can_undo, can_redo)) =
            self.with_edit(|e| (e.asset(), e.heading(), e.structure(), e.footer(), e.can_undo(), e.can_redo()))
        else {
            self.views.clear();
            return;
        };
        (self.host.tick)(&mut self.state.borrow_mut(), scene);
        let Some((views, selection)) = self.with_edit(|e| (e.nodes(), e.selection())) else {
            return;
        };
        self.views = views;
        if self.fitted_for != Some(asset) && rect.w > 10.0 {
            self.fit_view();
            self.fitted_for = Some(asset);
        }
        // Requests from the shortcuts and header buttons.
        let (open_menu, fit) = (self.host.take_requests)(&mut self.state.borrow_mut());
        if fit {
            self.fit_view();
        }
        if open_menu {
            let area = self.area();
            let at = if area.contains(self.cursor.0, self.cursor.1) {
                self.cursor
            } else {
                (area.x + area.w * 0.5 - MENU_W * 0.5, area.y + area.h * 0.3)
            };
            self.open_menu(at);
        }

        // Header and footer.
        self.header_bg.set_position(Position::new(rect.x - 1.0, rect.y, Z));
        self.header_bg.set_size(Size::new(rect.w + 2.0, HEADER_HEIGHT));
        let by = rect.y + ((HEADER_HEIGHT - 28.0) * 0.5).round();
        self.buttons[BACK].set_offset(Offset { x: rect.x + 8.0, y: by });
        self.title.set_content(&heading);
        let ty = theme::center_y(&self.title, HEADER_HEIGHT);
        let title_x = rect.x + 8.0 + self.buttons[BACK].get_size().width + 12.0;
        self.title.prepare_render_at(Position::new(title_x, rect.y + ty, Z + 2.0));
        let mut bx = rect.x + rect.w - 10.0;
        for button in self.buttons[1..].iter_mut().rev() {
            bx -= button.get_size().width;
            button.set_offset(Offset { x: bx, y: by });
            bx -= 6.0;
        }
        self.buttons[3].set_enabled(can_undo);
        self.buttons[4].set_enabled(can_redo);
        let area = self.area();
        self.footer_bg
            .set_position(Position::new(rect.x, area.y + area.h, Z));
        self.footer_bg.set_size(Size::new(rect.w, FOOTER_HEIGHT));
        self.footer.set_color(footer_color);
        self.footer.set_content(&footer);
        let fy = area.y + area.h + theme::center_y(&self.footer, FOOTER_HEIGHT);
        self.footer.prepare_render_at(Position::new(rect.x + 12.0, fy, Z + 2.0));
        self.background.set_position(Position::new(area.x, area.y, 0.5));
        self.background.set_size(Size::new(area.w, area.h));

        // Nodes.
        let count = self.views.len();
        if self.nodes_built_for != Some((structure, count)) {
            self.rebuild_nodes();
            self.nodes_built_for = Some((structure, count));
        }
        let nodes = self.views.clone();
        let zoom = self.zoom;
        let zoom_changed = (zoom - self.zoom_applied).abs() > 1e-3;
        let hover_node = self.hover_node;
        let hover_pin = self.hover_pin;
        // The topmost nodes each in depth levels of their own, so
        // overlapping nodes do not interleave their parts; the selected ones
        // on top. Text keeps whole depths only: each node takes two whole
        // levels, its planes between them, its text on the upper one.
        self.stacking = Self::stacking(&nodes, &selection);
        let count = self.stacking.len();
        let mut level = vec![0usize; nodes.len()];
        for (rank, &i) in self.stacking.iter().enumerate() {
            if let Some(l) = level.get_mut(i) {
                *l = (rank + OWN_LEVELS).saturating_sub(count);
            }
        }
        for (i, node) in nodes.iter().enumerate() {
            let z = Z + 1.0 + 2.0 * level[i] as f32;
            // Body, title bar, pins between the node's levels; text on top.
            let layer = |k: f32| if k >= 3.0 { z + 1.0 } else { z + k * 0.25 };
            let r = self.node_rect(node);
            let pin_centers: Vec<(f32, f32)> = self
                .nodes
                .get(i)
                .map(|v| v.pins.iter().map(|(side, pin, _, _)| self.pin_center(node, *side, *pin)).collect())
                .unwrap_or_default();
            let (summary_y, note_y) = text_y(rows(node));
            let Some(visual) = self.nodes.get_mut(i) else {
                continue;
            };
            let selected = selection.contains(&node.id);
            let hovered = hover_node == Some(node.id);
            visual.body.set_position(Position::new(r.x, r.y, layer(0.0)));
            visual.body.set_size(Size::new(r.w, r.h));
            visual.body.border_color = if selected {
                theme::ACCENT
            } else if node.problem {
                theme::DANGER
            } else if hovered {
                theme::BORDER_STRONG
            } else {
                theme::BORDER
            };
            visual.body.border_thickness = if selected || node.problem { 2.0 } else { 1.0 };
            visual.bar.set_position(Position::new(r.x, r.y, layer(1.0)));
            visual.bar.set_size(Size::new(r.w, TITLE_H * zoom));
            visual.bar.set_color(node.color);
            let pr = PIN_RADIUS * zoom.max(0.7);
            for ((side, pin, plane, label), (cx, cy)) in visual.pins.iter_mut().zip(pin_centers) {
                plane.set_position(Position::new(cx - pr, cy - pr, layer(2.0)));
                plane.set_size(Size::new(pr * 2.0, pr * 2.0));
                let lit = hover_pin == Some((node.id, *side, *pin));
                plane.set_color(if lit { theme::ACCENT } else { theme::BG_FIELD });
                if let Some(label) = label {
                    if zoom_changed {
                        label.set_font(Fonts::Inter, (11.0 * zoom).clamp(6.0, 20.0));
                    }
                    let w = label.measure_width();
                    let lx = match side {
                        Side::In => cx + pr + 4.0 * zoom,
                        Side::Out => cx - pr - 4.0 * zoom - w,
                    };
                    label.prepare_render_at(Position::new(lx, cy - 7.0 * zoom, layer(3.0)));
                    label.set_z_index(layer(3.0));
                }
            }
            if zoom_changed {
                visual.title.set_font(Fonts::InterSemiBold, (13.0 * zoom).clamp(7.0, 26.0));
                visual.summary.set_font(Fonts::Inter, (12.0 * zoom).clamp(6.0, 22.0));
                visual.count.set_font(Fonts::Inter, (12.0 * zoom).clamp(6.0, 22.0));
            }
            visual.title.set_content(&node.title);
            visual.title.set_color(theme::TEXT);
            visual.title.prepare_render_at(Position::new(r.x + 12.0 * zoom, r.y + 5.0 * zoom, layer(3.0)));
            visual.summary.set_content(&node.summary);
            visual.summary.prepare_render_at(Position::new(r.x + 16.0 * zoom, r.y + summary_y * zoom, layer(3.0)));
            visual.count.set_content(&node.note.0);
            visual.count.set_color(node.note.1);
            visual.count.prepare_render_at(Position::new(r.x + 16.0 * zoom, r.y + note_y * zoom, layer(3.0)));
            for text in [&mut visual.title, &mut visual.summary, &mut visual.count] {
                text.set_z_index(layer(3.0));
            }
        }
        if zoom_changed {
            self.zoom_applied = zoom;
        }
        if let Some(menu) = &mut self.menu {
            menu.layout();
        }
        // Tooltips: the add menu's highlighted kind beside the menu, or the
        // node the cursor has rested on.
        self.tip.shown = false;
        let bounds = self.area();
        if let Some(menu) = &self.menu {
            if let Some((_, entry)) = menu.choices().get(menu.index) {
                let r = menu.rect();
                let beside = if r.x + r.w + TIP_W + 8.0 <= bounds.x + bounds.w { r.x + r.w - 12.0 } else { r.x - TIP_W - 20.0 };
                self.tip.show(entry, (beside, r.y - 18.0), &bounds);
            }
        } else if self.drag.is_none() {
            match (self.hover_node, self.hover_since) {
                (Some(id), Some((since_id, since))) if id == since_id => {
                    if since.elapsed().as_secs_f32() >= TIP_DELAY {
                        if let Some(entry) = self.with_edit(|e| e.tip(id)).flatten() {
                            self.tip.show(&entry, self.cursor, &bounds);
                        }
                    }
                }
                (Some(id), _) => self.hover_since = Some((id, Instant::now())),
                (None, _) => self.hover_since = None,
            }
        }
        for button in &mut self.buttons {
            button.update(scene);
        }
    }

    fn render(&self) {
        if self.with_edit(|_| ()).is_none() {
            return;
        }
        let window = {
            let s = self.state.borrow();
            (s.layout.window_w, s.layout.window_h)
        };
        let area = self.area();
        self.background.render();
        clip::push(&self.background);

        // Grid.
        let step = GRID * self.zoom;
        if step >= 8.0 {
            let mut grid = Vec::new();
            let mut x = area.x + self.pan.0.rem_euclid(step);
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

        // Wires.
        let selected_wire = self.with_edit(|e| e.selected_wire()).flatten();
        let mut normal = Vec::new();
        let mut lit = Vec::new();
        for (wire, points) in self.wire_curves() {
            if Some(wire) == selected_wire || Some(wire) == self.hover_wire {
                lit.extend(polyline(&points));
            } else {
                normal.extend(polyline(&points));
            }
        }
        draw_lines(window, &normal, Vector3::new(0.55, 0.57, 0.62), 2);
        draw_lines(window, &lit, color3(theme::ACCENT), 3);
        if let Some(Drag::Wire { from }) = &self.drag {
            let start = self.view(from.node).map(|n| self.pin_center(n, Side::Out, from.pin));
            if let Some(start) = start {
                draw_lines(window, &polyline(&curve(start, self.cursor, self.zoom)), color3(theme::ACCENT_HOVER), 2);
            }
        }

        // Bottom to top, so blended edges fall over what lies below.
        for node in self.stacking.iter().filter_map(|&i| self.nodes.get(i)) {
            node.body.render();
            node.bar.render();
            for (_, _, plane, label) in &node.pins {
                plane.render();
                if let Some(label) = label {
                    label.render();
                }
            }
            node.title.render();
            node.summary.render();
            node.count.render();
        }

        // Selection box.
        if let Some(Drag::Select { start }) = &self.drag {
            let (a, b) = (*start, self.cursor);
            let lines = vec![seg(a, (b.0, a.1)), seg((b.0, a.1), b), seg(b, (a.0, b.1)), seg((a.0, b.1), a)];
            draw_lines(window, &lines, color3(theme::ACCENT), 1);
        }
        clip::pop(&self.background);

        self.header_bg.render();
        self.title.render();
        self.footer_bg.render();
        self.footer.render();
        for button in &self.buttons {
            button.render();
        }
    }

    fn render_overlay(&self) {
        for button in &self.buttons {
            button.render_overlay();
        }
        if let Some(menu) = &self.menu {
            menu.render();
        }
        self.tip.render();
    }

    fn handle_events(&mut self, scene: &mut Scene, window: &mut glfw::Window, glfw: &mut Glfw, event: &WindowEvent) -> bool {
        if self.with_edit(|_| ()).is_none() {
            return false;
        }
        // The add menu takes the keys and clicks while it is open.
        if self.menu.is_some() {
            match event {
                WindowEvent::Char(c) if !c.is_control() => {
                    let menu = self.menu.as_mut().expect("menu");
                    menu.filter.push(*c);
                    menu.index = 0;
                    return true;
                }
                WindowEvent::Key(key, _, Action::Press | Action::Repeat, _) => {
                    let menu = self.menu.as_mut().expect("menu");
                    let count = menu.choices().len().min(MENU_ROWS);
                    match key {
                        Key::Escape => self.menu = None,
                        Key::Backspace => {
                            menu.filter.pop();
                            menu.index = 0;
                        }
                        Key::Down => menu.index = (menu.index + 1).min(count.saturating_sub(1)),
                        Key::Up => menu.index = menu.index.saturating_sub(1),
                        Key::Enter | Key::KpEnter => {
                            let index = menu.index;
                            self.pick(index);
                        }
                        _ => {}
                    }
                    return true;
                }
                WindowEvent::Key(..) | WindowEvent::Char(..) => return true,
                WindowEvent::MouseButton(_, Action::Press, _) => {
                    let (x, y) = self.cursor;
                    let menu = self.menu.as_ref().expect("menu");
                    if menu.rect().contains(x, y) {
                        let row = (0..MENU_ROWS).find(|r| menu.row_rect(*r).contains(x, y));
                        if let Some(row) = row.filter(|r| *r < menu.choices().len()) {
                            self.pick(row);
                        }
                        return true;
                    }
                    self.menu = None;
                    // The click goes on to what is under it.
                }
                WindowEvent::CursorPos(x, y) => {
                    self.cursor = (*x as f32, *y as f32);
                    let menu = self.menu.as_mut().expect("menu");
                    if let Some(row) = (0..MENU_ROWS).find(|r| menu.row_rect(*r).contains(*x as f32, *y as f32)) {
                        if row < menu.choices().len() {
                            menu.index = row;
                        }
                    }
                    return false;
                }
                _ => {}
            }
        }
        for button in &mut self.buttons {
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
                    Some(Drag::Move { start, recorded }) => {
                        let (gx, gy) = ((x - previous.0) / self.zoom, (y - previous.1) / self.zoom);
                        let first = !*recorded;
                        *recorded = true;
                        let _ = start;
                        self.with_edit_mut(|e| {
                            if first {
                                e.begin_move();
                            }
                            e.move_selection([gx, gy]);
                        });
                        return true;
                    }
                    Some(Drag::Wire { .. } | Drag::Select { .. }) => {
                        self.hover_pin = self.pin_at(x, y);
                        return true;
                    }
                    None => {}
                }
                if area.contains(x, y) {
                    self.hover_pin = self.pin_at(x, y);
                    self.hover_node = self.node_at(x, y);
                    self.hover_wire = if self.hover_node.is_none() && self.hover_pin.is_none() { self.wire_at(x, y) } else { None };
                } else {
                    self.hover_pin = None;
                    self.hover_node = None;
                    self.hover_wire = None;
                }
                false
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, mods) => {
                let (x, y) = self.cursor;
                // A click puts the tooltip away until the cursor rests again.
                self.hover_since = self.hover_node.map(|id| (id, Instant::now()));
                if !area.contains(x, y) {
                    return rect.contains(x, y);
                }
                let now = Instant::now();
                let double = self.last_click.is_some_and(|t| now.duration_since(t).as_secs_f32() < DOUBLE_CLICK);
                self.last_click = Some(now);
                let ctrl = mods.contains(Modifiers::Control);
                let shift = mods.contains(Modifiers::Shift);

                match self.pin_at(x, y) {
                    Some((node, Side::Out, pin)) => {
                        self.drag = Some(Drag::Wire { from: PinRef { node, pin } });
                        return true;
                    }
                    Some((node, Side::In, pin)) => {
                        // Pick up the wire in this input, if any, to put it
                        // elsewhere or drop it.
                        let to = PinRef { node, pin };
                        if let Some(from) = self.with_edit_mut(|e| e.unwire_into(to)).flatten() {
                            self.drag = Some(Drag::Wire { from });
                        }
                        return true;
                    }
                    None => {}
                }
                if let Some(id) = self.node_at(x, y) {
                    let g = self.to_graph((x, y));
                    self.with_edit_mut(|e| {
                        if ctrl {
                            let mut selection = e.selection();
                            match selection.iter().position(|s| *s == id) {
                                Some(i) => {
                                    selection.remove(i);
                                }
                                None => selection.push(id),
                            }
                            e.select(selection);
                        } else if !e.selection().contains(&id) {
                            e.select(vec![id]);
                        }
                    });
                    self.drag = Some(Drag::Move { start: g, recorded: false });
                    return true;
                }
                if let Some(wire) = self.wire_at(x, y) {
                    self.with_edit_mut(|e| e.select_wire(Some(wire)));
                    return true;
                }
                if double {
                    self.open_menu((x, y));
                    return true;
                }
                if shift {
                    self.drag = Some(Drag::Select { start: (x, y) });
                } else {
                    self.with_edit_mut(|e| e.select(Vec::new()));
                    self.drag = Some(Drag::Pan { last: (x, y) });
                }
                true
            }
            WindowEvent::MouseButton(MouseButton::Button2, Action::Press, _) => {
                let (x, y) = self.cursor;
                if !area.contains(x, y) {
                    return false;
                }
                if self.node_at(x, y).is_none() && self.pin_at(x, y).is_none() {
                    self.open_menu((x, y));
                } else {
                    self.drag = Some(Drag::Pan { last: (x, y) });
                }
                true
            }
            WindowEvent::MouseButton(MouseButton::Button3, Action::Press, _) => {
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
                let (x, y) = self.cursor;
                match drag {
                    Drag::Wire { from } => {
                        if let Some((node, Side::In, pin)) = self.pin_at(x, y) {
                            let to = PinRef { node, pin };
                            let result = self.with_edit_mut(|e| e.wire(from, to));
                            if let Some(Err(e)) = result {
                                self.state.borrow_mut().set_status(tr!("graph_canvas.cannot_wire", error = e));
                            }
                        }
                    }
                    Drag::Select { start } => {
                        let (x0, x1) = (start.0.min(x), start.0.max(x));
                        let (y0, y1) = (start.1.min(y), start.1.max(y));
                        let inside: Vec<NodeId> = self
                            .views
                            .iter()
                            .filter(|n| {
                                let r = self.node_rect(n);
                                r.x < x1 && r.x + r.w > x0 && r.y < y1 && r.y + r.h > y0
                            })
                            .map(|n| n.id)
                            .collect();
                        self.with_edit_mut(|e| e.select(inside));
                    }
                    Drag::Pan { .. } | Drag::Move { .. } => {}
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
        for button in &mut self.buttons {
            button.set_z_index(Z + 3.0);
        }
    }
}
