//! The material workspace's inspector: a material's settings and
//! parameters; for a graph, the selected node's settings, or with nothing
//! selected the graph's parameters (added, renamed, typed, removed here).

use ferrite::core::{
    entity::component::{Property, PropertyValue},
    material::{
        asset::GraphRef,
        edit,
        graph::{NodeId, ParamType},
    },
    primitives::{Offset, Position, Size},
    renderer::text::icons,
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        dropdown::Dropdown,
        label::Label,
        panel::Panel,
        scroll::ScrollArea,
        section::Section,
    },
    theme, UIElement, UI,
};
use glfw::{Glfw, WindowEvent};

use super::{
    super::{
        drag::{DropAction, DropTarget},
        field_row::FieldRow,
        layout::Rect,
        send, Command, Shared,
    },
    Doc, MaterialEdit,
};

enum Field {
    Float { source: DataSource<f32>, last: f32 },
    Int { source: DataSource<i64>, last: i64 },
    Bool { source: DataSource<bool>, last: bool },
    Text { source: DataSource<String>, last: String },
    Choice { source: DataSource<u32>, last: u32, options: Vec<String> },
    /// Numbers of a vector, one field each (written as "x y z").
    Vector { sources: Vec<DataSource<f32>>, last: Vec<f32> },
    /// sRGB colour and alpha, 0..1 (written as "#rrggbbaa").
    Color { sources: [DataSource<f32>; 4], last: [f32; 4] },
}

/// Narrowest label column; it grows to fit the longest label shown.
const LABEL_W: f32 = 110.0;

/// How a text value is edited.
#[derive(Clone, Copy, PartialEq)]
enum TextKind {
    Plain,
    Vector,
    Color,
}

/// The numbers of a vector text ("1 0.5 0").
fn numbers_of(text: &str) -> Option<Vec<f32>> {
    let parts: Vec<f32> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().ok())
        .collect::<Option<Vec<f32>>>()?;
    (!parts.is_empty() && parts.len() <= 4).then_some(parts)
}

/// sRGB channels (0..1) and alpha of "#rrggbb(aa)".
fn rgba_of(text: &str) -> Option<[f32; 4]> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|b| b as f32 / 255.0);
    Some([byte(0)?, byte(2)?, byte(4)?, if hex.len() == 8 { byte(6)? } else { 1.0 }])
}

fn hex_of(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if b(c[3]) == 255 {
        format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]), b(c[3]))
    }
}

fn vector_text(v: &[f32]) -> String {
    v.iter().map(|x| format!("{}", (x * 10000.0).round() / 10000.0)).collect::<Vec<_>>().join(" ")
}

struct Binding {
    name: String,
    field: Field,
}

/// What a row is for, besides its value.
#[derive(Clone, PartialEq)]
enum Extra {
    None,
    /// A texture parameter: a choose button, and a drop target.
    Texture(String),
    /// The material's graph, when it is an asset: an edit button.
    OpenGraph(ferrite::core::assets::AssetId),
    /// A graph parameter: a remove button.
    RemoveParam(String),
}

/// The rows shown: section, property, extra.
type Rows = Vec<(String, Property, Extra)>;

#[derive(Clone, PartialEq)]
struct BuiltFor {
    asset: Option<ferrite::core::assets::AssetId>,
    node: Option<NodeId>,
    names: Vec<String>,
    structure: u64,
    width: i32,
}

pub struct MaterialInspector {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    bindings: Vec<Binding>,
    built_for: Option<BuiltFor>,
    /// Rows a texture asset can be dropped on.
    texture_rows: Vec<(UIElementHandle, String)>,
    /// The label column, as wide as the longest label shown.
    label_w: f32,
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// The node shown: the one selected, if just one is.
fn shown_node(edit: &MaterialEdit) -> Option<NodeId> {
    match edit.selection.as_slice() {
        [id] if edit.graph().is_some_and(|g| g.node(*id).is_some()) => Some(*id),
        _ => None,
    }
}

fn prop(name: String, value: PropertyValue, options: Option<Vec<String>>) -> Property {
    Property { name: name.into(), value, read_only: false, range: None, options, action: false }
}

/// A graph parameter's rows (`param:<name>:<field>`).
fn param_rows(edit: &MaterialEdit) -> Rows {
    let Some(graph) = edit.graph() else { return Vec::new() };
    let types: Vec<String> = ParamType::ALL.iter().map(|t| edit::param_type_name(*t).to_string()).collect();
    let mut rows = Vec::new();
    for (index, p) in graph.params.iter().enumerate() {
        let section = format!("Parameter {}: {}", index + 1, p.display());
        let key = |field: &str| format!("param:{index}:{field}");
        rows.push((section.clone(), prop(key("name"), PropertyValue::Text(p.name.clone()), None), Extra::RemoveParam(p.name.clone())));
        rows.push((
            section.clone(),
            prop(key("type"), PropertyValue::Text(edit::param_type_name(p.ty).into()), Some(types.clone())),
            Extra::None,
        ));
        let default = match p.ty {
            ParamType::Texture { .. } => None,
            ParamType::Color => Some(PropertyValue::Text(edit::color_text(p.default))),
            ParamType::Float => Some(PropertyValue::Float(p.default[0])),
            ParamType::Bool => Some(PropertyValue::Bool(p.default[0] > 0.5)),
            ParamType::Vec2 => Some(PropertyValue::Text(edit::vector_text(p.default, 2))),
            ParamType::Vec3 => Some(PropertyValue::Text(edit::vector_text(p.default, 3))),
        };
        if let Some(default) = default {
            rows.push((section.clone(), prop(key("default"), default, None), Extra::None));
        }
        rows.push((section.clone(), prop(key("label"), PropertyValue::Text(p.label.clone()), None), Extra::None));
        rows.push((section, prop(key("group"), PropertyValue::Text(p.group.clone()), None), Extra::None));
    }
    rows
}

/// What the inspector shows for `edit`.
fn rows(edit: &MaterialEdit) -> Rows {
    match &edit.doc {
        Doc::Material(file) => {
            let graph = edit.settings_graph();
            edit::material_properties(file, &graph)
                .into_iter()
                .map(|(section, p)| {
                    let extra = if p.name == "graph" {
                        match file.graph {
                            GraphRef::Asset(id) => Extra::OpenGraph(id),
                            GraphRef::Builtin(_) => Extra::None,
                        }
                    } else if graph.param(&p.name).is_some_and(|d| matches!(d.ty, ParamType::Texture { .. })) {
                        Extra::Texture(p.name.to_string())
                    } else {
                        Extra::None
                    };
                    (section, p, extra)
                })
                .collect()
        }
        Doc::Graph(graph) => match shown_node(edit) {
            Some(id) => {
                let title = graph.node(id).map(|n| n.kind.label()).unwrap_or_default();
                edit::node_properties(graph, id).into_iter().map(|p| (title.clone(), p, Extra::None)).collect()
            }
            None => param_rows(edit),
        },
    }
}

/// Hand a typed value to the document.
fn apply(edit: &mut MaterialEdit, name: &str, value: &PropertyValue) -> Result<(), String> {
    if let Some(rest) = name.strip_prefix("param:") {
        let (index, field) = rest.split_once(':').ok_or("bad row")?;
        let index: usize = index.parse().map_err(|_| "bad row")?;
        let param = edit.graph().and_then(|g| g.params.get(index)).map(|p| p.name.clone()).ok_or("no such parameter")?;
        let value_json = match value {
            PropertyValue::Text(t) => serde_json::json!(t),
            PropertyValue::Float(f) => serde_json::json!(f.to_string()),
            PropertyValue::Int(i) => serde_json::json!(i.to_string()),
            PropertyValue::Bool(b) => serde_json::json!(b),
        };
        let op = match field {
            "name" => serde_json::json!({"op": "set_param", "name": param, "new_name": value_json}),
            "type" => serde_json::json!({"op": "set_param", "name": param, "type": value_json}),
            "default" => serde_json::json!({"op": "set_param", "name": param, "default": value_json}),
            "group" => serde_json::json!({"op": "set_param", "name": param, "group": value_json}),
            "label" => serde_json::json!({"op": "set_param", "name": param, "label": value_json}),
            _ => return Err("bad row".into()),
        };
        return edit.apply_ops(&[op]).map(|_| ());
    }
    match &edit.doc {
        Doc::Material(_) => edit.set_material_property(name, value),
        Doc::Graph(_) => {
            let id = shown_node(edit).ok_or("no node")?;
            edit.set_node_property(id, name, value)
        }
    }
}

impl MaterialInspector {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Material".into(), Position::default(), Size::new(330.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 300.0));
        body.set_gap(4.0);
        Self { handle: UIElementHandle::new(), state, panel, body, bindings: Vec::new(), built_for: None, texture_rows: Vec::new(), label_w: LABEL_W }
    }

    fn rect(&self) -> Rect {
        let s = self.state.borrow();
        let (h, i) = (s.layout.hierarchy, s.layout.inspector);
        Rect { x: h.x.min(i.x), y: h.y.min(i.y), w: h.w.max(i.w), h: (i.y + i.h).max(h.y + h.h) - h.y.min(i.y) }
    }

    fn button(&self, icon: &str, label: &str, tooltip: &str, command: Command) -> Box<dyn UIElement> {
        let state = self.state.clone();
        let mut button = Button::new(Position::default(), Size::new(24.0, 24.0), Box::new(move |_| send(&state, command.clone())));
        button.set_style(ButtonStyle::Secondary);
        button.set_icon(icon);
        if !label.is_empty() {
            button.set_label(label);
            button.fit(24.0);
        }
        button.set_tooltip(tooltip);
        Box::new(button)
    }

    fn rebuild(&mut self) {
        self.body.clear();
        self.bindings.clear();
        self.texture_rows.clear();
        let width = (self.rect().w - 24.0).max(220.0);
        // The label column fits the longest label (at most half the panel).
        let longest = {
            let s = self.state.borrow();
            let graph = s.material.as_ref().map(|e| e.settings_graph()).unwrap_or_default();
            let shown = s.material.as_ref().map(rows).unwrap_or_default();
            shown
                .iter()
                .map(|(_, p, _)| {
                    theme::text(&edit::property_label(&p.name, &graph), theme::FONT_SIZE_SMALL + 0.5, theme::TEXT_MUTED)
                        .measure_width()
                })
                .fold(0.0f32, f32::max)
        };
        self.label_w = (longest + 14.0).clamp(LABEL_W, width * 0.5);
        let value_w = width - self.label_w - 16.0;
        let (rows, problems, is_graph, selected, saved_note) = {
            let s = self.state.borrow();
            let Some(edit) = s.material.as_ref() else {
                return;
            };
            (
                rows(edit),
                edit.problems(),
                edit.is_graph(),
                edit.selection.len(),
                edit.changed_on_disk,
            )
        };
        // Header buttons: save, revert, back.
        let header: Vec<Box<dyn UIElement>> = vec![
            self.button(icons::SAVE, "Save", "Save (Ctrl+S)", Command::SaveMaterial),
            self.button(icons::ROTATE_CW, "Revert", "Back to the saved file", Command::RevertMaterial),
            self.button(icons::ARROW_LEFT, "Back", "Back to the scene", Command::CloseMaterialEditor),
        ];
        self.body.add_child(Box::new(FieldRow::with_label_width("", 0.0, header)));
        if saved_note {
            let mut label = Label::new("The file was changed elsewhere; Revert takes it up.", theme::FONT_SIZE_SMALL);
            label.set_color(theme::WARNING);
            self.body.add_child(Box::new(label));
        }
        if is_graph && selected > 1 {
            self.body.add_child(Box::new(Label::muted(&format!("{selected} nodes selected. Select one to edit it."), theme::FONT_SIZE)));
        }
        let mut section: Option<Section> = None;
        let mut section_title = String::new();
        for (title, property, extra) in rows {
            if section.is_none() || title != section_title {
                if let Some(done) = section.take() {
                    self.body.add_child(Box::new(done));
                }
                section = Some(Section::new(&title, Some(icons::SLIDERS), width));
                section_title = title.clone();
            }
            let row = self.add_property(property, value_w, &extra);
            if let Extra::Texture(param) = &extra {
                self.texture_rows.push((*row.get_handle(), param.clone()));
            }
            section.as_mut().expect("made above").add_child(row);
        }
        if let Some(done) = section.take() {
            self.body.add_child(Box::new(done));
        }
        if is_graph && selected == 0 {
            let add = self.button(icons::PLUS, "Add parameter", "A new parameter materials set", Command::MaterialAddParam);
            self.body.add_child(Box::new(FieldRow::with_label_width("", 0.0, vec![add])));
            self.body.add_child(Box::new(Label::muted("Select a node to edit it; right-click the canvas to add one.", theme::FONT_SIZE_SMALL)));
        }
        for problem in problems {
            let mut label = Label::new(&problem, theme::FONT_SIZE_SMALL);
            label.set_color(theme::DANGER);
            self.body.add_child(Box::new(label));
        }
        self.body.set_z_index(2.0);
    }

    /// How a text row is edited: vectors and colours by their parts.
    fn text_kind(&self, name: &str, value: &str) -> TextKind {
        let vector_row = name == "value"
            || name.starts_with("in:")
            || name.ends_with(":default")
            || self.state.borrow().material.as_ref().is_some_and(|e| {
                !e.is_graph()
                    && e.settings_graph().param(name).is_some_and(|d| {
                        matches!(d.ty, ParamType::Vec2 | ParamType::Vec3 | ParamType::Color)
                    })
            });
        if !vector_row {
            TextKind::Plain
        } else if rgba_of(value).is_some() {
            TextKind::Color
        } else if numbers_of(value).is_some() {
            TextKind::Vector
        } else {
            TextKind::Plain
        }
    }

    fn add_property(&mut self, property: Property, value_w: f32, extra: &Extra) -> Box<dyn UIElement> {
        let name = property.name.to_string();
        let label = {
            let s = self.state.borrow();
            let graph = s.material.as_ref().map(|e| e.settings_graph()).unwrap_or_default();
            edit::property_label(&name, &graph)
        };
        let extra_w = match extra {
            Extra::None => 0.0,
            _ => 30.0,
        };
        let w = (value_w - extra_w).max(60.0);
        let (mut widgets, field): (Vec<Box<dyn UIElement>>, Field) = match &property.value {
            PropertyValue::Float(v) => {
                let source = DataSource::new(round3(*v));
                let widgets: Vec<Box<dyn UIElement>> = match property.range {
                    Some((min, max)) => vec![
                        UI::slider_sized(source.clone(), min, max, Size::new(w - 74.0, 20.0)),
                        UI::input_sized(source.clone(), Size::new(66.0, 24.0)),
                    ],
                    None => vec![UI::input_sized(source.clone(), Size::new(w.min(140.0), 24.0))],
                };
                (widgets, Field::Float { source, last: round3(*v) })
            }
            PropertyValue::Int(v) => {
                let source = DataSource::new(*v);
                (vec![UI::input_sized(source.clone(), Size::new(w.min(140.0), 24.0))], Field::Int { source, last: *v })
            }
            PropertyValue::Bool(v) => {
                let source = DataSource::new(*v);
                (vec![UI::checkbox(source.clone())], Field::Bool { source, last: *v })
            }
            PropertyValue::Text(v) if property.options.is_some() => {
                let options = property.options.clone().unwrap_or_default();
                let index = options.iter().position(|o| o == v).unwrap_or(0) as u32;
                let source = DataSource::new(index);
                let dropdown = Dropdown::new(Position::default(), Size::new(w, 24.0), options.clone(), source.clone());
                (vec![Box::new(dropdown)], Field::Choice { source, last: index, options })
            }
            PropertyValue::Text(v) => match self.text_kind(&name, v) {
                TextKind::Color => {
                    let c = rgba_of(v).unwrap_or([1.0; 4]);
                    let sources = c.map(DataSource::new);
                    let [r, g, b, a] = [0, 1, 2, 3].map(|i| sources[i].clone());
                    (vec![UI::color_picker(w, r, g, b, a) as Box<dyn UIElement>], Field::Color { sources, last: c })
                }
                TextKind::Vector => {
                    let numbers = numbers_of(v).unwrap_or_default();
                    let sources: Vec<DataSource<f32>> = numbers.iter().map(|x| DataSource::new(*x)).collect();
                    let each = ((w - 4.0 * (sources.len() as f32 - 1.0)) / sources.len() as f32).max(36.0);
                    let widgets = sources
                        .iter()
                        .map(|s| UI::input_sized(s.clone(), Size::new(each, 24.0)) as Box<dyn UIElement>)
                        .collect();
                    (widgets, Field::Vector { sources, last: numbers })
                }
                TextKind::Plain => {
                    let source = DataSource::new(v.clone());
                    (vec![UI::input_sized(source.clone(), Size::new(w, 24.0))], Field::Text { source, last: v.clone() })
                }
            },
        };
        self.bindings.push(Binding { name, field });
        match extra {
            Extra::None => {}
            Extra::Texture(param) => widgets.push(self.button(
                icons::LAYOUT_GRID,
                "",
                "Choose a texture (or drop one here): automatic, none, or a texture asset",
                Command::PickMaterialTexture { param: param.clone() },
            )),
            Extra::OpenGraph(id) => widgets.push(self.button(
                icons::PENCIL,
                "",
                "Edit this graph",
                Command::EditAsset { id: *id, variant: None },
            )),
            Extra::RemoveParam(name) => widgets.push(self.button(
                icons::TRASH,
                "",
                "Remove this parameter (nodes reading it then show a problem)",
                Command::MaterialRemoveParam(name.clone()),
            )),
        }
        Box::new(FieldRow::with_label_width(&label, self.label_w, widgets))
    }

    /// Hand what was typed to the document, and show what it holds now (an
    /// undo, an MCP edit).
    fn sync(&mut self) {
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.material.as_mut() else {
            return;
        };
        let current: Vec<(String, PropertyValue)> =
            rows(edit).into_iter().map(|(_, p, _)| (p.name.to_string(), p.value)).collect();
        let mut problem = None;
        for binding in &mut self.bindings {
            let typed = match &mut binding.field {
                Field::Float { source, last } => {
                    let v = source.read();
                    ((v - *last).abs() > 1e-6).then(|| {
                        *last = v;
                        PropertyValue::Float(v)
                    })
                }
                Field::Int { source, last } => {
                    let v = source.read();
                    (v != *last).then(|| {
                        *last = v;
                        PropertyValue::Int(v)
                    })
                }
                Field::Bool { source, last } => {
                    let v = source.read();
                    (v != *last).then(|| {
                        *last = v;
                        PropertyValue::Bool(v)
                    })
                }
                Field::Text { source, last } => {
                    let v = source.read();
                    (v != *last).then(|| {
                        *last = v.clone();
                        PropertyValue::Text(v)
                    })
                }
                Field::Choice { source, last, options } => {
                    let v = source.read();
                    (v != *last).then(|| {
                        *last = v;
                        PropertyValue::Text(options.get(v as usize).cloned().unwrap_or_default())
                    })
                }
                Field::Vector { sources, last } => {
                    let v: Vec<f32> = sources.iter().map(|s| s.read()).collect();
                    (v.iter().zip(last.iter()).any(|(a, b)| (a - b).abs() > 1e-6)).then(|| {
                        *last = v.clone();
                        PropertyValue::Text(vector_text(&v))
                    })
                }
                Field::Color { sources, last } => {
                    let v = [0, 1, 2, 3].map(|i| sources[i].read());
                    (v.iter().zip(last.iter()).any(|(a, b)| (a - b).abs() > 1e-4)).then(|| {
                        *last = v;
                        PropertyValue::Text(hex_of(v))
                    })
                }
            };
            if let Some(value) = typed {
                // A half-typed vector or name is no error yet: it is tried
                // again with every key.
                if let Err(e) = apply(edit, &binding.name, &value) {
                    problem = Some(e);
                }
                continue;
            }
            let Some((_, now)) = current.iter().find(|(n, _)| *n == binding.name) else { continue };
            match (&mut binding.field, now) {
                (Field::Float { source, last }, PropertyValue::Float(v)) if (round3(*v) - *last).abs() > 1e-6 => {
                    *last = round3(*v);
                    source.write(*last);
                }
                (Field::Int { source, last }, PropertyValue::Int(v)) if *v != *last => {
                    *last = *v;
                    source.write(*v);
                }
                (Field::Bool { source, last }, PropertyValue::Bool(v)) if *v != *last => {
                    *last = *v;
                    source.write(*v);
                }
                (Field::Choice { source, last, options }, PropertyValue::Text(v)) => {
                    let index = options.iter().position(|o| o == v).unwrap_or(0) as u32;
                    if index != *last {
                        *last = index;
                        source.write(index);
                    }
                }
                (Field::Vector { sources, last }, PropertyValue::Text(t)) => {
                    if let Some(v) = numbers_of(t).filter(|v| v.len() == last.len()) {
                        if v.iter().zip(last.iter()).any(|(a, b)| (a - b).abs() > 1e-4) {
                            for (s, x) in sources.iter().zip(&v) {
                                s.write(*x);
                            }
                            *last = v;
                        }
                    }
                }
                (Field::Color { sources, last }, PropertyValue::Text(t)) => {
                    if let Some(v) = rgba_of(t) {
                        if v.iter().zip(last.iter()).any(|(a, b)| (a - b).abs() > 2.0 / 255.0) {
                            for (s, x) in sources.iter().zip(v) {
                                s.write(x);
                            }
                            *last = v;
                        }
                    }
                }
                // Text is not overwritten while it may be being typed.
                _ => {}
            }
        }
        if let Some(problem) = problem {
            state.set_status(problem);
        }
    }

    /// Texture rows take texture assets dropped from the browser.
    fn register_drop_targets(&self) {
        let panel = self.rect();
        let mut targets = Vec::new();
        for (handle, param) in &self.texture_rows {
            let Some(row) = self.body.get_child(handle) else { continue };
            let rect = Rect { x: panel.x + 4.0, y: row.get_offset().y - 1.0, w: (panel.w - 8.0).max(1.0), h: row.get_size().height + 2.0 };
            if rect.y < panel.y || rect.y + rect.h > panel.y + panel.h {
                continue;
            }
            targets.push(DropTarget { rect, action: DropAction::MaterialTexture { param: param.clone() } });
        }
        self.state.borrow_mut().drop_targets.extend(targets);
    }
}

impl UIElement for MaterialInspector {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        let wanted = {
            let s = self.state.borrow();
            let edit = s.material.as_ref();
            BuiltFor {
                asset: edit.map(|e| e.asset),
                node: edit.and_then(shown_node),
                names: edit.map(|e| rows(e).into_iter().map(|(_, p, _)| p.name.to_string()).collect()).unwrap_or_default(),
                structure: edit.map_or(0, |e| e.structure),
                width: rect.w as i32,
            }
        };
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));
        let header = self.panel.header_height();
        self.body.set_position(Position::new(rect.x + 4.0, rect.y + header + 4.0, 0.0));
        self.body.set_size(Size::new((rect.w - 8.0).max(10.0), (rect.h - header - 8.0).max(10.0)));
        if self.built_for.as_ref() != Some(&wanted) {
            self.rebuild();
            self.built_for = Some(wanted);
        }
        self.sync();
        self.panel.update(scene);
        self.body.update(scene);
        self.register_drop_targets();
    }

    fn render(&self) {
        self.panel.render();
        self.body.render();
    }

    fn render_overlay(&self) {
        self.body.render_overlay();
    }

    fn handle_events(&mut self, scene: &mut Scene, window: &mut glfw::Window, glfw: &mut Glfw, event: &WindowEvent) -> bool {
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.rect().contains(x as f32, y as f32);
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
        self.panel.get_offset()
    }
    fn set_offset(&mut self, _offset: Offset) {}
    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }
    fn set_z_index(&mut self, z: f32) {
        self.panel.set_z_index(z);
        self.body.set_z_index(z + 2.0);
    }
}
