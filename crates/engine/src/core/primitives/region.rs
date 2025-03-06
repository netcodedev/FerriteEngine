use super::{Offset, Position, Region, Size};

impl Region {
    pub fn new(position: Position, size: Size) -> Self {
        Self {
            position,
            size,
            ..Default::default()
        }
    }

    pub fn new_with_offset(position: Position, size: Size, offset: Offset) -> Self {
        Self {
            offset,
            position,
            size,
        }
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        let x = x - self.position.x;
        let y = y - self.position.y;

        x >= self.offset.x
            && x <= self.offset.x + self.size.width
            && y >= self.offset.y
            && y <= self.offset.y + self.size.height
    }

    pub fn get_absolute_position(self) -> Position {
        Position {
            x: self.position.x + self.offset.x,
            y: self.position.y + self.offset.y,
            z: self.position.z,
        }
    }
}
