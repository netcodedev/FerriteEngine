use std::borrow::Cow;

use cgmath::{Deg, Euler, Quaternion};
use ferrite::{
    core::{
        entity::{
            component::{Property, PropertyValue},
            EntityHandle,
        },
        primitives::{Offset, Position, Size},
        renderer::text::icons,
        scene::{scene_file::ComponentData, Scene},
        utils::DataSource,
    },
    terrain::scatter::Scatter,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        dropdown::Dropdown,
        label::Label,
        menu_button::{MenuButton, MenuItem},
        panel::Panel,
        scroll::ScrollArea,
        section::Section,
    },
    theme, UIElement, UI,
};
use glfw::{Glfw, WindowEvent};

use super::{
    drag::{DropAction, DropTarget},
    field_row::{FieldRow, LABEL_COLUMN},
    hierarchy::entity_icon,
    layout::Rect,
    send, Command, Shared,
};

/// Shape of what the inspector currently shows. When the selected entity's
/// shape changes (another entity, component added/removed) the widgets are
/// rebuilt; otherwise only values are synchronised.
#[derive(Clone, PartialEq)]
struct Signature {
    entity: Option<EntityHandle>,
    /// Per component its name and its properties (name, read-only).
    components: Vec<(String, Vec<(Cow<'static, str>, bool)>)>,
    width: i32,
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
    Choice {
        source: DataSource<u32>,
        last: u32,
        options: Vec<String>,
    },
}

struct PropertyBinding {
    component: usize,
    name: Cow<'static, str>,
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
    /// Rows a model asset can be dropped on, with the scatter layer they
    /// stand for (`None`: the "Add model" row).
    scatter_rows: Vec<(UIElementHandle, EntityHandle, Option<usize>)>,
    /// The selected entity, if it has a terrain: an asset dropped anywhere
    /// on the inspector becomes a new scatter layer of it.
    terrain_entity: Option<EntityHandle>,
    /// Open/closed state per section title, kept across rebuilds.
    section_open: std::collections::HashMap<String, DataSource<bool>>,
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn changed(a: f32, b: f32) -> bool {
    (a - b).abs() > 0.0005
}

fn number_input(source: &DataSource<f32>, width: f32) -> Box<dyn UIElement> {
    UI::input_sized(source.clone(), Size::new(width, 24.0))
}

/// Icon for a component section, by display name.
fn component_icon(name: &str) -> &'static str {
    match name {
        "Camera" => icons::CAMERA,
        "SkyLight" | "Sky light" => icons::SUN,
        "Terrain" => icons::MOUNTAIN,
        "Debug controller" => icons::BUG,
        "Player controller" => icons::USER,
        "Collider" | "Bone colliders" => icons::SCAN,
        "Animation" | "AnimationComponent" => icons::ACTIVITY,
        "ModelComponent" => icons::PACKAGE,
        _ => icons::COMPONENT,
    }
}

impl InspectorPanel {
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
            signature: None,
            transform: None,
            properties: Vec::new(),
            scatter_rows: Vec::new(),
            terrain_entity: None,
            section_open: std::collections::HashMap::new(),
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
                            c.properties()
                                .into_iter()
                                .map(|p| (p.name, p.read_only))
                                .collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Signature {
            entity: id.filter(|id| scene.get_entity(id).is_some()),
            components,
            width: 0,
        }
    }

    fn section(&mut self, title: &str, icon: &str, width: f32) -> Section {
        let mut section = Section::new(title, Some(icon), width);
        let flag = self
            .section_open
            .entry(title.to_owned())
            .or_insert_with(|| DataSource::new(true))
            .clone();
        section.bind_open(flag);
        section
    }

    fn rebuild(&mut self, scene: &Scene, signature: &Signature) {
        self.body.clear();
        self.transform = None;
        self.properties.clear();
        self.scatter_rows.clear();
        self.terrain_entity = None;
        let width = (self.state.borrow().layout.inspector.w - 24.0).max(200.0);

        let Some(id) = signature.entity else {
            let _ = width;
            self.body.add_child(Box::new(Label::muted(
                "Select an entity in the hierarchy",
                theme::FONT_SIZE,
            )));
            self.body.add_child(Box::new(Label::muted(
                "or click one in the viewport.",
                theme::FONT_SIZE,
            )));
            return;
        };
        let Some(entity) = scene.get_entity(&id) else {
            return;
        };

        // Entity name.
        let name = entity.get_name_ref();
        let mut header = self.section("Entity", entity_icon(entity), width);
        header.add_child(Box::new(FieldRow::new(
            "Name",
            vec![UI::input_sized(name.clone(), Size::new(width - LABEL_COLUMN - 16.0, 24.0))],
        )));
        self.body.add_child(Box::new(header));

        // Transform.
        let position: [DataSource<f32>; 3] = std::array::from_fn(|_| DataSource::new(0.0));
        let rotation: [DataSource<f32>; 3] = std::array::from_fn(|_| DataSource::new(0.0));
        let field_w = ((width - LABEL_COLUMN - 16.0 - 8.0) / 3.0).floor();
        let mut transform = self.section("Transform", icons::AXIS_3D, width);
        transform.add_child(Box::new(FieldRow::new(
            "Position",
            position.iter().map(|s| number_input(s, field_w)).collect(),
        )));
        transform.add_child(Box::new(FieldRow::new(
            "Rotation",
            rotation.iter().map(|s| number_input(s, field_w)).collect(),
        )));
        self.body.add_child(Box::new(transform));
        self.transform = Some(TransformBinding {
            position,
            position_last: [f32::NAN; 3],
            rotation,
            rotation_last: [f32::NAN; 3],
            name_last: name.read(),
            name,
        });

        // One section per component, with a remove action in its header.
        for (index, component) in entity.components().iter().enumerate() {
            let display = component.display_name();
            let mut section = self.section(&display, component_icon(&display), width);
            let remove_state = self.state.clone();
            let mut remove = Button::new(
                Position::default(),
                Size::new(24.0, 24.0),
                Box::new(move |_| send(&remove_state, Command::RemoveComponent(id, index))),
            );
            remove.set_style(ButtonStyle::Danger);
            remove.set_icon(icons::TRASH);
            remove.set_tooltip("Remove component");
            section.set_accessory(Box::new(remove));
            let properties = component.properties();
            if properties.is_empty() {
                section.add_child(Box::new(Label::muted("No editable properties", theme::FONT_SIZE_SMALL)));
            }
            for property in properties {
                // A terrain's scatter models get an asset chooser and a
                // remove button.
                let layer = (display == "Terrain")
                    .then(|| Scatter::model_property_layer(&property.name))
                    .flatten();
                let extra = match layer {
                    Some(layer) => self.scatter_model_buttons(id, layer),
                    None => Vec::new(),
                };
                let row = self.add_property(index, property, width, extra);
                if let Some(layer) = layer {
                    self.scatter_rows.push((*row.get_handle(), id, Some(layer)));
                }
                section.add_child(row);
            }
            if display == "Terrain" {
                self.terrain_entity = Some(id);
                let add_state = self.state.clone();
                let mut add = Button::new(
                    Position::default(),
                    Size::new(10.0, 26.0),
                    Box::new(move |_| {
                        send(
                            &add_state,
                            Command::PickScatterAsset {
                                entity: id,
                                layer: None,
                            },
                        )
                    }),
                );
                add.set_style(ButtonStyle::Secondary);
                add.set_icon(icons::PLUS);
                add.set_label("Add model");
                add.set_tooltip("Scatter another model over the terrain");
                add.fit(26.0);
                let row = Box::new(FieldRow::new("", vec![Box::new(add)]));
                self.scatter_rows.push((*row.get_handle(), id, None));
                section.add_child(row);
            }
            if display == "Animation" {
                let edit_state = self.state.clone();
                let mut edit = Button::new(
                    Position::default(),
                    Size::new(10.0, 26.0),
                    Box::new(move |_| send(&edit_state, Command::EditAnimationGraph(id))),
                );
                edit.set_style(ButtonStyle::Primary);
                edit.set_icon(icons::PENCIL);
                edit.set_label("Edit graph");
                edit.fit(26.0);
                section.add_child(Box::new(FieldRow::new("", vec![Box::new(edit)])));
            }
            self.body.add_child(Box::new(section));
        }

        // Add component.
        let add_state = self.state.clone();
        let items = ComponentData::catalog()
            .iter()
            .map(|(name, _)| MenuItem::new(*name).icon(component_icon(name)))
            .collect();
        let mut add = MenuButton::with_items(
            "Add component",
            Some(icons::PLUS),
            items,
            Box::new(move |index, _scene| {
                if let Some((_, data)) = ComponentData::catalog().into_iter().nth(index) {
                    send(&add_state, Command::AddComponent(id, data));
                }
            }),
        );
        add.set_button_style(ButtonStyle::Secondary);
        self.body.add_child(Box::new(add));
    }

    /// Tell the drag layer where a model asset can be dropped: on a scatter
    /// layer's row (replaces its model), on the "Add model" row or anywhere
    /// else on the inspector of a terrain (adds a layer).
    fn register_drop_targets(&self) {
        let Some(entity) = self.terrain_entity else {
            return;
        };
        let panel = self.state.borrow().layout.inspector;
        let mut targets = vec![DropTarget {
            rect: panel,
            action: DropAction::ScatterLayer {
                entity,
                layer: None,
            },
        }];
        for (handle, entity, layer) in &self.scatter_rows {
            let Some(row) = self.body.get_child(handle) else {
                continue;
            };
            // The full width of the panel, not just as far as the row's
            // widgets reach.
            let rect = Rect {
                x: panel.x + 4.0,
                y: row.get_offset().y - 1.0,
                w: (panel.w - 8.0).max(1.0),
                h: row.get_size().height + 2.0,
            };
            // Rows scrolled out of the panel cannot be hit.
            if rect.y < panel.y || rect.y + rect.h > panel.y + panel.h {
                continue;
            }
            targets.push(DropTarget {
                rect,
                action: DropAction::ScatterLayer {
                    entity: *entity,
                    layer: *layer,
                },
            });
        }
        self.state.borrow_mut().drop_targets.extend(targets);
    }

    /// "Choose asset" and "remove" buttons for one scatter layer of a terrain.
    fn scatter_model_buttons(&self, id: EntityHandle, layer: usize) -> Vec<Box<dyn UIElement>> {
        let browse_state = self.state.clone();
        let mut browse = Button::new(
            Position::default(),
            Size::new(24.0, 24.0),
            Box::new(move |_| {
                send(
                    &browse_state,
                    Command::PickScatterAsset {
                        entity: id,
                        layer: Some(layer),
                    },
                )
            }),
        );
        browse.set_style(ButtonStyle::Secondary);
        browse.set_icon(icons::PACKAGE);
        browse.set_tooltip("Choose a model asset");

        let remove_state = self.state.clone();
        let mut remove = Button::new(
            Position::default(),
            Size::new(24.0, 24.0),
            Box::new(move |_| send(&remove_state, Command::RemoveScatterModel(id, layer))),
        );
        remove.set_style(ButtonStyle::Danger);
        remove.set_icon(icons::X);
        remove.set_tooltip("Remove this model");
        vec![Box::new(browse), Box::new(remove)]
    }

    /// `extra` widgets are placed before the value widget, which is narrowed
    /// to leave room for them.
    fn add_property(
        &mut self,
        component: usize,
        property: Property,
        width: f32,
        extra: Vec<Box<dyn UIElement>>,
    ) -> Box<dyn UIElement> {
        let extra_w: f32 = extra.iter().map(|w| w.get_size().width + 4.0).sum();
        let value_w = width - LABEL_COLUMN - 16.0 - extra_w;
        let mut widgets: Vec<Box<dyn UIElement>>;
        let field = if property.read_only {
            let source = DataSource::new(display_value(&property.value));
            let mut label = Label::bound(source.clone(), theme::FONT_SIZE_SMALL + 0.5);
            label.set_color(theme::TEXT_MUTED);
            widgets = vec![Box::new(label)];
            Field::ReadOnly { source }
        } else {
            match &property.value {
                PropertyValue::Float(v) => {
                    let source = DataSource::new(round3(*v));
                    widgets = match property.range {
                        Some((min, max)) => vec![
                            UI::slider_sized(source.clone(), min, max, Size::new(value_w - 74.0, 20.0)),
                            number_input(&source, 66.0),
                        ],
                        None => vec![number_input(&source, value_w.min(140.0))],
                    };
                    Field::Float {
                        source,
                        last: round3(*v),
                    }
                }
                PropertyValue::Int(v) => {
                    let source = DataSource::new(*v);
                    widgets = vec![UI::input_sized(source.clone(), Size::new(value_w.min(140.0), 24.0))];
                    Field::Int { source, last: *v }
                }
                PropertyValue::Bool(v) => {
                    let source = DataSource::new(*v);
                    widgets = vec![UI::checkbox(source.clone())];
                    Field::Bool { source, last: *v }
                }
                PropertyValue::Text(v) if property.options.is_some() => {
                    let options = property.options.clone().unwrap_or_default();
                    let index = options.iter().position(|o| o == v).unwrap_or(0) as u32;
                    let source = DataSource::new(index);
                    widgets = vec![Box::new(Dropdown::new(
                        Position::default(),
                        Size::new(value_w, 24.0),
                        options.clone(),
                        source.clone(),
                    ))];
                    Field::Choice {
                        source,
                        last: index,
                        options,
                    }
                }
                PropertyValue::Text(v) => {
                    let source = DataSource::new(v.clone());
                    widgets = vec![UI::input_sized(source.clone(), Size::new(value_w, 24.0))];
                    Field::Text {
                        source,
                        last: v.clone(),
                    }
                }
            }
        };
        // In front, so they stay put whatever the length of a read-only
        // value.
        let mut widgets_in_order = extra;
        widgets_in_order.append(&mut widgets);
        let row = Box::new(FieldRow::new(&property.name, widgets_in_order));
        self.properties.push(PropertyBinding {
            component,
            name: property.name,
            field,
        });
        row
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
                        component.set_property(&binding.name, PropertyValue::Float(typed));
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
                        component.set_property(&binding.name, PropertyValue::Int(typed));
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
                        component.set_property(&binding.name, PropertyValue::Bool(typed));
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
                        component.set_property(&binding.name, PropertyValue::Text(typed.clone()));
                        *last = typed;
                        edited = true;
                    } else if actual != *last {
                        source.write(actual.clone());
                        *last = actual;
                    }
                }
                (Field::Choice { source, last, options }, PropertyValue::Text(actual)) => {
                    let typed = source.read();
                    if typed != *last {
                        *last = typed;
                        if let Some(option) = options.get(typed as usize) {
                            component.set_property(&binding.name, PropertyValue::Text(option.clone()));
                            edited = true;
                        }
                    } else if let Some(index) = options.iter().position(|o| *o == actual) {
                        if index as u32 != *last {
                            source.write(index as u32);
                            *last = index as u32;
                        }
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

        let mut signature = Self::signature(scene, selected);
        signature.width = rect.w as i32;
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
        self.register_drop_targets();
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
