use ferrite::core::{
    primitives::{Offset, Position, Region, Size},
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::{Fonts, Text as FerriteText},
    },
    scene::Scene,
    utils::DataSource,
};
use glfw::{
    Action::Press, Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

struct Option_ {
    label: String,
    text: FerriteText,
    bg: Plane,
    region: Region,
}

/// Collapsed selector that expands into a vertical list of options. Selection
/// is written back to `DataSource<u32>`.
pub struct Dropdown {
    handle: UIElementHandle,
    region: Region,
    selected: DataSource<u32>,
    options: Vec<Option_>,
    is_open: bool,
    is_hovering: bool,
    hover_index: Option<usize>,
    row_height: f32,

    button: Plane,
    button_text: FerriteText,
    menu: Plane,
}

impl Dropdown {
    pub fn new(position: Position, size: Size, items: Vec<String>, selected: DataSource<u32>) -> Self {
        let row_height = size.height;
        let button = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.2, 0.2, 0.2, 1.0))
            .border_color((0.6, 0.6, 0.6, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(3.0)
            .build();
        let button_text = FerriteText::new(Fonts::RobotoMono, 0, 0, 0, 14.0, String::new());
        let menu = PlaneBuilder::new()
            .position(&position + (0.0, size.height, 1.0))
            .size(Size::new(size.width, row_height * items.len() as f32))
            .color((0.15, 0.15, 0.15, 1.0))
            .border_color((0.6, 0.6, 0.6, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(3.0)
            .build();
        let options = items
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                let row_pos = &position + (0.0, size.height + i as f32 * row_height, 2.0);
                let bg = PlaneBuilder::new()
                    .position(row_pos)
                    .size(Size::new(size.width, row_height))
                    .color((0.0, 0.0, 0.0, 0.0))
                    .build();
                let text = FerriteText::new(Fonts::RobotoMono, 0, 0, 0, 14.0, label.clone());
                Option_ {
                    label,
                    text,
                    bg,
                    region: Region::new(row_pos, Size::new(size.width, row_height)),
                }
            })
            .collect();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            selected,
            options,
            is_open: false,
            is_hovering: false,
            hover_index: None,
            row_height,
            button,
            button_text,
            menu,
        }
    }

    fn selected_label(&self) -> &str {
        let i = self.selected.read() as usize;
        self.options.get(i).map(|o| o.label.as_str()).unwrap_or("")
    }
}

impl UIElement for Dropdown {
    fn update(&mut self, _scene: &mut Scene) {
        let base = &self.region.position + &self.region.offset;
        self.button.set_position(base);
        self.button_text
            .set_content(&self.selected_label().to_owned());
        self.button_text
            .prepare_render_at(&base + (6.0, 3.0, 1.0));
        if self.is_open {
            // Lift the open menu well above sibling widgets so later-rendered
            // siblings (tabs, pickers, …) do not z-fight it.
            let overlay_z = 40.0;
            self.menu
                .set_position(&base + (0.0, self.region.size.height, overlay_z));
            for (i, opt) in self.options.iter_mut().enumerate() {
                let p = &base
                    + (
                        0.0,
                        self.region.size.height + i as f32 * self.row_height,
                        overlay_z + 1.0,
                    );
                opt.region.position = p;
                opt.bg.set_position(p);
                opt.bg.set_color(if self.hover_index == Some(i) {
                    (0.25, 0.4, 0.6, 1.0)
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                });
                opt.text.set_content(&opt.label);
                opt.text.prepare_render_at(&p + (6.0, 3.0, 1.0));
                opt.bg.set_z_index(p.z);
                opt.text.set_z_index(p.z + 1.0);
            }
            self.menu.set_z_index(overlay_z);
        }
    }

    fn render(&self) {
        self.button.render();
        self.button_text.render();
        if self.is_open {
            self.menu.render();
            for opt in &self.options {
                opt.bg.render();
                opt.text.render();
            }
        }
    }

    fn handle_events(
        &mut self,
        _scene: &mut Scene,
        window: &mut Window,
        _glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match event {
            MouseButton(MouseButtonLeft, Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if self.region.contains(x, y) {
                    self.is_open = !self.is_open;
                    return true;
                }
                if self.is_open {
                    for (i, opt) in self.options.iter().enumerate() {
                        if opt.region.contains(x, y) {
                            self.selected.write(i as u32);
                            self.is_open = false;
                            return true;
                        }
                    }
                    // Click-away: close and consume so widgets underneath
                    // the menu do not fire from the same click.
                    let over_menu = self
                        .options
                        .iter()
                        .any(|o| o.region.contains(x, y));
                    self.is_open = false;
                    return over_menu;
                }
                false
            }
            CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                let over_button = self.region.contains(x, y);
                let over_menu = self.is_open
                    && self.options.iter().any(|o| o.region.contains(x, y));
                if self.is_open {
                    self.hover_index = self.options.iter().position(|o| o.region.contains(x, y));
                }
                let should_hover = over_button || over_menu;
                if should_hover && !self.is_hovering {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                    self.is_hovering = true;
                    self.button.set_color((0.26, 0.26, 0.26, 1.0));
                } else if !should_hover && self.is_hovering {
                    window.set_cursor(None);
                    self.is_hovering = false;
                    self.button.set_color((0.2, 0.2, 0.2, 1.0));
                }
                over_menu
            }
            _ => false,
        }
    }

    leaf_children_methods!();

    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.region.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.region.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.button.set_z_index(z_index);
        self.button_text.set_z_index(z_index + 1.0);
        self.menu.set_z_index(z_index + 2.0);
        for opt in &mut self.options {
            opt.bg.set_z_index(z_index + 3.0);
            opt.text.set_z_index(z_index + 4.0);
        }
    }
}
