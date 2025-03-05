use std::ops::Add;

use super::{Offset, Position, Size};

impl Position {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Position { x, y, z }
    }
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
            z: self.z,
        }
    }
}

impl Add<(f32, f32)> for &Position {
    type Output = Position;

    fn add(self, rhs: (f32, f32)) -> Position {
        Position {
            x: self.x + rhs.0,
            y: self.y + rhs.1,
            z: self.z,
        }
    }
}

impl Add<(f32, f32, f32)> for &Position {
    type Output = Position;

    fn add(self, rhs: (f32, f32, f32)) -> Position {
        Position {
            x: self.x + rhs.0,
            y: self.y + rhs.1,
            z: self.z + rhs.2,
        }
    }
}

impl From<(f32, f32)> for Position {
    fn from((x, y): (f32, f32)) -> Position {
        Position { x, y, z: 0.0 }
    }
}

impl From<(f32, f32, f32)> for Position {
    fn from((x, y, z): (f32, f32, f32)) -> Position {
        Position { x, y, z }
    }
}

impl From<Offset> for Position {
    fn from(offset: Offset) -> Position {
        Position {
            x: offset.x,
            y: offset.y,
            z: 0.0,
        }
    }
}

impl From<Size> for Position {
    fn from(size: Size) -> Position {
        Position {
            x: size.width,
            y: size.height,
            z: 0.0,
        }
    }
}
