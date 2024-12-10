use std::cmp::Ordering;

use rand::Rng;

mod offset;
mod position;
mod region;
mod size;

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
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Offset {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialOrd, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

pub struct Region {
    pub offset: Option<Offset>,
    pub position: Position,
    pub size: Size,
}
