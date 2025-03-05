use ferrite::core::{entity::EntityHandle, utils::DataSource};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{button::Button, panel::Panel, popup::Popup},
};

mod component;
pub mod ecs;
mod entity;

pub struct EntityComponentsPanel {
    handle: UIElementHandle,
    panel: Box<Panel>,
    entity_panel_handle: UIElementHandle,
    components_panel_handle: UIElementHandle,
}

pub struct EntityUI {
    handle: UIElementHandle,
    entity_handle: EntityHandle,
    panel: Panel,
}
pub struct AddEntityButton {
    handle: UIElementHandle,
    button: Box<Button>,
}

pub struct EditEntityButton {
    handle: UIElementHandle,
    button: Box<Button>,
    show_popup: DataSource<bool>,
    popup: Popup,
}

pub struct AddComponentButton {
    handle: UIElementHandle,
    button: Box<Button>,
}
