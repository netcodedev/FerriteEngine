use std::ops::Add;

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

impl Add<(f32, f32)> for &Size {
    type Output = Size;

    fn add(self, rhs: (f32, f32)) -> Size {
        Size {
            width: self.width + rhs.0,
            height: self.height + rhs.1,
        }
    }
}
