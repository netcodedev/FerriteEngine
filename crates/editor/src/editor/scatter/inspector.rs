//! The scatter graph workspace's inspector: the selected node's settings
//! (`NodeKind::properties`), edited in place as one undo step per change.

use ferrite::{
    core::{
        entity::component::{Property, PropertyValue},
        primitives::{Offset, Position, Size},
        renderer::text::icons,
        scene::Scene,
        utils::DataSource,
    },
    terrain::scatter::graph::{NodeId, NodeKind},
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

use super::super::{
    field_row::{FieldRow, LABEL_COLUMN},
    layout::Rect,
    send, Command, Shared,
};

/// One setting's widget source and the value last written to it.
enum Field {
    Float { source: DataSource<f32>, last: f32 },
    Int { source: DataSource<i64>, last: i64 },
    Bool { source: DataSource<bool>, last: bool },
    Text { source: DataSource<String>, last: String },
    Choice { source: DataSource<u32>, last: u32, options: Vec<String> },
}

struct Binding {
    name: String,
    field: Field,
}

#[derive(Clone, PartialEq)]
struct BuiltFor {
    node: Option<NodeId>,
    /// The names of the node's rows.
    properties: Vec<String>,
    structure: u64,
    width: i32,
}

pub struct ScatterInspector {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    bindings: Vec<Binding>,
    built_for: Option<BuiltFor>,
    /// The setting last changed and when: changes to the same one in quick
    /// succession (dragging a slider) are one undo step.
    last_change: Option<(NodeId, String, std::time::Instant)>,
}

/// Changes to one setting this close together are one undo step.
const ONE_STEP: std::time::Duration = std::time::Duration::from_millis(700);

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

impl ScatterInspector {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new(tr!("scatter.inspector.title").into(), Position::default(), Size::new(330.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 300.0));
        body.set_gap(4.0);
        Self { handle: UIElementHandle::new(), state, panel, body, bindings: Vec::new(), built_for: None, last_change: None }
    }

    /// Hierarchy and inspector together, as the asset workspace uses them.
    fn rect(&self) -> Rect {
        let s = self.state.borrow();
        let (h, i) = (s.layout.hierarchy, s.layout.inspector);
        Rect { x: h.x.min(i.x), y: h.y.min(i.y), w: h.w.max(i.w), h: (i.y + i.h).max(h.y + h.h) - h.y.min(i.y) }
    }

    /// The node the inspector shows: the one selected, if just one is.
    fn shown(&self) -> Option<(NodeId, NodeKind)> {
        let s = self.state.borrow();
        let edit = s.scatter.as_ref()?;
        match edit.selection.as_slice() {
            [id] => edit.graph.node(*id).map(|n| (n.id, n.kind.clone())),
            _ => None,
        }
    }

    fn rebuild(&mut self) {
        self.body.clear();
        self.bindings.clear();
        let width = (self.rect().w - 24.0).max(220.0);
        let Some((id, kind)) = self.shown() else {
            let (selected, problems) = {
                let s = self.state.borrow();
                s.scatter.as_ref().map_or((0, Vec::new()), |e| (e.selection.len(), e.graph.problems()))
            };
            let lines: Vec<String> = if selected > 1 {
                vec![tr!("scatter.inspector.nodes_selected", count = selected), tr!("scatter.inspector.select_one").into()]
            } else {
                vec![tr!("scatter.inspector.select_node").into(), tr!("scatter.inspector.right_click_add").into()]
            };
            for line in lines {
                self.body.add_child(Box::new(Label::muted(&line, theme::FONT_SIZE)));
            }
            for problem in problems {
                let mut label = Label::new(&problem.to_string(), theme::FONT_SIZE_SMALL);
                label.set_color(theme::DANGER);
                self.body.add_child(Box::new(label));
            }
            self.body.set_z_index(2.0);
            return;
        };
        let mut section = Section::new(kind.label(), Some(icons::SLIDERS), width);
        let value_w = width - LABEL_COLUMN - 16.0;
        for property in kind.properties() {
            // A spawn mesh's model is picked by a button behind its name.
            let mut extra: Vec<Box<dyn UIElement>> = Vec::new();
            if matches!(kind, NodeKind::SpawnMesh(_)) && property.name == "model" {
                let state = self.state.clone();
                let mut button = Button::new(
                    Position::default(),
                    Size::new(10.0, 24.0),
                    Box::new(move |_| send(&state, Command::PickScatterModel(id))),
                );
                button.set_style(ButtonStyle::Secondary);
                button.set_icon(icons::PACKAGE);
                button.set_label(tr!("scatter.inspector.choose"));
                button.set_tooltip(tr!("scatter.inspector.choose_tip"));
                button.fit(24.0);
                extra.push(Box::new(button));
            }
            if let Some(row) = self.add_property(property, value_w, extra) {
                section.add_child(row);
            }
        }
        self.body.add_child(Box::new(section));
        // Graphics settings never change what is planted over the
        // landscape (docs/graphics-settings.md): say so where a value is
        // bound in vain.
        let ignored = {
            let s = self.state.borrow();
            s.scatter.as_ref().is_some_and(|e| e.graph.bindings_ignored().contains(&id))
        };
        if ignored {
            let mut label = Label::new(
                tr!("scatter.inspector.landscape_ignored"),
                theme::FONT_SIZE_SMALL,
            );
            label.set_color(theme::WARNING);
            self.body.add_child(Box::new(label));
        }
        self.body.set_z_index(2.0);
    }

    /// A row for `property`, with `extra` widgets behind its value.
    fn add_property(&mut self, property: Property, value_w: f32, extra: Vec<Box<dyn UIElement>>) -> Option<Box<dyn UIElement>> {
        let name = property.name.to_string();
        let mut widgets: Vec<Box<dyn UIElement>> = if property.read_only {
            let mut label = Label::new(&display(&property.value), theme::FONT_SIZE_SMALL + 0.5);
            label.set_color(theme::TEXT_MUTED);
            vec![Box::new(label)]
        } else {
            let (widgets, field): (Vec<Box<dyn UIElement>>, Field) = match &property.value {
                PropertyValue::Float(v) => {
                    let source = DataSource::new(round3(*v));
                    let widgets: Vec<Box<dyn UIElement>> = match property.range {
                        Some((min, max)) => vec![
                            UI::slider_sized(source.clone(), min, max, Size::new(value_w - 74.0, 20.0)),
                            UI::input_sized(source.clone(), Size::new(66.0, 24.0)),
                        ],
                        None => vec![UI::input_sized(source.clone(), Size::new(value_w.min(140.0), 24.0))],
                    };
                    (widgets, Field::Float { source, last: round3(*v) })
                }
                PropertyValue::Int(v) => {
                    let source = DataSource::new(*v);
                    (vec![UI::input_sized(source.clone(), Size::new(value_w.min(140.0), 24.0))], Field::Int { source, last: *v })
                }
                PropertyValue::Bool(v) => {
                    let source = DataSource::new(*v);
                    (vec![UI::checkbox(source.clone())], Field::Bool { source, last: *v })
                }
                PropertyValue::Text(v) if property.options.is_some() => {
                    let options = property.options.clone().unwrap_or_default();
                    let index = options.iter().position(|o| o == v).unwrap_or(0) as u32;
                    let source = DataSource::new(index);
                    let dropdown = Dropdown::new(Position::default(), Size::new(value_w, 24.0), options.clone(), source.clone());
                    (vec![Box::new(dropdown)], Field::Choice { source, last: index, options })
                }
                PropertyValue::Text(v) => {
                    let source = DataSource::new(v.clone());
                    (vec![UI::input_sized(source.clone(), Size::new(value_w, 24.0))], Field::Text { source, last: v.clone() })
                }
            };
            self.bindings.push(Binding { name: name.clone(), field });
            widgets
        };
        widgets.extend(extra);
        Some(Box::new(FieldRow::new(&name, widgets)))
    }

    /// Hand what was typed to the node, one undo step per change; and show
    /// what the node holds now (an undo, an MCP edit).
    fn sync(&mut self) {
        let Some((id, _)) = self.shown() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let Some(edit) = state.scatter.as_mut() else {
            return;
        };
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
            };
            if let Some(value) = typed {
                let mut kind = match edit.graph.node(id) {
                    Some(node) => node.kind.clone(),
                    None => return,
                };
                if kind.set_property(&binding.name, &value) {
                    let now = std::time::Instant::now();
                    let same = self
                        .last_change
                        .as_ref()
                        .is_some_and(|(node, name, at)| *node == id && *name == binding.name && now.duration_since(*at) < ONE_STEP);
                    if !same {
                        edit.record();
                    }
                    self.last_change = Some((id, binding.name.clone(), now));
                    if let Some(node) = edit.graph.node_mut(id) {
                        node.kind = kind;
                    }
                    // A changed range shows or hides the distance.
                    if binding.name == "range" {
                        edit.touch_structure();
                    } else {
                        edit.touch_rules();
                    }
                }
                continue;
            }
            // Not typed: show the node's value (it may have changed).
            let Some(current) = edit
                .graph
                .node(id)
                .and_then(|n| n.kind.properties().into_iter().find(|p| p.name == binding.name))
                .map(|p| p.value)
            else {
                continue;
            };
            match (&mut binding.field, current) {
                (Field::Float { source, last }, PropertyValue::Float(v)) if (round3(v) - *last).abs() > 1e-6 => {
                    *last = round3(v);
                    source.write(*last);
                }
                (Field::Int { source, last }, PropertyValue::Int(v)) if v != *last => {
                    *last = v;
                    source.write(v);
                }
                (Field::Bool { source, last }, PropertyValue::Bool(v)) if v != *last => {
                    *last = v;
                    source.write(v);
                }
                (Field::Text { source, last }, PropertyValue::Text(v)) if v != *last => {
                    *last = v.clone();
                    source.write(v);
                }
                (Field::Choice { source, last, options }, PropertyValue::Text(v)) => {
                    let index = options.iter().position(|o| *o == v).unwrap_or(0) as u32;
                    if index != *last {
                        *last = index;
                        source.write(index);
                    }
                }
                _ => {}
            }
        }
    }
}

fn display(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Float(v) => format!("{v:.3}"),
        PropertyValue::Int(v) => v.to_string(),
        PropertyValue::Bool(v) => if *v { tr!("common.yes") } else { tr!("common.no") }.into(),
        PropertyValue::Text(v) => v.clone(),
    }
}

impl UIElement for ScatterInspector {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        let wanted = {
            let s = self.state.borrow();
            BuiltFor {
                node: self.shown().map(|(id, _)| id),
                // Binding a value to a setting changes the rows shown.
                properties: self
                    .shown()
                    .map(|(_, kind)| kind.properties().into_iter().map(|p| p.name.to_string()).collect())
                    .unwrap_or_default(),
                structure: s.scatter.as_ref().map_or(0, |e| e.structure),
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
