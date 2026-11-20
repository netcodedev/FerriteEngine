use core::panic;
use std::{
    str::FromStr,
    sync::{Arc, RwLock},
};

use russimp_ng::Matrix4x4;

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

#[derive(Debug)]
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

    /// Parse and store `data`; returns `false` (leaving the value untouched)
    /// when it does not parse, e.g. a half-typed number like `-` or `1e`.
    pub fn try_write_from_string(&self, data: &str) -> bool {
        match data.parse() {
            Ok(data) => {
                self.write(data);
                true
            }
            Err(_) => false,
        }
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

/// Write `contents` to `path` whole or not at all: into a file next to it
/// first, which then replaces it. A crash or a full disk while saving leaves
/// the file as it was, not cut off.
pub fn write_atomically(path: &std::path::Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = std::path::PathBuf::from(partial);
    let written = std::fs::write(&partial, contents).and_then(|()| std::fs::rename(&partial, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

#[cfg(test)]
mod write_tests {
    use super::write_atomically;

    #[test]
    fn a_file_is_replaced_whole_and_nothing_is_left_beside_it() {
        let dir = std::env::temp_dir().join(format!("ferrite-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.ron");
        write_atomically(&path, "first").unwrap();
        write_atomically(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(files, ["scene.ron"]);
        // Into a folder that is not there: an error, and no file.
        assert!(write_atomically(&dir.join("missing").join("x.ron"), "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
