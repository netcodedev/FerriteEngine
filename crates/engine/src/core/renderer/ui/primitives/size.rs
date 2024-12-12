use std::ops::Add;

use super::Size;

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

impl Add<&Size> for &Size {
    type Output = Size;

    fn add(self, rhs: &Size) -> Size {
        Size {
            width: self.width + rhs.width,
            height: self.height + rhs.height,
        }
    }
}

impl From<(f32, f32)> for Size {
    fn from((width, height): (f32, f32)) -> Size {
        Size { width, height }
    }
}
