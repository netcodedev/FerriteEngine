use crate::ui::theme;
use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::{
        plane::{PlaneBuilder, PlaneRenderer},
        text::icons,
    },
    scene::Scene,
    utils::DataSource,
};

use crate::ui::{element_handle::UIElementHandle, elements::panel::Panel, UIElement, UI};

use super::Popup;

impl Popup {
    pub fn new(
        title: String,
        close_ref: DataSource<bool>,
        children: Vec<Box<dyn UIElement>>,
    ) -> Self {
        // Panel starts off-screen; we recenter it in `update` once children are
        // laid out and the real screen size is known.
        let mut panel = Panel::new(
            title,
            Position { x: 0.0, y: 0.0, z: 51.0 },
            Size { width: 200.0, height: 150.0 },
        );
        panel.set_movable(false);
        for child in children {
            panel.add_child(child);
        }
        // Add the close control last so it anchors against the final width.
        let mut button = UI::icon_button(
            icons::X,
            Box::new(move |_| {
                close_ref.write(false);
            }),
        );
        button.set_size(Size { width: 22.0, height: 22.0 });
        panel.add_control(button);

        let background = PlaneBuilder::new()
            .position((0.0, 0.0, 51.0).into())
            .size((5000.0, 5000.0).into())
            .color(theme::OVERLAY_DIM)
            .build();

        Self {
            handle: UIElementHandle::new(),
            background,
            panel,
        }
    }
}

impl UIElement for Popup {
    fn update(&mut self, scene: &mut Scene) {
        let (sw, sh) = PlaneRenderer::get_size();
        let psize = *self.panel.get_size();
        let target = Offset {
            x: (sw - psize.width) * 0.5,
            y: (sh - psize.height) * 0.5,
        };
        if *self.panel.get_offset() != target {
            self.panel.set_offset(target);
        }
        self.panel.update(scene);
    }

    fn render(&self) {
        // Nothing in the regular pass — popups live in the overlay pass so
        // they end up on top of every other UI element.
    }

    fn render_overlay(&self) {
        // Depth writes disabled so later overlays stay unaffected; depth test
        // disabled too so the dim is not clipped by the depth buffer we
        // inherited from the normal pass.
        unsafe {
            gl::Disable(gl::DEPTH_TEST);
            gl::DepthMask(gl::FALSE);
        }
        PlaneRenderer::render(&self.background);
        self.panel.render();
        unsafe {
            gl::DepthMask(gl::TRUE);
            gl::Enable(gl::DEPTH_TEST);
        }
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

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.panel.add_child(child);
    }

    // `render_overlay` is implemented above; Popup owns the overlay pass.

    fn add_child_to(&mut self, parent: UIElementHandle, element: Box<dyn UIElement>) {
        self.panel.add_child_to(parent, element);
    }

    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.panel.contains_child(handle)
    }

    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        self.panel.get_child(handle)
    }

    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        self.panel.get_child_mut(handle)
    }

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
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
