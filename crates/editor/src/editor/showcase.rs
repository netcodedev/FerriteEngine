use ferrite::core::{
    primitives::{Offset, Position, Size},
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{panel::Panel, popup::Popup, scroll::ScrollArea},
    UIElement, UI,
};

use super::{layout::Rect, Shared};
use glfw::{Glfw, WindowEvent};

/// Wrapper around a `Popup` that only renders/forwards events while
/// `show` is `true`. The popup's close button flips `show` to `false`.
pub struct PopupGate {
    handle: UIElementHandle,
    show: DataSource<bool>,
    popup: Popup,
}

impl PopupGate {
    pub fn new(show: DataSource<bool>, popup: Popup) -> Self {
        Self {
            handle: UIElementHandle::new(),
            show,
            popup,
        }
    }
}

impl UIElement for PopupGate {
    fn update(&mut self, scene: &mut Scene) {
        if self.show.read() {
            self.popup.update(scene);
        }
    }
    fn render(&self) {
        if self.show.read() {
            self.popup.render();
        }
    }
    fn render_overlay(&self) {
        if self.show.read() {
            self.popup.render_overlay();
        }
    }
    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if self.show.read() {
            self.popup.handle_events(scene, window, glfw, event)
        } else {
            false
        }
    }
    fn add_child(&mut self, _c: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _p: UIElementHandle, _c: Box<dyn UIElement>) {}
    fn contains_child(&self, _h: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _h: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _h: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        self.popup.get_offset()
    }
    fn set_offset(&mut self, offset: Offset) {
        self.popup.set_offset(offset);
    }
    fn get_size(&self) -> &Size {
        self.popup.get_size()
    }
    fn set_z_index(&mut self, z: f32) {
        self.popup.set_z_index(z);
    }
}

/// Floating panel that exercises every widget in `ferrite_ui`; opened from
/// the editor's Window menu.
fn build_widget_showcase(show_popup: DataSource<bool>, state: Shared) -> Box<dyn UIElement> {
    let mut panel = ScrollArea::new(Position::default(), Size::new(290.0, 400.0));
    panel.set_gap(6.0);

    // Text (static).
    panel.add_child(UI::label("Text (static)", 14.0));
    panel.add_child(UI::text("The quick brown fox", 14.0));

    // Button.
    panel.add_child(UI::label("Button", 14.0));
    let mut btn_row = UI::container_horizontal();
    btn_row.add_child(UI::button(
        "Click me",
        Box::new(|_scene| {
            println!("button clicked");
        }),
    ));
    btn_row.add_child(UI::button(
        "Open popup",
        Box::new(move |_scene| {
            show_popup.write(true);
        }),
    ));
    panel.add_child(btn_row);

    // Input bound to a DataSource<String>.
    panel.add_child(UI::label("Input", 14.0));
    panel.add_child(UI::input_sized(
        DataSource::new(String::from("edit me")),
        Size::new(290.0, 22.0),
    ));

    // Numeric input bound to a DataSource<f32> (same widget, different type).
    panel.add_child(UI::label("Numeric input", 14.0));
    panel.add_child(UI::input_sized(
        DataSource::<f32>::new(1.5),
        Size::new(290.0, 22.0),
    ));

    // Checkbox + toggle row.
    panel.add_child(UI::label("Checkbox / Toggle", 14.0));
    let mut row = UI::container_horizontal();
    row.add_child(UI::checkbox(DataSource::new(true)));
    row.add_child(UI::toggle(DataSource::new(false)));
    panel.add_child(row);

    // Radios.
    panel.add_child(UI::label("Radio group", 14.0));
    let radio_value = DataSource::new(0u32);
    let mut radios = UI::container_horizontal();
    radios.add_child(UI::radio(radio_value.clone(), 0));
    radios.add_child(UI::radio(radio_value.clone(), 1));
    radios.add_child(UI::radio(radio_value.clone(), 2));
    panel.add_child(radios);

    // Divider.
    panel.add_child(UI::divider_h(290.0));

    // Slider + progress.
    panel.add_child(UI::label("Slider", 14.0));
    panel.add_child(UI::slider(DataSource::new(0.5), 0.0, 1.0));
    panel.add_child(UI::label("Progress", 14.0));
    panel.add_child(UI::progress(DataSource::new(0.35)));

    // Dropdown.
    panel.add_child(UI::label("Dropdown", 14.0));
    panel.add_child(UI::dropdown(
        vec!["One".into(), "Two".into(), "Three".into()],
        DataSource::new(0u32),
    ));

    // List.
    panel.add_child(UI::label("List", 14.0));
    panel.add_child(UI::list(
        vec!["Alpha".into(), "Beta".into(), "Gamma".into(), "Delta".into()],
        DataSource::new(1u32),
        Size::new(290.0, 80.0),
    ));

    // Scroll area with a stack of labels.
    panel.add_child(UI::label("Scroll area", 14.0));
    let mut scroll = UI::scroll_area(Size::new(290.0, 90.0));
    for i in 0..20 {
        scroll.add_child(UI::label(&format!("scroll row {i}"), 13.0));
    }
    panel.add_child(scroll);

    // Tabs with simple label pages.
    panel.add_child(UI::label("Tabs", 14.0));
    let pages: Vec<Box<dyn UIElement>> = vec![
        UI::label("Page A content", 13.0),
        UI::label("Page B content", 13.0),
        UI::label("Page C content", 13.0),
    ];
    panel.add_child(UI::tabs(
        vec!["A".into(), "B".into(), "C".into()],
        pages,
        DataSource::new(0u32),
        Size::new(290.0, 90.0),
    ));

    // Color picker.
    panel.add_child(UI::label("Color picker", 14.0));
    panel.add_child(UI::color_picker(
        290.0,
        DataSource::new(0.8),
        DataSource::new(0.4),
        DataSource::new(0.2),
        DataSource::new(1.0),
    ));

    Box::new(DockedShowcase::new(state, panel))
}

/// The showcase docked in the right column (below the statistics when both
/// are shown).
struct DockedShowcase {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    body: ScrollArea,
}

impl DockedShowcase {
    fn new(state: Shared, mut body: ScrollArea) -> Self {
        let mut panel = Panel::new("Widget showcase".into(), Position::default(), Size::new(300.0, 400.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);
        body.set_z_index(2.0);
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            body,
        }
    }

    fn rect(&self) -> Rect {
        self.state.borrow().layout.widgets
    }
}

impl UIElement for DockedShowcase {
    fn update(&mut self, scene: &mut Scene) {
        let rect = self.rect();
        if rect.w <= 0.0 {
            return;
        }
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));
        let header = self.panel.header_height();
        self.body
            .set_position(Position::new(rect.x + 4.0, rect.y + header + 4.0, 0.0));
        self.body.set_size(Size::new(
            (rect.w - 8.0).max(10.0),
            (rect.h - header - 8.0).max(10.0),
        ));
        self.panel.update(scene);
        self.body.update(scene);
    }

    fn render(&self) {
        if self.rect().w <= 0.0 {
            return;
        }
        self.panel.render();
        self.body.render();
    }

    fn render_overlay(&self) {
        if self.rect().w > 0.0 {
            self.body.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        let rect = self.rect();
        if rect.w <= 0.0 {
            return false;
        }
        if self.body.handle_events(scene, window, glfw, event) {
            return true;
        }
        if let WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) = event {
            let (x, y) = window.get_cursor_pos();
            return rect.contains(x as f32, y as f32);
        }
        false
    }

    fn add_child(&mut self, child: Box<dyn UIElement>) {
        self.body.add_child(child);
    }
    fn add_child_to(&mut self, parent: UIElementHandle, child: Box<dyn UIElement>) {
        self.body.add_child_to(parent, child);
    }
    fn contains_child(&self, handle: &UIElementHandle) -> bool {
        self.body.contains_child(handle)
    }
    fn get_child(&self, handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        self.body.get_child(handle)
    }
    fn get_child_mut(&mut self, handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        self.body.get_child_mut(handle)
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
    fn set_z_index(&mut self, z: f32) {
        self.panel.set_z_index(z);
        self.body.set_z_index(z + 2.0);
    }
}

/// The widget showcase panel plus the modal popup its "Open popup" button
/// shows. Both are added to the UI separately so the popup can sit above
/// every other element.
pub fn widget_showcase(state: Shared) -> (Box<dyn UIElement>, Box<dyn UIElement>) {
    let show_popup = DataSource::new(false);
    let panel = build_widget_showcase(show_popup.clone(), state);
    let popup = Popup::new(
        String::from("Popup demo"),
        show_popup.clone(),
        vec![
            UI::label("A Popup is a modal panel.", 13.0),
            UI::label("Click the x to close.", 13.0),
        ],
    );
    (panel, Box::new(PopupGate::new(show_popup, popup)))
}
