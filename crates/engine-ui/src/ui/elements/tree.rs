use std::collections::HashSet;

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
    Action::Press,
    Glfw, MouseButtonLeft, Window,
    WindowEvent::{self, CursorPos, MouseButton, Scroll},
};

use crate::ui::{element_handle::UIElementHandle, UIElement};

/// One node of a `TreeView`, listed in depth-first order.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeItem {
    pub id: u64,
    pub label: String,
    pub depth: usize,
    pub has_children: bool,
}

/// Value written to the selection source when nothing is selected.
pub const NO_SELECTION: u64 = 0;

struct Row {
    item: usize,
    bg: Plane,
    arrow: FerriteText,
    text: FerriteText,
    region: Region,
    arrow_region: Region,
}

/// Collapsible, scrollable tree of labelled items with single selection.
/// The selected item's id is written to a shared `DataSource<u64>`
/// (`NO_SELECTION` when empty). Feed it with `set_items` whenever the
/// underlying data changes; collapse state is kept across updates.
pub struct TreeView {
    handle: UIElementHandle,
    region: Region,
    selected: DataSource<u64>,
    items: Vec<TreeItem>,
    collapsed: HashSet<u64>,
    rows: Vec<Row>,
    hover: Option<usize>,
    is_hovering: bool,
    row_height: f32,
    indent: f32,
    scroll_y: f32,
    font_size: f32,
    frame: Plane,
    dirty: bool,
}

impl TreeView {
    pub fn new(position: Position, size: Size, selected: DataSource<u64>) -> Self {
        let frame = PlaneBuilder::new()
            .position(position)
            .size(size)
            .color((0.13, 0.13, 0.13, 1.0))
            .border_radius_uniform(3.0)
            .build();
        Self {
            handle: UIElementHandle::new(),
            region: Region::new(position, size),
            selected,
            items: Vec::new(),
            collapsed: HashSet::new(),
            rows: Vec::new(),
            hover: None,
            is_hovering: false,
            row_height: 20.0,
            indent: 14.0,
            scroll_y: 0.0,
            font_size: 14.0,
            frame,
            dirty: true,
        }
    }

    pub fn set_items(&mut self, items: Vec<TreeItem>) {
        if items != self.items {
            self.items = items;
            self.dirty = true;
        }
    }

    pub fn set_size(&mut self, size: Size) {
        self.region.size = size;
        self.frame.set_size(size);
        self.clamp_scroll();
        self.dirty = true;
    }

    pub fn set_position(&mut self, position: Position) {
        self.region.position = Position::new(position.x, position.y, self.region.position.z);
        self.dirty = true;
    }

    /// Expand every ancestor of `id` and scroll it into view.
    pub fn reveal(&mut self, id: u64) {
        let Some(index) = self.items.iter().position(|i| i.id == id) else {
            return;
        };
        let mut depth = self.items[index].depth;
        for item in self.items[..index].iter().rev() {
            if item.depth < depth {
                self.collapsed.remove(&item.id);
                depth = item.depth;
            }
        }
        self.dirty = true;
        self.layout();
        if let Some(row) = self.rows.iter().position(|r| r.item == index) {
            let top = row as f32 * self.row_height;
            let bottom = top + self.row_height;
            if top < self.scroll_y {
                self.scroll_y = top;
            } else if bottom > self.scroll_y + self.region.size.height {
                self.scroll_y = bottom - self.region.size.height;
            }
            self.dirty = true;
        }
    }

    fn visible_items(&self) -> Vec<usize> {
        let mut visible = Vec::new();
        let mut hidden_below: Option<usize> = None;
        for (i, item) in self.items.iter().enumerate() {
            if let Some(depth) = hidden_below {
                if item.depth > depth {
                    continue;
                }
                hidden_below = None;
            }
            visible.push(i);
            if item.has_children && self.collapsed.contains(&item.id) {
                hidden_below = Some(item.depth);
            }
        }
        visible
    }

    fn content_height(&self) -> f32 {
        self.rows.len() as f32 * self.row_height
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_height() - self.region.size.height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    fn layout(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let visible = self.visible_items();
        let z = self.region.position.z;
        while self.rows.len() < visible.len() {
            self.rows.push(Row {
                item: 0,
                bg: PlaneBuilder::new()
                    .color((0.0, 0.0, 0.0, 0.0))
                    .border_radius_uniform(2.0)
                    .build(),
                arrow: FerriteText::new(Fonts::RobotoMono, 0, 0, 0, self.font_size, String::new()),
                text: FerriteText::new(Fonts::RobotoMono, 0, 0, 0, self.font_size, String::new()),
                region: Region::default(),
                arrow_region: Region::default(),
            });
        }
        self.rows.truncate(visible.len());
        self.clamp_scroll();

        let base = self.region.get_absolute_position();
        let width = self.region.size.width;
        for (row_index, item_index) in visible.into_iter().enumerate() {
            let item = &self.items[item_index];
            let row = &mut self.rows[row_index];
            row.item = item_index;
            let y = row_index as f32 * self.row_height - self.scroll_y;
            let row_pos = Position::new(base.x, base.y + y, z + 1.0);
            row.region = Region::new(row_pos, Size::new(width, self.row_height));
            row.bg.set_position(row_pos);
            row.bg.set_size(Size::new(width, self.row_height));
            row.bg.set_z_index(z + 1.0);

            let indent = 4.0 + item.depth as f32 * self.indent;
            let arrow = match (item.has_children, self.collapsed.contains(&item.id)) {
                (false, _) => "",
                (true, true) => "+",
                (true, false) => "-",
            };
            row.arrow.set_content(arrow);
            row.arrow
                .prepare_render_at(Position::new(row_pos.x + indent, row_pos.y + 2.0, z + 2.0));
            row.arrow.set_z_index(z + 2.0);
            row.arrow_region = Region::new(
                Position::new(row_pos.x + indent - 2.0, row_pos.y, z),
                Size::new(self.indent, self.row_height),
            );
            row.text.set_content(&item.label);
            row.text.prepare_render_at(Position::new(
                row_pos.x + indent + self.indent,
                row_pos.y + 2.0,
                z + 2.0,
            ));
            row.text.set_z_index(z + 2.0);
        }
    }

    fn row_color(&self, row_index: usize) -> (f32, f32, f32, f32) {
        let item = &self.items[self.rows[row_index].item];
        if item.id == self.selected.read() {
            (0.25, 0.4, 0.6, 1.0)
        } else if self.hover == Some(row_index) {
            (0.22, 0.22, 0.22, 1.0)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        }
    }

    fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.region.contains(x, y) {
            return None;
        }
        self.rows.iter().position(|r| r.region.contains(x, y))
    }
}

impl UIElement for TreeView {
    fn update(&mut self, _scene: &mut Scene) {
        self.layout();
        for i in 0..self.rows.len() {
            let color = self.row_color(i);
            self.rows[i].bg.set_color(color);
        }
        self.frame.set_position(self.region.get_absolute_position());
    }

    fn render(&self) {
        self.frame.render();
        crate::ui::clip::push(&self.frame);
        for row in &self.rows {
            row.bg.render();
            row.arrow.render();
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
                let (x, y) = (x as f32, y as f32);
                let Some(row_index) = self.row_at(x, y) else {
                    return false;
                };
                let row = &self.rows[row_index];
                let item = &self.items[row.item];
                if item.has_children && row.arrow_region.contains(x, y) {
                    if !self.collapsed.remove(&item.id) {
                        self.collapsed.insert(item.id);
                    }
                    self.dirty = true;
                } else {
                    self.selected.write(item.id);
                }
                true
            }
            CursorPos(x, y) => {
                let (x, y) = (*x as f32, *y as f32);
                self.hover = self.row_at(x, y);
                let inside = self.hover.is_some();
                if inside && !self.is_hovering {
                    window.set_cursor(Some(glfw::Cursor::standard(glfw::StandardCursor::Hand)));
                } else if !inside && self.is_hovering {
                    window.set_cursor(None);
                }
                self.is_hovering = inside;
                false
            }
            Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if self.region.contains(x as f32, y as f32) {
                    self.scroll_y -= *dy as f32 * self.row_height * 2.0;
                    self.clamp_scroll();
                    self.dirty = true;
                    return true;
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
        if self.region.offset != offset {
            self.region.offset = offset;
            self.dirty = true;
        }
    }
    fn get_size(&self) -> &Size {
        &self.region.size
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.region.position.z = z_index;
        self.frame.set_z_index(z_index);
        self.dirty = true;
    }
}
