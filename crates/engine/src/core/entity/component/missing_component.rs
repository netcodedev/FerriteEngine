use glfw::{Glfw, WindowEvent};

use crate::core::{
    entity::Entity,
    scene::{scene_file::ComponentData, Scene},
};

use super::{Component, Property};

/// Stands in for a component that could not be built when the scene was
/// loaded, e.g. because its asset is missing. It does nothing, but keeps the
/// component's data, so saving the scene does not lose it and the component
/// comes back once the cause is fixed.
pub struct MissingComponent {
    data: ComponentData,
    reason: String,
}

impl MissingComponent {
    pub fn new(data: ComponentData, reason: impl Into<String>) -> Self {
        Self {
            data,
            reason: reason.into(),
        }
    }

    pub fn data(&self) -> &ComponentData {
        &self.data
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl Component for MissingComponent {
    fn update(&mut self, _: &mut Scene, _: &mut Entity, _: f64) {}

    fn handle_event(&mut self, _: &mut Glfw, _: &mut glfw::Window, _: &WindowEvent) {}

    fn display_name(&self) -> String {
        format!("Missing: {}", self.data.label())
    }

    fn to_data(&self) -> Option<ComponentData> {
        Some(self.data.clone())
    }

    fn properties(&self) -> Vec<Property> {
        vec![Property::text("problem", self.reason.clone()).read_only()]
    }
}
