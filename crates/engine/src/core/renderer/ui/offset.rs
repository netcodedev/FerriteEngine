use std::ops::Add;

use super::position::Position;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Offset {
    pub x: f32,
    pub y: f32,
}

impl From<Position> for Offset {
    fn from(position: Position) -> Offset {
        Offset {
            x: position.x,
            y: position.y,
        }
    }
}

impl Add<&Offset> for &Offset {
    type Output = Offset;

    fn add(self, rhs: &Offset) -> Offset {
        Offset {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Add<&Position> for &Offset {
    type Output = Offset;

    fn add(self, rhs: &Position) -> Offset {
        Offset {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Add<(f32, f32)> for Offset {
    type Output = Offset;

    fn add(self, rhs: (f32, f32)) -> Offset {
        Offset {
            x: self.x + rhs.0,
            y: self.y + rhs.1,
        }
    }
}

impl Add<(f32, f32)> for &Offset {
    type Output = Offset;

    fn add(self, rhs: (f32, f32)) -> Offset {
        Offset {
            x: self.x + rhs.0,
            y: self.y + rhs.1,
        }
    }
}
