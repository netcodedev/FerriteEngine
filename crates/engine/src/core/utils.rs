use std::sync::{Arc, RwLock};

use russimp::Matrix4x4;

pub trait ToMatrix4 {
    fn to_matrix_4(&self) -> cgmath::Matrix4<f32>;
}

impl ToMatrix4 for Matrix4x4 {
    fn to_matrix_4(&self) -> cgmath::Matrix4<f32> {
        cgmath::Matrix4::new(
            self.a1, self.b1, self.c1, self.d1, self.a2, self.b2, self.c2, self.d2, self.a3,
            self.b3, self.c3, self.d3, self.a4, self.b4, self.c4, self.d4,
        )
    }
}

pub struct DataSource<T> {
    data: Arc<RwLock<T>>,
}

impl<T: Clone> DataSource<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: Arc::new(RwLock::new(data)),
        }
    }

    pub fn read(&self) -> T {
        self.data.read().unwrap().clone()
    }

    pub fn write(&self, data: T) {
        *self.data.write().unwrap() = data;
    }

    pub fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}
