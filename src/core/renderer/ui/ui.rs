use crate::core::scene::Scene;

use super::{
    button::{Button, ButtonBuilder},
    input::{Input, InputBuilder},
    panel::{Panel, PanelBuilder},
    text::Text,
    UIElement, UIRenderer, UI,
};

impl UIRenderer {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add(&mut self, element: Box<dyn UIElement>) {
        self.children.push(element);
    }

    pub fn render(&mut self, scene: &mut Scene) {
        for child in &mut self.children {
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
        for child in &mut self.children {
            if child.handle_events(scene, window, glfw, event) {
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
        let mut text = Text::new(text, size);
        text = init_fn(text);
        Box::new(text)
    }

    pub fn input<InitFn>(init_fn: InitFn) -> Box<Input>
    where
        InitFn: FnOnce(InputBuilder) -> InputBuilder + 'static,
    {
        let mut builder = InputBuilder::new("");
        builder = init_fn(builder);
        Box::new(builder.build())
    }

    pub fn button<InitFn>(text: &str, on_click: Box<dyn Fn(&mut Scene)>, init_fn: InitFn) -> Box<Button>
    where
        InitFn: FnOnce(ButtonBuilder) -> ButtonBuilder + 'static,
    {
        let mut builder = ButtonBuilder::new();
        builder = builder
            .on_click(on_click)
            .size(100.0, 20.0)
            .add_child(Box::new(Text::new(text, 16.0)));
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
}
