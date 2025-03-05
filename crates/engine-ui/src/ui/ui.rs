use std::{collections::BTreeMap, str::FromStr};

use ferrite::core::{
    primitives::{Position, Size},
    scene::Scene,
    utils::DataSource,
};
use glfw::{Glfw, Window, WindowEvent};

use super::{
    element_handle::UIElementHandle,
    elements::{button::Button, input::Input, panel::Panel, popup::Popup, text::Text},
    UIElement, UI,
};

impl UI {
    pub fn new() -> Self {
        Self {
            children: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, element: Box<dyn UIElement>) -> UIElementHandle {
        let handle = UIElementHandle::new();
        self.children.insert(handle, element);
        handle
    }

    pub fn insert(&mut self, key: UIElementHandle, element: Box<dyn UIElement>) {
        self.children.insert(key, element);
    }

    pub fn insert_to(&mut self, parent: UIElementHandle, element: Box<dyn UIElement>) {
        if let Some(parent) = self.children.get_mut(&parent) {
            parent.add_child(element);
        } else {
            for (_, child) in &mut self.children {
                if child.contains_child(&parent) {
                    child.add_child_to(parent, element);
                    return;
                }
            }
        }
    }

    pub fn update(&mut self, scene: &mut Scene) {
        for (_, child) in &mut self.children {
            child.update(scene);
        }
    }

    pub fn render(&mut self) {
        for (_, child) in &mut self.children {
            child.render();
        }
    }

    pub fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        for (_, child) in &mut self.children {
            if child.handle_events(scene, window, glfw, event) {
                return true;
            }
        }
        false
    }

    pub fn contains_key(&self, key: &UIElementHandle) -> bool {
        if self.children.contains_key(key) {
            return true;
        }
        for (_, child) in &self.children {
            if child.contains_child(key) {
                return true;
            }
        }
        false
    }

    pub fn button(text: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Box<Button> {
        let mut button = Button::new(
            Position::default(),
            Size {
                width: 100.0,
                height: 20.0,
            },
            on_click,
        );
        button.add_child(Box::new(Text::new(text.to_owned(), 16.0)));
        Box::new(button)
    }

    pub fn input<T: Clone + ToString + FromStr>(data_source: DataSource<T>) -> Box<Input<T>> {
        let input = Input::new(
            Position::default(),
            Size::default(),
            data_source.read(),
            Some(data_source),
        );
        Box::new(input)
    }

    pub fn panel(title: &str, position: Position, size: Size) -> Box<Panel> {
        let panel = Panel::new(title.to_owned(), position, size);
        Box::new(panel)
    }

    pub fn popup(
        title: &str,
        close_ref: DataSource<bool>,
        children: Vec<Box<dyn UIElement>>,
    ) -> Box<Popup> {
        Box::new(Popup::new(title.to_owned(), close_ref, children))
    }

    pub fn text(text: &str, font_size: f32) -> Box<Text> {
        Box::new(Text::new(text.to_owned(), font_size))
    }
}
