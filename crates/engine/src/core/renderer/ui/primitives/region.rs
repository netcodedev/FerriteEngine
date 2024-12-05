use super::{Offset, Position, Region, Size};

impl Region {
    pub fn new(position: Position, size: Size) -> Self {
        Self {
            offset: None,
            position,
            size,
        }
    }

    pub fn new_with_offset(position: Position, size: Size, offset: Offset) -> Self {
        Self {
            offset: Some(offset),
            position,
            size,
        }
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        let x = x - self.position.x;
        let y = y - self.position.y;

        if let Some(offset) = self.offset {
            x >= offset.x
                && x <= offset.x + self.size.width
                && y >= offset.y
                && y <= offset.y + self.size.height
        } else {
            x >= 0.0 && x <= self.size.width && y >= 0.0 && y <= self.size.height
        }
    }
}
