use ferrite::core::{
    model::animation_graph::graph_file::{ClipDef, CompareOp, Comparison, Term},
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
    super::{field_row::FieldRow, send, Command, Shared},
    animation_files, AnimSelection,
};

const NONE_LABEL: &str = "-";

enum Binding {
    StateName { source: DataSource<String>, last: String },
    Sync { source: DataSource<bool>, last: bool },
    ClipName { clip: usize, source: DataSource<String>, last: String },
    ClipFile { clip: usize, source: DataSource<u32>, last: u32, files: Vec<String> },
    Blend { source: DataSource<f32>, last: f32 },
    TermA { cond: usize, source: DataSource<u32>, last: u32 },
    Sign { cond: usize, source: DataSource<u32>, last: u32 },
    TermB { cond: usize, source: DataSource<u32>, last: u32 },
    Op { cond: usize, source: DataSource<u32>, last: u32 },
    Value { cond: usize, source: DataSource<f32>, last: f32 },
}

#[derive(Clone, PartialEq)]
struct BuiltFor {
    selection: AnimSelection,
    structure: u64,
    labels: u64,
}

/// Left-bottom panel of the animation workspace: edits the selected state
/// or transition.
pub struct GraphInspector {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    bindings: Vec<Binding>,
    built_for: Option<BuiltFor>,
}

fn dropdown(items: Vec<String>, selected: &DataSource<u32>, width: f32) -> Box<dyn UIElement> {
    Box::new(Dropdown::new(
        Position::default(),
        Size::new(width, 24.0),
        items,
        selected.clone(),
    ))
}

fn action_button(label: &str, icon: &str, style: ButtonStyle, on_click: Box<dyn Fn(&mut Scene)>) -> Box<dyn UIElement> {
    let mut button = Button::new(Position::default(), Size::new(10.0, 26.0), on_click);
    button.set_style(style);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(26.0);
    Box::new(button)
}

impl GraphInspector {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Inspector".into(), Position::default(), Size::new(330.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 300.0));
        body.set_gap(4.0);
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            bindings: Vec::new(),
            built_for: None,
        }
    }

    fn rebuild(&mut self) {
        self.body.clear();
        self.bindings.clear();
        let (selection, width) = {
            let s = self.state.borrow();
            (s.anim.selection.clone(), (s.layout.inspector.w - 24.0).max(220.0))
        };
        match selection {
            AnimSelection::None => {
                self.body.add_child(Box::new(Label::muted(
                    "Select a state or a transition",
                    theme::FONT_SIZE,
                )));
                self.body.add_child(Box::new(Label::muted(
                    "in the graph to edit it.",
                    theme::FONT_SIZE,
                )));
            }
            AnimSelection::State(name) => self.build_state(&name, width),
            AnimSelection::Transition(index) => self.build_transition(index, width),
        }
        self.body.set_z_index(2.0);
    }

    fn build_state(&mut self, name: &str, width: f32) {
        let (def, is_default, outgoing) = {
            let s = self.state.borrow();
            let Some(def) = s.anim.file.state(name).cloned() else {
                return;
            };
            let outgoing: Vec<(usize, String, usize)> = s
                .anim
                .file
                .transitions
                .iter()
                .enumerate()
                .filter(|(_, t)| t.from == name)
                .map(|(i, t)| (i, t.to.clone(), t.conditions.len()))
                .collect();
            (def, s.anim.file.default_state == name, outgoing)
        };
        let value_w = width - 96.0 - 16.0;

        let mut section = Section::new("State", Some(icons::BOX), width);
        let name_source = DataSource::new(def.name.clone());
        section.add_child(Box::new(FieldRow::new(
            "Name",
            vec![UI::input_sized(name_source.clone(), Size::new(value_w, 24.0))],
        )));
        self.bindings.push(Binding::StateName {
            source: name_source,
            last: def.name.clone(),
        });
        let sync = DataSource::new(def.sync);
        section.add_child(Box::new(FieldRow::new("Sync clips", vec![UI::checkbox(sync.clone())])));
        self.bindings.push(Binding::Sync { source: sync, last: def.sync });
        if is_default {
            section.add_child(Box::new(FieldRow::new(
                "Default",
                vec![Box::new(Label::muted("Entry state of the graph", theme::FONT_SIZE_SMALL + 0.5))],
            )));
        } else {
            let default_state = self.state.clone();
            let default_name = def.name.clone();
            section.add_child(Box::new(FieldRow::new(
                "Default",
                vec![action_button(
                    "Make default",
                    icons::CHECK_MARK,
                    ButtonStyle::Secondary,
                    Box::new(move |_| {
                        let mut s = default_state.borrow_mut();
                        s.anim.file.default_state = default_name.clone();
                        s.anim.touch_structure();
                    }),
                )],
            )));
        }
        self.body.add_child(Box::new(section));

        // Clips.
        let files = animation_files();
        let mut clips = Section::new("Clips", Some(icons::PACKAGE), width);
        if def.clips.is_empty() {
            clips.add_child(Box::new(Label::muted("No clips: the state plays nothing.", theme::FONT_SIZE_SMALL + 0.5)));
        }
        for (i, clip) in def.clips.iter().enumerate() {
            let mut items = files.clone();
            if !items.contains(&clip.file) {
                items.insert(0, clip.file.clone());
            }
            let selected = items.iter().position(|f| *f == clip.file).unwrap_or(0) as u32;
            let file_source = DataSource::new(selected);
            let name_source = DataSource::new(clip.name.clone());
            let remove_state = self.state.clone();
            let state_name = def.name.clone();
            let remove = UI::icon_button(
                icons::TRASH,
                Box::new(move |_| {
                    let mut s = remove_state.borrow_mut();
                    if let Some(st) = s.anim.file.states.iter_mut().find(|st| st.name == state_name) {
                        if i < st.clips.len() {
                            st.clips.remove(i);
                        }
                    }
                    s.anim.touch_structure();
                }),
            );
            let file_w = (width - 16.0 - 90.0 - 26.0 - 8.0).max(80.0);
            clips.add_child(Box::new(FieldRow::with_label_width(
                "",
                0.0,
                vec![
                    UI::input_sized(name_source.clone(), Size::new(90.0, 24.0)),
                    dropdown(items.clone(), &file_source, file_w),
                    remove,
                ],
            )));
            self.bindings.push(Binding::ClipName {
                clip: i,
                source: name_source,
                last: clip.name.clone(),
            });
            self.bindings.push(Binding::ClipFile {
                clip: i,
                source: file_source,
                last: selected,
                files: items,
            });
        }
        let add_state = self.state.clone();
        let state_name = def.name.clone();
        let first_file = files.first().cloned().unwrap_or_else(|| "Idle.fbx".into());
        clips.add_child(action_button(
            "Add clip",
            icons::PLUS,
            ButtonStyle::Ghost,
            Box::new(move |_| {
                let mut s = add_state.borrow_mut();
                if let Some(st) = s.anim.file.states.iter_mut().find(|st| st.name == state_name) {
                    let name = format!("clip{}", st.clips.len() + 1);
                    st.clips.push(ClipDef {
                        name,
                        file: first_file.clone(),
                    });
                }
                s.anim.touch_structure();
            }),
        ));
        self.body.add_child(Box::new(clips));

        // Outgoing transitions.
        let mut transitions = Section::new("Transitions", Some(icons::ACTIVITY), width);
        if outgoing.is_empty() {
            transitions.add_child(Box::new(Label::muted(
                "Drag the node's handle onto another state.",
                theme::FONT_SIZE_SMALL + 0.5,
            )));
        }
        for (index, to, conditions) in outgoing {
            let select_state = self.state.clone();
            let label = format!(
                "to {to}   ({conditions} condition{})",
                if conditions == 1 { "" } else { "s" }
            );
            transitions.add_child(action_button(
                &label,
                icons::CHEVRON_RIGHT,
                ButtonStyle::Ghost,
                Box::new(move |_| {
                    select_state.borrow_mut().anim.selection = AnimSelection::Transition(index);
                }),
            ));
        }
        self.body.add_child(Box::new(transitions));

        let delete_state = self.state.clone();
        self.body.add_child(action_button(
            "Delete state",
            icons::TRASH,
            ButtonStyle::Danger,
            Box::new(move |_| send(&delete_state, Command::DeleteGraphSelection)),
        ));
    }

    fn build_transition(&mut self, index: usize, width: f32) {
        let (t, inputs) = {
            let s = self.state.borrow();
            let Some(t) = s.anim.file.transitions.get(index).cloned() else {
                return;
            };
            let inputs: Vec<String> = s.anim.file.inputs.iter().map(|i| i.name.clone()).collect();
            (t, inputs)
        };

        let mut section = Section::new("Transition", Some(icons::ACTIVITY), width);
        let mut route = Label::new(format!("{}  \u{2192}  {}", t.from, t.to), theme::FONT_SIZE);
        route.set_font(ferrite::core::renderer::text::Fonts::InterSemiBold, theme::FONT_SIZE);
        section.add_child(Box::new(route));
        let blend = DataSource::new((t.blend * 1000.0).round() / 1000.0);
        let value_w = width - 96.0 - 16.0;
        section.add_child(Box::new(FieldRow::new(
            "Blend (s)",
            vec![
                UI::slider_sized(blend.clone(), 0.0, 2.0, Size::new(value_w - 70.0, 20.0)),
                UI::input_sized(blend.clone(), Size::new(64.0, 24.0)),
            ],
        )));
        self.bindings.push(Binding::Blend {
            last: blend.read(),
            source: blend,
        });
        self.body.add_child(Box::new(section));

        // Conditions: input [+|-] input <op> value.
        let mut conditions = Section::new("Conditions (all must hold)", Some(icons::SLIDERS), width);
        if t.conditions.is_empty() {
            conditions.add_child(Box::new(Label::muted(
                "No conditions: taken immediately.",
                theme::FONT_SIZE_SMALL + 0.5,
            )));
        }
        let mut a_items = inputs.clone();
        if a_items.is_empty() {
            a_items.push(NONE_LABEL.into());
        }
        let mut b_items = vec![NONE_LABEL.to_string()];
        b_items.extend(inputs.iter().cloned());
        let op_items: Vec<String> = CompareOp::ALL.iter().map(|o| o.symbol().to_string()).collect();
        let row_w = width - 16.0;
        for (ci, c) in t.conditions.iter().enumerate() {
            let mut header = Label::new(format!("{}.  {}", ci + 1, c.describe()), theme::FONT_SIZE_SMALL + 0.5);
            header.set_color(theme::TEXT_MUTED);
            conditions.add_child(Box::new(header));
            if c.terms.len() > 2 {
                conditions.add_child(Box::new(Label::muted(
                    "More than two inputs: edit in the .ron file.",
                    theme::FONT_SIZE_SMALL,
                )));
                continue;
            }
            let a_index = c
                .terms
                .first()
                .and_then(|term| inputs.iter().position(|i| *i == term.input))
                .unwrap_or(0) as u32;
            let b_index = c
                .terms
                .get(1)
                .and_then(|term| inputs.iter().position(|i| *i == term.input))
                .map(|i| i as u32 + 1)
                .unwrap_or(0);
            let sign_index = match c.terms.get(1) {
                Some(term) if term.sign < 0.0 => 1,
                _ => 0,
            };
            let op_index = CompareOp::ALL.iter().position(|o| *o == c.op).unwrap_or(0) as u32;
            let (a, sign, b, op, value) = (
                DataSource::new(a_index),
                DataSource::new(sign_index),
                DataSource::new(b_index),
                DataSource::new(op_index),
                DataSource::new(c.value),
            );
            let term_w = ((row_w - 52.0 - 8.0) / 2.0).floor();
            conditions.add_child(Box::new(FieldRow::with_label_width(
                "",
                0.0,
                vec![
                    dropdown(a_items.clone(), &a, term_w),
                    dropdown(vec!["+".into(), "-".into()], &sign, 52.0),
                    dropdown(b_items.clone(), &b, term_w),
                ],
            )));
            let remove_state = self.state.clone();
            let remove = UI::icon_button(
                icons::TRASH,
                Box::new(move |_| {
                    let mut s = remove_state.borrow_mut();
                    if let Some(t) = s.anim.file.transitions.get_mut(index) {
                        if ci < t.conditions.len() {
                            t.conditions.remove(ci);
                        }
                    }
                    s.anim.touch_structure();
                }),
            );
            conditions.add_child(Box::new(FieldRow::with_label_width(
                "",
                0.0,
                vec![
                    dropdown(op_items.clone(), &op, 70.0),
                    UI::input_sized(value.clone(), Size::new(80.0, 24.0)),
                    remove,
                ],
            )));
            self.bindings.push(Binding::TermA { cond: ci, last: a_index, source: a });
            self.bindings.push(Binding::Sign { cond: ci, last: sign_index, source: sign });
            self.bindings.push(Binding::TermB { cond: ci, last: b_index, source: b });
            self.bindings.push(Binding::Op { cond: ci, last: op_index, source: op });
            self.bindings.push(Binding::Value { cond: ci, last: c.value, source: value });
        }
        let add_state = self.state.clone();
        let first_input = inputs.first().cloned();
        conditions.add_child(action_button(
            "Add condition",
            icons::PLUS,
            ButtonStyle::Ghost,
            Box::new(move |_| {
                let mut s = add_state.borrow_mut();
                if let Some(t) = s.anim.file.transitions.get_mut(index) {
                    t.conditions.push(Comparison {
                        terms: first_input
                            .iter()
                            .map(|input| Term {
                                input: input.clone(),
                                sign: 1.0,
                            })
                            .collect(),
                        op: CompareOp::Greater,
                        value: 0.0,
                    });
                }
                s.anim.touch_structure();
            }),
        ));
        self.body.add_child(Box::new(conditions));

        let delete_state = self.state.clone();
        self.body.add_child(action_button(
            "Delete transition",
            icons::TRASH,
            ButtonStyle::Danger,
            Box::new(move |_| send(&delete_state, Command::DeleteGraphSelection)),
        ));
    }

    /// Push edited widget values into the graph file.
    fn sync(&mut self) {
        let mut s = self.state.borrow_mut();
        let selection = s.anim.selection.clone();
        let inputs: Vec<String> = s.anim.file.inputs.iter().map(|i| i.name.clone()).collect();
        let mut changed = false;
        let mut renamed_to: Option<String> = None;
        for binding in self.bindings.iter_mut() {
            match (binding, &selection) {
                (Binding::StateName { source, last }, AnimSelection::State(name)) => {
                    let value = source.read();
                    let trimmed = value.trim().to_owned();
                    if value != *last {
                        *last = value.clone();
                        let taken = s.anim.file.state(&trimmed).is_some();
                        if !trimmed.is_empty() && !taken {
                            s.anim.file.rename_state(name, &trimmed);
                            renamed_to = Some(trimmed);
                            changed = true;
                        }
                    }
                }
                (Binding::Sync { source, last }, AnimSelection::State(name)) => {
                    let value = source.read();
                    if value != *last {
                        *last = value;
                        if let Some(st) = s.anim.file.states.iter_mut().find(|st| st.name == *name) {
                            st.sync = value;
                        }
                        changed = true;
                    }
                }
                (Binding::ClipName { clip, source, last }, AnimSelection::State(name)) => {
                    let value = source.read();
                    if value != *last && !value.trim().is_empty() {
                        *last = value.clone();
                        if let Some(c) = s
                            .anim
                            .file
                            .states
                            .iter_mut()
                            .find(|st| st.name == *name)
                            .and_then(|st| st.clips.get_mut(*clip))
                        {
                            c.name = value.trim().to_owned();
                        }
                        changed = true;
                    }
                }
                (Binding::ClipFile { clip, source, last, files }, AnimSelection::State(name)) => {
                    let value = source.read();
                    if value != *last {
                        *last = value;
                        if let (Some(c), Some(file)) = (
                            s.anim
                                .file
                                .states
                                .iter_mut()
                                .find(|st| st.name == *name)
                                .and_then(|st| st.clips.get_mut(*clip)),
                            files.get(value as usize),
                        ) {
                            c.file = file.clone();
                        }
                        changed = true;
                    }
                }
                (Binding::Blend { source, last }, AnimSelection::Transition(index)) => {
                    let value = source.read();
                    if (value - *last).abs() > 1e-6 {
                        *last = value;
                        if let Some(t) = s.anim.file.transitions.get_mut(*index) {
                            t.blend = value.max(0.0);
                        }
                        changed = true;
                    }
                }
                (binding, AnimSelection::Transition(index)) => {
                    let Some(t) = s.anim.file.transitions.get_mut(*index) else {
                        continue;
                    };
                    changed |= sync_condition(binding, &mut t.conditions, &inputs);
                }
                _ => {}
            }
        }
        if let Some(new_name) = renamed_to {
            // Keep the panel (and the focused name field) instead of
            // rebuilding it for the "new" selection.
            s.anim.selection = AnimSelection::State(new_name.clone());
            if let Some(built) = &mut self.built_for {
                built.selection = AnimSelection::State(new_name);
            }
        }
        if changed {
            s.anim.touch();
        }
    }
}

fn sync_condition(binding: &mut Binding, conditions: &mut [Comparison], inputs: &[String]) -> bool {
    let input_name = |i: u32| inputs.get(i as usize).cloned();
    match binding {
        Binding::TermA { cond, source, last } => {
            let value = source.read();
            if value == *last {
                return false;
            }
            *last = value;
            let (Some(c), Some(name)) = (conditions.get_mut(*cond), input_name(value)) else {
                return false;
            };
            match c.terms.first_mut() {
                Some(term) => term.input = name,
                None => c.terms.push(Term { input: name, sign: 1.0 }),
            }
            true
        }
        Binding::Sign { cond, source, last } => {
            let value = source.read();
            if value == *last {
                return false;
            }
            *last = value;
            if let Some(term) = conditions.get_mut(*cond).and_then(|c| c.terms.get_mut(1)) {
                term.sign = if value == 1 { -1.0 } else { 1.0 };
            }
            true
        }
        Binding::TermB { cond, source, last } => {
            let value = source.read();
            if value == *last {
                return false;
            }
            *last = value;
            let Some(c) = conditions.get_mut(*cond) else {
                return false;
            };
            if value == 0 {
                c.terms.truncate(1);
            } else if let Some(name) = input_name(value - 1) {
                let sign = c.terms.get(1).map_or(-1.0, |t| t.sign);
                c.terms.truncate(1);
                c.terms.push(Term { input: name, sign });
            }
            true
        }
        Binding::Op { cond, source, last } => {
            let value = source.read();
            if value == *last {
                return false;
            }
            *last = value;
            if let (Some(c), Some(op)) = (conditions.get_mut(*cond), CompareOp::ALL.get(value as usize)) {
                c.op = *op;
            }
            true
        }
        Binding::Value { cond, source, last } => {
            let value = source.read();
            if (value - *last).abs() <= 1e-6 {
                return false;
            }
            *last = value;
            if let Some(c) = conditions.get_mut(*cond) {
                c.value = value;
            }
            true
        }
        _ => false,
    }
}

impl UIElement for GraphInspector {
    fn update(&mut self, scene: &mut Scene) {
        let (rect, wanted) = {
            let s = self.state.borrow();
            (
                s.layout.inspector,
                BuiltFor {
                    selection: s.anim.selection.clone(),
                    structure: s.anim.structure,
                    labels: s.anim.labels,
                },
            )
        };
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));
        let header = self.panel.header_height();
        self.body
            .set_position(Position::new(rect.x + 4.0, rect.y + header + 4.0, 0.0));
        self.body.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.h - header - 8.0).max(10.0),
        ));
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

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.state.borrow().layout.inspector.contains(x as f32, y as f32);
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
