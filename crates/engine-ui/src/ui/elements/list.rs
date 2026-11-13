use crate::ui::theme;
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
    WindowEvent::{self, CursorPos, MouseButton, Scroll},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

struct Row {
    label: String,
    text: FerriteText,
    bg: Plane,
    region: Region,
}

/// Vertical list of selectable string items bound to `DataSource<u32>`.
/// Rows beyond its height are clipped and reached with the scroll wheel.
pub struct List {
    handle: UIElementHandle,
    region: Region,
    selected: DataSource<u32>,
    rows: Vec<Row>,
    hover_index: Option<usize>,
    row_height: f32,
    scroll_y: f32,
    frame: Plane,
}

impl List {
    pub fn new(position: Position, size: Size, items: Vec<String>, selected: DataSource<u32>) -> Self {
        let row_height = 20.0;
        let frame = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color(theme::BG_FIELD)
            .border_color(theme::BORDER)
            .border_thickness(1.0)
            .border_radius_uniform(theme::RADIUS)
            .build();
        let rows = items
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                let row_pos = &position + (0.0, i as f32 * row_height, 1.0);
                let bg = PlaneBuilder::new()
                    .position(row_pos)
                    .size(Size::new(size.width, row_height))
                    .color(theme::TRANSPARENT)
                    .build();
                let text = FerriteText::new(Fonts::Inter, 0, 0, 0, 14.0, label.clone());
                Row {
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
            rows,
            hover_index: None,
            row_height,
            scroll_y: 0.0,
            frame,
        }
    }

    fn max_scroll(&self) -> f32 {
        (self.rows.len() as f32 * self.row_height - self.region.size.height).max(0.0)
    }

    fn is_row_visible(&self, row: &Row) -> bool {
        let top = self.region.position.y + self.region.offset.y;
        let bottom = top + self.region.size.height;
        row.region.position.y < bottom && row.region.position.y + self.row_height > top
    }

    /// The row under the cursor, if it is inside the list's visible area.
    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.region.contains(x, y) {
            return None;
        }
        self.rows.iter().position(|r| r.region.contains(x, y))
    }
}

impl UIElement for List {
    fn update(&mut self, _scene: &mut Scene) {
        let base = &self.region.position + &self.region.offset;
        let sel = self.selected.read() as usize;
        for (i, row) in self.rows.iter_mut().enumerate() {
            let p = &base + (0.0, i as f32 * self.row_height - self.scroll_y, 1.0);
            row.region.position = p;
            row.bg.set_position(p);
            row.bg.set_color(if i == sel {
                theme::ACCENT_SOFT
            } else if self.hover_index == Some(i) {
                theme::BG_ROW_HOVER
            } else {
                theme::TRANSPARENT
            });
            row.text.set_content(&row.label);
            row.text.prepare_render_at(&p + (6.0, 3.0, 1.0));
        }
        self.frame.set_position(base);
    }

    fn render(&self) {
        self.frame.render();
        crate::ui::clip::push(&self.frame);
        for row in self.rows.iter().filter(|row| self.is_row_visible(row)) {
            row.bg.render();
            row.text.render();
        }
        crate::ui::clip::pop(&self.frame);
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
                match self.row_at(x as f32, y as f32) {
                    Some(i) => {
                        self.selected.write(i as u32);
                        true
                    }
                    None => false,
                }
            }
            Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if !self.region.contains(x as f32, y as f32) {
                    return false;
                }
                let before = self.scroll_y;
                self.scroll_y = (self.scroll_y - *dy as f32 * self.row_height)
                    .clamp(0.0, self.max_scroll());
                (self.scroll_y - before).abs() > f32::EPSILON
            }
            CursorPos(x, y) => {
                let was_hovering = self.hover_index.is_some();
                self.hover_index = self.row_at(*x as f32, *y as f32);
                let is_hovering = self.hover_index.is_some();
                if is_hovering && !was_hovering {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                } else if !is_hovering && was_hovering {
                    window.set_cursor(None);
                }
                false
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
        self.frame.set_z_index(z_index);
        for row in &mut self.rows {
            row.bg.set_z_index(z_index + 1.0);
            row.text.set_z_index(z_index + 2.0);
        }
    }
}
