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

struct Row {
    label: String,
    text: FerriteText,
    bg: Plane,
    region: Region,
}

/// Vertical list of selectable string items bound to `DataSource<u32>`.
pub struct List {
    handle: UIElementHandle,
    region: Region,
    selected: DataSource<u32>,
    rows: Vec<Row>,
    hover_index: Option<usize>,
    row_height: f32,
    frame: Plane,
}

impl List {
    pub fn new(position: Position, size: Size, items: Vec<String>, selected: DataSource<u32>) -> Self {
        let row_height = 20.0;
        let frame = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.12, 0.12, 0.12, 1.0))
            .border_color((0.5, 0.5, 0.5, 1.0))
            .border_thickness(1.0)
            .border_radius_uniform(3.0)
            .build();
        let rows = items
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                let row_pos = &position + (0.0, i as f32 * row_height, 1.0);
                let bg = PlaneBuilder::new()
                    .position(row_pos)
                    .size(Size::new(size.width, row_height))
                    .color((0.0, 0.0, 0.0, 0.0))
                    .build();
                let text = FerriteText::new(Fonts::RobotoMono, 0, 0, 0, 14.0, label.clone());
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
            frame,
        }
    }
}

impl UIElement for List {
    fn update(&mut self, _scene: &mut Scene) {
        let base = &self.region.position + &self.region.offset;
        let sel = self.selected.read() as usize;
        for (i, row) in self.rows.iter_mut().enumerate() {
            let p = &base + (0.0, i as f32 * self.row_height, 1.0);
            row.region.position = p;
            row.bg.set_position(p);
            row.bg.set_color(if i == sel {
                (0.25, 0.4, 0.6, 1.0)
            } else if self.hover_index == Some(i) {
                (0.22, 0.22, 0.22, 1.0)
            } else {
                (0.0, 0.0, 0.0, 0.0)
            });
            row.text.set_content(&row.label);
            row.text.prepare_render_at(&p + (6.0, 3.0, 1.0));
        }
        self.frame.set_position(base);
    }

    fn render(&self) {
        self.frame.render();
        for row in &self.rows {
            row.bg.render();
            row.text.render();
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
                for (i, row) in self.rows.iter().enumerate() {
                    if row.region.contains(x, y) {
                        self.selected.write(i as u32);
                        return true;
                    }
                }
                false
            }
            CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                let was_hovering = self.hover_index.is_some();
                self.hover_index = self
                    .rows
                    .iter()
                    .position(|r| r.region.contains(x, y));
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
