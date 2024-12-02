use core::panic;
use std::{
    str::FromStr,
    sync::{Arc, RwLock},
};

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

pub struct DataSource<T: Clone + ToString> {
    data: Arc<RwLock<T>>,
}

impl<T: Clone + ToString + FromStr> DataSource<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: Arc::new(RwLock::new(data)),
        }
    }

    pub fn read(&self) -> T {
        self.data.read().unwrap().clone()
    }

    pub fn to_string(&self) -> String {
        self.read().to_string()
    }

    pub fn write(&self, data: T) {
        *self.data.write().unwrap() = data;
    }

    pub fn write_from_string(&self, data: String) {
        match data.parse() {
            Ok(data) => self.write(data),
            Err(_) => {
                panic!("Failed to parse string to data type");
            }
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}
