use std::{collections::BTreeMap, str::FromStr};

use ferrite::core::{
    primitives::{Position, Size},
    scene::Scene,
    utils::DataSource,
};
use glfw::{Glfw, Window, WindowEvent};

use std::path::PathBuf;

use super::{
    element_handle::UIElementHandle,
    elements::{
        button::Button,
        checkbox::Checkbox,
        container::{Container, Direction},
        color_picker::ColorPicker,
        divider::{Divider, Orientation},
        dropdown::Dropdown,
        image::Image,
        input::Input,
        label::Label,
        list::List,
        menu_button::MenuButton,
        panel::Panel,
        popup::Popup,
        progress::ProgressBar,
        radio::Radio,
        scroll::ScrollArea,
        slider::Slider,
        tabs::Tabs,
        text::Text,
        toggle::Toggle,
        tooltip::Tooltip,
        tree::TreeView,
    },
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
        super::clip::reset();
        for (_, child) in &mut self.children {
            child.render();
        }
        // Overlay pass: popups, modal dim layers, etc. — drawn after every
        // normal panel so they always sit on top.
        for (_, child) in &mut self.children {
            child.render_overlay();
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
            if child.is_modal() {
                return child.handle_events(scene, window, glfw, event);
            }
        }
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
        let mut button = Button::new(Position::default(), Size::new(10.0, 26.0), on_click);
        button.set_label(text);
        button.fit(26.0);
        Box::new(button)
    }

    pub fn input<T: Clone + ToString + FromStr>(data_source: DataSource<T>) -> Box<Input<T>> {
        Self::input_sized(data_source, Size::new(140.0, 22.0))
    }

    /// A button showing only a Lucide icon, in the ghost style.
    pub fn icon_button(icon: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Box<Button> {
        let mut button = Button::new(Position::default(), Size::new(26.0, 26.0), on_click);
        button.set_style(super::elements::button::ButtonStyle::Ghost);
        button.set_icon(icon);
        Box::new(button)
    }

    pub fn input_sized<T: Clone + ToString + FromStr>(
        data_source: DataSource<T>,
        size: Size,
    ) -> Box<Input<T>> {
        let mut input = Input::new(
            Position::default(),
            size,
            data_source.read(),
            Some(data_source),
        );
        input.set_size(size);
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

    pub fn container_vertical() -> Box<Container> {
        let mut c = Container::new(Position::default(), Size::default());
        c.set_direction(Direction::Vertical);
        Box::new(c)
    }

    pub fn container_horizontal() -> Box<Container> {
        let mut c = Container::new(Position::default(), Size::default());
        c.set_direction(Direction::Horizontal);
        Box::new(c)
    }

    pub fn text(text: &str, font_size: f32) -> Box<Text> {
        Box::new(Text::new(text.to_owned(), font_size))
    }

    pub fn label(text: &str, font_size: f32) -> Box<Label> {
        Box::new(Label::new(text, font_size))
    }

    pub fn checkbox(value: DataSource<bool>) -> Box<Checkbox> {
        Box::new(Checkbox::new(Position::default(), value))
    }

    pub fn toggle(value: DataSource<bool>) -> Box<Toggle> {
        Box::new(Toggle::new(Position::default(), value))
    }

    pub fn radio(value: DataSource<u32>, index: u32) -> Box<Radio> {
        Box::new(Radio::new(Position::default(), value, index))
    }

    pub fn slider(value: DataSource<f32>, min: f32, max: f32) -> Box<Slider> {
        Box::new(Slider::new(
            Position::default(),
            Size::new(140.0, 18.0),
            value,
            min,
            max,
        ))
    }

    pub fn slider_sized(value: DataSource<f32>, min: f32, max: f32, size: Size) -> Box<Slider> {
        Box::new(Slider::new(Position::default(), size, value, min, max))
    }

    pub fn progress(value: DataSource<f32>) -> Box<ProgressBar> {
        Box::new(ProgressBar::new(
            Position::default(),
            Size::new(140.0, 10.0),
            value,
        ))
    }

    pub fn divider_h(length: f32) -> Box<Divider> {
        Box::new(Divider::new(Position::default(), length, Orientation::Horizontal))
    }

    pub fn divider_v(length: f32) -> Box<Divider> {
        Box::new(Divider::new(Position::default(), length, Orientation::Vertical))
    }

    pub fn image(size: Size, path: PathBuf) -> Box<Image> {
        Box::new(Image::new(Position::default(), size, path))
    }

    pub fn tooltip(text: &str, trigger: ferrite::core::primitives::Region) -> Box<Tooltip> {
        Box::new(Tooltip::new(text, trigger))
    }

    pub fn dropdown(items: Vec<String>, selected: DataSource<u32>) -> Box<Dropdown> {
        Box::new(Dropdown::new(
            Position::default(),
            Size::new(140.0, 20.0),
            items,
            selected,
        ))
    }

    pub fn list(items: Vec<String>, selected: DataSource<u32>, size: Size) -> Box<List> {
        Box::new(List::new(Position::default(), size, items, selected))
    }

    pub fn tabs(
        labels: Vec<String>,
        pages: Vec<Box<dyn UIElement>>,
        selected: DataSource<u32>,
        size: Size,
    ) -> Box<Tabs> {
        Box::new(Tabs::new(Position::default(), size, labels, pages, selected))
    }

    pub fn scroll_area(size: Size) -> Box<ScrollArea> {
        Box::new(ScrollArea::new(Position::default(), size))
    }

    pub fn color_picker(
        width: f32,
        r: DataSource<f32>,
        g: DataSource<f32>,
        b: DataSource<f32>,
        a: DataSource<f32>,
    ) -> Box<ColorPicker> {
        Box::new(ColorPicker::new(Position::default(), width, r, g, b, a))
    }

    pub fn tree(selected: DataSource<u64>, size: Size) -> Box<TreeView> {
        Box::new(TreeView::new(Position::default(), size, selected))
    }

    pub fn menu_button(
        label: &str,
        items: Vec<String>,
        on_select: Box<dyn Fn(usize, &mut Scene)>,
    ) -> Box<MenuButton> {
        Box::new(MenuButton::new(label, items, on_select))
    }
}
