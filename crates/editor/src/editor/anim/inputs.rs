use ferrite::core::{
    model::animation_graph::graph_file::InputDef,
    primitives::{Offset, Position, Size},
    renderer::text::icons,
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        label::Label,
        panel::Panel,
        scroll::ScrollArea,
    },
    theme, UIElement, UI,
};
use glfw::{Glfw, WindowEvent};

use super::super::{field_row::FieldRow, Shared};

struct InputBinding {
    index: usize,
    name: DataSource<String>,
    name_last: String,
    default: DataSource<f32>,
    default_last: f32,
}

/// Left-top panel of the animation workspace: the graph's inputs.
pub struct InputsPanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    add: Button,
    bindings: Vec<InputBinding>,
    built_for: Option<u64>,
}

impl InputsPanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Inputs".into(), Position::default(), Size::new(330.0, 300.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 200.0));
        body.set_gap(4.0);
        let add_state = state.clone();
        let mut add = Button::new(
            Position::default(),
            Size::new(10.0, 24.0),
            Box::new(move |_| {
                let mut s = add_state.borrow_mut();
                let name = s.anim.unique_input_name("input");
                s.anim.file.inputs.push(InputDef { name, default: 0.0 });
                s.anim.touch_structure();
            }),
        );
        add.set_style(ButtonStyle::Ghost);
        add.set_icon(icons::PLUS);
        add.set_label("Add");
        add.fit(24.0);
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            add,
            bindings: Vec::new(),
            built_for: None,
        }
    }

    fn rebuild(&mut self) {
        self.body.clear();
        self.bindings.clear();
        let (inputs, width) = {
            let s = self.state.borrow();
            (s.anim.file.inputs.clone(), s.layout.hierarchy.w)
        };
        if inputs.is_empty() {
            self.body
                .add_child(Box::new(Label::muted("No inputs yet.", theme::FONT_SIZE)));
        }
        let name_w = (width - 24.0 - 70.0 - 26.0 - 16.0).max(80.0);
        for (index, input) in inputs.iter().enumerate() {
            let name = DataSource::new(input.name.clone());
            let default = DataSource::new(input.default);
            let remove_state = self.state.clone();
            let remove = UI::icon_button(
                icons::TRASH,
                Box::new(move |_| {
                    let mut s = remove_state.borrow_mut();
                    if index < s.anim.file.inputs.len() {
                        s.anim.file.inputs.remove(index);
                        s.anim.touch_structure();
                    }
                }),
            );
            self.body.add_child(Box::new(FieldRow::with_label_width(
                "",
                0.0,
                vec![
                    UI::input_sized(name.clone(), Size::new(name_w, 24.0)),
                    UI::input_sized(default.clone(), Size::new(64.0, 24.0)),
                    remove,
                ],
            )));
            self.bindings.push(InputBinding {
                index,
                name_last: input.name.clone(),
                name,
                default_last: input.default,
                default,
            });
        }
        self.body.set_z_index(2.0);
    }

    /// Apply edits from the fields to the graph file.
    fn sync(&mut self) {
        let mut s = self.state.borrow_mut();
        for b in self.bindings.iter_mut() {
            let name = b.name.read();
            let trimmed = name.trim().to_owned();
            if name != b.name_last && !trimmed.is_empty() {
                let taken = s
                    .anim
                    .file
                    .inputs
                    .iter()
                    .enumerate()
                    .any(|(i, input)| i != b.index && input.name == trimmed);
                if !taken {
                    let old = s.anim.file.inputs[b.index].name.clone();
                    s.anim.file.rename_input(&old, &trimmed);
                    s.anim.labels += 1;
                    s.anim.touch();
                }
                b.name_last = name;
            }
            let default = b.default.read();
            if (default - b.default_last).abs() > 1e-6 {
                if let Some(input) = s.anim.file.inputs.get_mut(b.index) {
                    input.default = default;
                }
                s.anim.touch();
                b.default_last = default;
            }
        }
    }
}

impl UIElement for InputsPanel {
    fn update(&mut self, scene: &mut Scene) {
        let (rect, structure) = {
            let s = self.state.borrow();
            (s.layout.hierarchy, s.anim.structure)
        };
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));
        let header = self.panel.header_height();
        self.add.set_offset(Offset {
            x: rect.x + rect.w - self.add.get_size().width - 6.0,
            y: rect.y + (header - 24.0) * 0.5,
        });
        self.body
            .set_position(Position::new(rect.x + 4.0, rect.y + header + 4.0, 0.0));
        self.body.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.h - header - 8.0).max(10.0),
        ));
        if self.built_for != Some(structure) {
            self.rebuild();
            self.built_for = Some(structure);
        }
        self.sync();
        self.panel.update(scene);
        self.add.update(scene);
        self.body.update(scene);
    }

    fn render(&self) {
        self.panel.render();
        self.add.render();
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
        if self.add.handle_events(scene, window, glfw, event) && !matches!(event, WindowEvent::CursorPos(..)) {
            return true;
        }
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return self.state.borrow().layout.hierarchy.contains(x as f32, y as f32);
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
        self.add.set_z_index(z + 3.0);
        self.body.set_z_index(z + 2.0);
    }
}
