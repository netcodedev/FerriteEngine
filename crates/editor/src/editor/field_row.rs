use ferrite::core::{
    primitives::{Offset, Position, Size},
    renderer::text::{Fonts, Text},
    scene::Scene,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, UIElement};
use glfw::{Glfw, WindowEvent};

/// Inspector row: a label in a fixed-width column, then widgets laid out
/// left to right. Keeps the value widgets of all rows aligned.
pub struct FieldRow {
    handle: UIElementHandle,
    offset: Offset,
    z: f32,
    size: Size,
    label: Text,
    label_width: f32,
    gap: f32,
    widgets: Vec<Box<dyn UIElement>>,
}

pub const LABEL_COLUMN: f32 = 110.0;

impl FieldRow {
    pub fn new(label: &str, widgets: Vec<Box<dyn UIElement>>) -> Self {
        Self::with_label_width(label, LABEL_COLUMN, widgets)
    }

    pub fn with_label_width(
        label: &str,
        label_width: f32,
        widgets: Vec<Box<dyn UIElement>>,
    ) -> Self {
        let mut row = Self {
            handle: UIElementHandle::new(),
            offset: Offset::default(),
            z: 0.0,
            size: Size::new(label_width, 22.0),
            label: Text::new(Fonts::RobotoMono, 0, 0, 0, 14.0, label.to_owned()),
            label_width,
            gap: 4.0,
            widgets,
        };
        row.layout();
        row
    }

    fn layout(&mut self) {
        let height = self
            .widgets
            .iter()
            .map(|w| w.get_size().height)
            .fold(20.0f32, f32::max);
        let mut x = self.offset.x + self.label_width;
        for widget in self.widgets.iter_mut() {
            let h = widget.get_size().height;
            let target = Offset {
                x,
                y: self.offset.y + (height - h) * 0.5,
            };
            if *widget.get_offset() != target {
                widget.set_offset(target);
            }
            x += widget.get_size().width + self.gap;
        }
        self.size = Size::new(x - self.offset.x, height);
        self.label.prepare_render_at(Position::new(
            self.offset.x,
            self.offset.y + (height - 16.0).max(0.0) * 0.5,
            self.z + 1.0,
        ));
    }
}

impl UIElement for FieldRow {
    fn update(&mut self, scene: &mut Scene) {
        self.layout();
        for widget in self.widgets.iter_mut() {
            widget.update(scene);
        }
    }

    fn render(&self) {
        self.label.render();
        for widget in &self.widgets {
            widget.render();
        }
    }

    fn render_overlay(&self) {
        for widget in &self.widgets {
            widget.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        // Every widget sees cursor moves (hover state), clicks stop at the
        // first widget that takes them.
        let mut handled = false;
        for widget in self.widgets.iter_mut() {
            if widget.handle_events(scene, window, glfw, event) {
                handled = true;
                if !matches!(event, WindowEvent::CursorPos(..)) {
                    break;
                }
            }
        }
        handled
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.widgets.push(child);
    }
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        if let Some(w) = self.widgets.iter_mut().find(|w| w.contains_child(&parent)) {
            w.add_child_to(parent, child);
        }
    }
    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.widgets
            .iter()
            .any(|w| w.get_handle() == handle || w.contains_child(handle))
    }
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        for w in &self.widgets {
            if w.get_handle() == handle {
                return Some(w);
            }
            if let Some(c) = w.get_child(handle) {
                return Some(c);
            }
        }
        None
    }
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        for w in self.widgets.iter_mut() {
            if w.get_handle() == handle {
                return Some(w);
            }
            if let Some(c) = w.get_child_mut(handle) {
                return Some(c);
            }
        }
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
        self.layout();
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.z = z_index;
        self.label.set_z_index(z_index + 1.0);
        for widget in self.widgets.iter_mut() {
            widget.set_z_index(z_index + 1.0);
        }
    }
}
