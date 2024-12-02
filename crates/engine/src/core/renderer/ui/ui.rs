use std::{collections::BTreeMap, str::FromStr};

use crate::core::{scene::Scene, utils::DataSource};

use super::{
    button::{Button, ButtonBuilder},
    container::{Container, ContainerBuilder},
    input::{Input, InputBuilder},
    panel::{Panel, PanelBuilder},
    text::Text,
    UIElement, UIElementHandle, UIRenderer, UI,
};

impl UIRenderer {
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

    pub fn insert_to(
        &mut self,
        parent: UIElementHandle,
        id: Option<UIElementHandle>,
        element: Box<dyn UIElement>,
    ) {
        if let Some(parent) = self.children.get_mut(&parent) {
            parent.add_children(vec![(id, element)]);
        } else {
            for (_, child) in &mut self.children {
                if child.contains_child(&parent) {
                    child.add_child_to(parent, id, element);
                    return;
                }
            }
        }
    }

    pub fn render(&mut self, scene: &mut Scene) {
        for (_, child) in &mut self.children {
            child.render(scene);
        }
    }

    pub fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
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
}

impl UI {
    pub fn text<InitFn>(text: &str, size: f32, init_fn: InitFn) -> Box<Text>
    where
        InitFn: FnOnce(Text) -> Text + 'static,
    {
        let mut text = Text::new(text.to_string(), size);
        text = init_fn(text);
        Box::new(text)
    }

    pub fn collapsible<InitFn>(title: &str, init_fn: InitFn) -> Box<Panel>
    where
        InitFn: FnOnce(PanelBuilder) -> PanelBuilder + 'static,
    {
        let mut builder = PanelBuilder::new(title);
        builder = builder.size(200.0, 200.0).collapsible();
        builder = init_fn(builder);
        Box::new(builder.build())
    }

    pub fn collapsible_dyn<InitFn>(title: DataSource<String>, init_fn: InitFn) -> Box<Panel>
    where
        InitFn: FnOnce(PanelBuilder) -> PanelBuilder + 'static,
    {
        let mut builder = PanelBuilder::new("").title_source(title);
        builder = builder.size(200.0, 200.0).collapsible();
        builder = init_fn(builder);
        Box::new(builder.build())
    }

    pub fn input<T: Clone + ToString + FromStr, InitFn>(
        data_source: DataSource<T>,
        init_fn: InitFn,
    ) -> Box<Input<T>>
    where
        InitFn: FnOnce(InputBuilder<T>) -> InputBuilder<T> + 'static,
    {
        let mut builder = InputBuilder::new(data_source.read()).data_source(Some(data_source));
        builder = init_fn(builder);
        Box::new(builder.build())
    }

    pub fn button<InitFn>(
        text: &str,
        on_click: Box<dyn Fn(&mut Scene)>,
        init_fn: InitFn,
    ) -> Box<Button>
    where
        InitFn: FnOnce(ButtonBuilder) -> ButtonBuilder + 'static,
    {
        let mut builder = ButtonBuilder::new();
        builder = builder
            .on_click(on_click)
            .size(100.0, 20.0)
            .add_child(Box::new(Text::new(text.to_string(), 16.0)));
        builder = init_fn(builder);
        Box::new(builder.build())
    }

    pub fn panel<InitFn>(title: &str, init_fn: InitFn) -> Box<Panel>
    where
        InitFn: FnOnce(PanelBuilder) -> PanelBuilder + 'static,
    {
        let mut builder = PanelBuilder::new(title);
        builder = builder.size(200.0, 200.0);
        builder = init_fn(builder);
        Box::new(builder.build())
    }

    pub fn container<InitFn>(init_fn: InitFn) -> Box<Container>
    where
        InitFn: FnOnce(ContainerBuilder) -> ContainerBuilder + 'static,
    {
        let mut builder = ContainerBuilder::new();
        builder = init_fn(builder);
        Box::new(builder.build())
    }
}
