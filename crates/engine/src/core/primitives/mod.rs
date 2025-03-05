mod offset;
mod position;
mod region;
mod size;

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

#[derive(Clone, Copy, Debug, Default)]
pub struct Region {
    pub offset: Offset,
    pub position: Position,
    pub size: Size,
}
