use cgmath::{Deg, Euler, Quaternion};
use ferrite::core::{
    entity::{
        component::{Property, PropertyValue},
        EntityHandle,
    },
    primitives::{Offset, Position, Size},
    scene::{scene_file::ComponentData, Scene},
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::Button, label::Label, menu_button::MenuButton, panel::Panel, scroll::ScrollArea,
    },
    UIElement, UI,
};
use glfw::{Glfw, WindowEvent};

use super::{field_row::FieldRow, send, Command, Shared};

/// Shape of what the inspector currently shows. When the selected entity's
/// shape changes (another entity, component added/removed) the widgets are
/// rebuilt; otherwise only values are synchronised.
#[derive(Clone, PartialEq)]
struct Signature {
    entity: Option<EntityHandle>,
    components: Vec<(String, Vec<&'static str>)>,
}

enum Field {
    Float {
        source: DataSource<f32>,
        last: f32,
    },
    Int {
        source: DataSource<i64>,
        last: i64,
    },
    Bool {
        source: DataSource<bool>,
        last: bool,
    },
    Text {
        source: DataSource<String>,
        last: String,
    },
    ReadOnly {
        source: DataSource<String>,
    },
}

struct PropertyBinding {
    component: usize,
    name: &'static str,
    field: Field,
}

struct TransformBinding {
    position: [DataSource<f32>; 3],
    position_last: [f32; 3],
    rotation: [DataSource<f32>; 3],
    rotation_last: [f32; 3],
    name: DataSource<String>,
    name_last: String,
}

pub struct InspectorPanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
    signature: Option<Signature>,
    transform: Option<TransformBinding>,
    properties: Vec<PropertyBinding>,
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn changed(a: f32, b: f32) -> bool {
    (a - b).abs() > 0.0005
}

fn number_input(source: &DataSource<f32>, width: f32) -> Box<dyn UIElement> {
    UI::input_sized(source.clone(), Size::new(width, 20.0))
}

fn section_title(text: &str) -> Box<dyn UIElement> {
    UI::label(text, 15.0)
}

impl InspectorPanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Inspector".into(), Position::default(), Size::new(330.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        let mut body = ScrollArea::new(Position::default(), Size::new(320.0, 300.0));
        body.set_gap(6.0);
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
            signature: None,
            transform: None,
            properties: Vec::new(),
        }
    }

    fn signature(scene: &Scene, id: Option<EntityHandle>) -> Signature {
        let components = id
            .and_then(|id| scene.get_entity(&id))
            .map(|entity| {
                entity
                    .components()
                    .iter()
                    .map(|c| {
                        (
                            c.display_name(),
                            c.properties().iter().map(|p| p.name).collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Signature {
            entity: id.filter(|id| scene.get_entity(id).is_some()),
            components,
        }
    }

    fn rebuild(&mut self, scene: &Scene, signature: &Signature) {
        self.body.clear();
        self.transform = None;
        self.properties.clear();

        let Some(id) = signature.entity else {
            self.body
                .add_child(UI::label("Select an entity in the hierarchy", 14.0));
            self.body
                .add_child(UI::label("or click one in the viewport.", 14.0));
            return;
        };
        let Some(entity) = scene.get_entity(&id) else {
            return;
        };

        // Name and transform.
        let name = entity.get_name_ref();
        self.body.add_child(Box::new(FieldRow::new(
            "Name",
            vec![UI::input_sized(name.clone(), Size::new(190.0, 20.0))],
        )));
        self.body.add_child(section_title("Transform"));
        let position: [DataSource<f32>; 3] = std::array::from_fn(|_| DataSource::new(0.0));
        let rotation: [DataSource<f32>; 3] = std::array::from_fn(|_| DataSource::new(0.0));
        self.body.add_child(Box::new(FieldRow::new(
            "Position",
            position.iter().map(|s| number_input(s, 60.0)).collect(),
        )));
        self.body.add_child(Box::new(FieldRow::new(
            "Rotation",
            rotation.iter().map(|s| number_input(s, 60.0)).collect(),
        )));
        self.transform = Some(TransformBinding {
            position,
            position_last: [f32::NAN; 3],
            rotation,
            rotation_last: [f32::NAN; 3],
            name_last: name.read(),
            name,
        });

        // Components.
        for (index, component) in entity.components().iter().enumerate() {
            self.body.add_child(UI::divider_h(300.0));
            let remove_state = self.state.clone();
            let mut remove = Button::new(
                Position::default(),
                Size::new(70.0, 20.0),
                Box::new(move |_| send(&remove_state, Command::RemoveComponent(id, index))),
            );
            remove.set_label("Remove");
            self.body.add_child(Box::new(FieldRow::with_label_width(
                &component.display_name(),
                220.0,
                vec![Box::new(remove)],
            )));
            for property in component.properties() {
                self.add_property(index, property);
            }
        }

        // Add component.
        self.body.add_child(UI::divider_h(300.0));
        let catalog = ComponentData::catalog();
        let names = catalog.iter().map(|(name, _)| name.to_string()).collect();
        let add_state = self.state.clone();
        let add = MenuButton::new(
            "+ Add component",
            names,
            Box::new(move |index, _scene| {
                if let Some((_, data)) = ComponentData::catalog().into_iter().nth(index) {
                    send(&add_state, Command::AddComponent(id, data));
                }
            }),
        );
        self.body.add_child(Box::new(add));
    }

    fn add_property(&mut self, component: usize, property: Property) {
        let widgets: Vec<Box<dyn UIElement>>;
        let field = if property.read_only {
            let source = DataSource::new(display_value(&property.value));
            widgets = vec![Box::new(Label::bound(source.clone(), 14.0))];
            Field::ReadOnly { source }
        } else {
            match &property.value {
                PropertyValue::Float(v) => {
                    let source = DataSource::new(round3(*v));
                    widgets = match property.range {
                        Some((min, max)) => vec![
                            UI::slider_sized(source.clone(), min, max, Size::new(110.0, 18.0)),
                            number_input(&source, 66.0),
                        ],
                        None => vec![number_input(&source, 120.0)],
                    };
                    Field::Float {
                        source,
                        last: round3(*v),
                    }
                }
                PropertyValue::Int(v) => {
                    let source = DataSource::new(*v);
                    widgets = vec![UI::input_sized(source.clone(), Size::new(120.0, 20.0))];
                    Field::Int { source, last: *v }
                }
                PropertyValue::Bool(v) => {
                    let source = DataSource::new(*v);
                    widgets = vec![UI::checkbox(source.clone())];
                    Field::Bool { source, last: *v }
                }
                PropertyValue::Text(v) => {
                    let source = DataSource::new(v.clone());
                    widgets = vec![UI::input_sized(source.clone(), Size::new(180.0, 20.0))];
                    Field::Text {
                        source,
                        last: v.clone(),
                    }
                }
            }
        };
        self.body
            .add_child(Box::new(FieldRow::new(property.name, widgets)));
        self.properties.push(PropertyBinding {
            component,
            name: property.name,
            field,
        });
    }

    /// Two-way sync: a source whose value differs from what we last wrote was
    /// edited by the user and is applied to the scene; otherwise the scene's
    /// current value is written into the source (so live values update).
    fn sync(&mut self, scene: &mut Scene, id: EntityHandle) -> bool {
        let mut edited = false;

        if let Some(t) = &mut self.transform {
            let Some(entity) = scene.get_entity(&id) else {
                return false;
            };
            let name = t.name.read();
            if name != t.name_last {
                t.name_last = name;
                edited = true;
            }

            let p = entity.get_position();
            let actual = [round3(p.x), round3(p.y), round3(p.z)];
            let typed: [f32; 3] = std::array::from_fn(|i| t.position[i].read());
            if (0..3).any(|i| changed(typed[i], t.position_last[i])) && !t.position_last[0].is_nan() {
                scene.set_entity_position(&id, typed.into());
                t.position_last = typed;
                edited = true;
            } else {
                for i in 0..3 {
                    if changed(actual[i], t.position_last[i]) || t.position_last[i].is_nan() {
                        t.position[i].write(actual[i]);
                        t.position_last[i] = actual[i];
                    }
                }
            }

            let Some(entity) = scene.get_entity(&id) else {
                return edited;
            };
            let euler = Euler::from(entity.get_rotation());
            let actual = [
                round3(Deg::from(euler.x).0),
                round3(Deg::from(euler.y).0),
                round3(Deg::from(euler.z).0),
            ];
            let typed: [f32; 3] = std::array::from_fn(|i| t.rotation[i].read());
            if (0..3).any(|i| changed(typed[i], t.rotation_last[i])) && !t.rotation_last[0].is_nan() {
                let q = Quaternion::from(Euler::new(Deg(typed[0]), Deg(typed[1]), Deg(typed[2])));
                scene.set_entity_rotation(&id, q);
                t.rotation_last = typed;
                edited = true;
            } else {
                for i in 0..3 {
                    if changed(actual[i], t.rotation_last[i]) || t.rotation_last[i].is_nan() {
                        t.rotation[i].write(actual[i]);
                        t.rotation_last[i] = actual[i];
                    }
                }
            }
        }

        let Some(entity) = scene.get_entity_mut(&id) else {
            return edited;
        };
        for binding in self.properties.iter_mut() {
            let Some(component) = entity.components_mut().get_mut(binding.component) else {
                continue;
            };
            let current = component
                .properties()
                .into_iter()
                .find(|p| p.name == binding.name)
                .map(|p| p.value);
            let Some(current) = current else {
                continue;
            };
            match (&mut binding.field, current) {
                (Field::Float { source, last }, PropertyValue::Float(actual)) => {
                    let typed = source.read();
                    if changed(typed, *last) {
                        component.set_property(binding.name, PropertyValue::Float(typed));
                        *last = typed;
                        edited = true;
                    } else if changed(round3(actual), *last) {
                        source.write(round3(actual));
                        *last = round3(actual);
                    }
                }
                (Field::Int { source, last }, PropertyValue::Int(actual)) => {
                    let typed = source.read();
                    if typed != *last {
                        component.set_property(binding.name, PropertyValue::Int(typed));
                        *last = typed;
                        edited = true;
                    } else if actual != *last {
                        source.write(actual);
                        *last = actual;
                    }
                }
                (Field::Bool { source, last }, PropertyValue::Bool(actual)) => {
                    let typed = source.read();
                    if typed != *last {
                        component.set_property(binding.name, PropertyValue::Bool(typed));
                        *last = typed;
                        edited = true;
                    } else if actual != *last {
                        source.write(actual);
                        *last = actual;
                    }
                }
                (Field::Text { source, last }, PropertyValue::Text(actual)) => {
                    let typed = source.read();
                    if typed != *last {
                        component.set_property(binding.name, PropertyValue::Text(typed.clone()));
                        *last = typed;
                        edited = true;
                    } else if actual != *last {
                        source.write(actual.clone());
                        *last = actual;
                    }
                }
                (Field::ReadOnly { source }, value) => {
                    let text = display_value(&value);
                    if source.read() != text {
                        source.write(text);
                    }
                }
                _ => {}
            }
        }
        edited
    }
}

fn display_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Float(v) => format!("{v:.3}"),
        PropertyValue::Int(v) => v.to_string(),
        PropertyValue::Bool(v) => if *v { "yes" } else { "no" }.into(),
        PropertyValue::Text(v) => v.clone(),
    }
}

impl UIElement for InspectorPanel {
    fn update(&mut self, scene: &mut Scene) {
        let (rect, selected) = {
            let state = self.state.borrow();
            (state.layout.inspector, state.selected())
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

        let signature = Self::signature(scene, selected);
        if self.signature.as_ref() != Some(&signature) {
            self.rebuild(scene, &signature);
            self.body.set_z_index(2.0);
            self.signature = Some(signature.clone());
        }
        if let Some(id) = signature.entity {
            if self.sync(scene, id) {
                self.state.borrow_mut().dirty = true;
            }
        }

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
            return self
                .state
                .borrow()
                .layout
                .inspector
                .contains(x as f32, y as f32);
        }
        false
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
        self.panel.get_offset()
    }
    fn set_offset(&mut self, _offset: Offset) {}
    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.panel.set_z_index(z_index);
        self.body.set_z_index(z_index + 2.0);
    }
}
