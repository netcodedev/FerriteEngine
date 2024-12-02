use std::cmp::Ordering;

use rand::Rng;
use std::ops::Add;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd)]
pub struct UIElementHandle(u64);

impl UIElementHandle {
    pub fn new() -> Self {
        Self {
            0: rand::thread_rng().gen::<u64>(),
        }
    }
    pub fn from(id: u64) -> Self {
        Self { 0: id }
    }
}

impl Ord for UIElementHandle {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

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

#[derive(Clone, Copy, Debug, PartialOrd, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl PartialEq for Size {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Offset {
    pub x: f32,
    pub y: f32,
}

impl Add<&Offset> for Position {
    type Output = Position;

    fn add(self, rhs: &Offset) -> Position {
        Position {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Add<&Position> for Offset {
    type Output = Offset;

    fn add(self, rhs: &Position) -> Offset {
        Offset {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}
