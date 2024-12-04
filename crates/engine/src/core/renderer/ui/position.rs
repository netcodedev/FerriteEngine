use std::ops::Add;

use super::offset::Offset;

#[derive(Clone, Copy, Debug, PartialOrd, Default)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl PartialEq for Position {
    fn eq(&self, other: &Self) -> bool {
        self.x == other.x && self.y == other.y
    }
}

impl Add<&Offset> for &Position {
    type Output = Position;

    fn add(self, rhs: &Offset) -> Position {
        Position {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Add<(f32, f32)> for &Position {
    type Output = Position;

    fn add(self, rhs: (f32, f32)) -> Position {
        Position {
            x: self.x + rhs.0,
            y: self.y + rhs.1,
        }
    }
}
impl From<Offset> for Position {
    fn from(offset: Offset) -> Position {
        Position {
            x: offset.x,
            y: offset.y,
        }
    }
}
