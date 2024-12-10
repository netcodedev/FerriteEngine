use crate::core::{
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        ui::{panel::PanelBuilder, Offset, Size, UIElement, UIElementHandle, UI},
    },
    scene::Scene,
    utils::DataSource,
};

use super::Popup;

impl Popup {
    pub fn new(
        title: &str,
        close_ref: DataSource<bool>,
        children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>,
    ) -> Self {
        let mut panel = PanelBuilder::new(title)
            .position(400.0, 300.0, 51.0)
            .size(200.0, 150.0)
            .add_control(
                None,
                UI::button(
                    "x",
                    Box::new(move |_| {
                        close_ref.write(false);
                    }),
                    |builder| builder.size(18.0, 18.0),
                ),
            )
            .build();
        panel.add_children(children);
        let background = PlaneBuilder::new()
            .position((0.0, 0.0, 50.0).into())
            .size((5000.0, 5000.0).into())
            .color((0.0, 0.0, 0.0, 0.6))
            .build();
        Self { background, panel }
    }
}

impl UIElement for Popup {
    fn render(&mut self, scene: &mut Scene) {
        PlaneRenderer::render(&self.background);
        self.panel.render(scene);
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut glfw::Glfw,
        event: &glfw::WindowEvent,
    ) -> bool {
        self.panel.handle_events(scene, window, glfw, event);
        true
    }

    fn add_children(&mut self, children: Vec<(Option<UIElementHandle>, Box<dyn UIElement>)>) {
        self.panel.add_children(children);
    }

    fn add_child_to(
        &mut self,
        parent: UIElementHandle,
        id: Option<UIElementHandle>,
        element: Box<dyn UIElement>,
    ) {
        self.panel.add_child_to(parent, id, element);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.panel.contains_child(handle)
    }

    fn get_offset(&self) -> &Offset {
        self.panel.get_offset()
    }

    fn set_offset(&mut self, offset: Offset) {
        self.panel.set_offset(offset);
    }

    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }

    fn set_z_index(&mut self, z_index: f32) {
        self.panel.set_z_index(z_index);
    }
}
