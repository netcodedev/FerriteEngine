use ferrite::core::{
    entity::EntityHandle,
    renderer::ui::{button::Button, panel::Panel, popup::Popup, primitives::UIElementHandle},
    utils::DataSource,
};

mod component;
pub mod ecs;
mod entity;

pub struct EntityComponentsPanel {
    panel: Box<Panel>,
    entity_panel_handle: UIElementHandle,
    components_panel_handle: UIElementHandle,
}

pub struct EntityUI {
    entity_handle: EntityHandle,
    panel: Panel,
}
pub struct AddEntityButton {
    button: Box<Button>,
}

pub struct EditEntityButton {
    button: Box<Button>,
    show_popup: DataSource<bool>,
    popup: Popup,
}

pub struct AddComponentButton {
    button: Box<Button>,
}
