//! The asset pack of an exported game (docs/asset-pack.md): one file with
//! the project's models as cooked scenes and its images as BC7 blocks,
//! read with positioned reads from any thread. `open` it once at start;
//! the loaders ask `entry` / `read` through `vfs` and fall back to loose
//! files for what it does not hold.

use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    path::{Component, Path, PathBuf},
    sync::RwLock,
};

use serde::{Deserialize, Serialize};

/// The pack's file name, next to the project file.
pub const FILE_NAME: &str = "assets.pack";
const MAGIC: &[u8; 4] = b"FPAK";
/// Changes when the layout changes; a pack of another version is ignored.
pub const VERSION: u32 = 1;

/// What an entry holds.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A model file read with Assimp, as `cooked_scene` writes it.
    Scene,
    /// An image as BC7 blocks with every mip level, the largest first.
    Bc7 { width: u32, height: u32, mips: u32 },
    /// A file's bytes as they are.
    Raw,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    /// Project relative, '/' between folders.
    pub path: String,
    pub kind: Kind,
    pub offset: u64,
    pub length: u64,
}

struct Pack {
    file: File,
    entries: HashMap<String, Entry>,
    /// Folder → names of the files in it.
    folders: HashMap<String, Vec<String>>,
}

static PACK: RwLock<Option<Pack>> = RwLock::new(None);

/// Open the pack at `path`; the loaders read from it from now on.
pub fn open(path: &Path) -> Result<usize, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut header = [0u8; 24];
    read_at(&file, &mut header, 0).map_err(|e| format!("{}: {e}", path.display()))?;
    if &header[0..4] != MAGIC {
        return Err(format!("{} is not an asset pack", path.display()));
    }
    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if version != VERSION {
        return Err(format!("{}: pack version {version}, this game reads {VERSION}", path.display()));
    }
    let index_at = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let index_length = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let mut index = vec![0u8; index_length as usize];
    read_at(&file, &mut index, index_at).map_err(|e| format!("{}: {e}", path.display()))?;
    let list: Vec<Entry> = bincode::deserialize(&index).map_err(|e| format!("{}: {e}", path.display()))?;
    let count = list.len();
    let mut entries = HashMap::with_capacity(count);
    let mut folders: HashMap<String, Vec<String>> = HashMap::new();
    for entry in list {
        let (folder, name) = match entry.path.rsplit_once('/') {
            Some((folder, name)) => (folder.to_string(), name.to_string()),
            None => (String::new(), entry.path.clone()),
        };
        folders.entry(folder).or_default().push(name);
        entries.insert(entry.path.clone(), entry);
    }
    *PACK.write().unwrap_or_else(|e| e.into_inner()) = Some(Pack { file, entries, folders });
    log::info!("asset pack {} with {count} entries", path.display());
    Ok(count)
}

/// Forget the pack (tests; a project closed).
pub fn close() {
    *PACK.write().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn is_open() -> bool {
    PACK.read().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// `path` as the pack names it: project relative, '/' separated, `.` and
/// `..` resolved. `None` for a path outside the project.
pub fn key(path: &Path) -> Option<String> {
    let relative = if path.is_absolute() {
        let here = std::env::current_dir().ok()?;
        path.strip_prefix(&here).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// The entry for `path`, if the pack holds it.
pub fn entry(path: &Path) -> Option<Entry> {
    let pack = PACK.read().unwrap_or_else(|e| e.into_inner());
    let pack = pack.as_ref()?;
    pack.entries.get(&key(path)?).cloned()
}

/// The bytes of `entry`.
pub fn read(entry: &Entry) -> Result<Vec<u8>, String> {
    let pack = PACK.read().unwrap_or_else(|e| e.into_inner());
    let pack = pack.as_ref().ok_or("no asset pack is open")?;
    let mut bytes = vec![0u8; entry.length as usize];
    read_at(&pack.file, &mut bytes, entry.offset).map_err(|e| format!("{}: {e}", entry.path))?;
    Ok(bytes)
}

/// The files the pack holds in `folder` (not below it), as paths under
/// `folder` as given.
pub fn files_in(folder: &Path) -> Vec<PathBuf> {
    let pack = PACK.read().unwrap_or_else(|e| e.into_inner());
    let (Some(pack), Some(key)) = (pack.as_ref(), key(folder)) else {
        return Vec::new();
    };
    pack.folders.get(&key).map_or_else(Vec::new, |names| names.iter().map(|n| folder.join(n)).collect())
}

fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buffer, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buffer.len() {
            let n = file.seek_read(&mut buffer[done..], offset + done as u64)?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
}

/// Writes a pack: entries one after the other, then the index.
pub struct PackWriter {
    file: std::io::BufWriter<File>,
    at: u64,
    entries: BTreeMap<String, Entry>,
}

impl PackWriter {
    pub fn create(path: &Path) -> Result<Self, String> {
        use std::io::Write;
        let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut file = std::io::BufWriter::new(file);
        // The header is written again at the end, with the index's place.
        file.write_all(&[0u8; 24]).map_err(|e| e.to_string())?;
        Ok(Self { file, at: 24, entries: BTreeMap::new() })
    }

    /// Add `bytes` as the entry for `path` (project relative).
    pub fn add(&mut self, path: &str, kind: Kind, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        self.file.write_all(bytes).map_err(|e| e.to_string())?;
        let entry = Entry { path: path.to_string(), kind, offset: self.at, length: bytes.len() as u64 };
        self.at += bytes.len() as u64;
        self.entries.insert(path.to_string(), entry);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Write the index and the header. Returns the pack's size.
    pub fn finish(mut self) -> Result<u64, String> {
        use std::io::{Seek, SeekFrom, Write};
        let list: Vec<Entry> = self.entries.into_values().collect();
        let index = bincode::serialize(&list).map_err(|e| e.to_string())?;
        self.file.write_all(&index).map_err(|e| e.to_string())?;
        let mut header = Vec::with_capacity(24);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&VERSION.to_le_bytes());
        header.extend_from_slice(&self.at.to_le_bytes());
        header.extend_from_slice(&(index.len() as u64).to_le_bytes());
        self.file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        self.file.write_all(&header).map_err(|e| e.to_string())?;
        self.file.flush().map_err(|e| e.to_string())?;
        Ok(self.at + index.len() as u64)
    }
}

/// Tests that open packs run one at a time: the pack is the process's.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_project_relative_with_slashes() {
        assert_eq!(key(Path::new("assets/models/a.fbx")).as_deref(), Some("assets/models/a.fbx"));
        assert_eq!(key(Path::new("./assets/models/x/../a.fbx")).as_deref(), Some("assets/models/a.fbx"));
        assert_eq!(key(Path::new("../outside.png")), None);
        let here = std::env::current_dir().unwrap();
        assert_eq!(key(&here.join("assets/b.png")).as_deref(), Some("assets/b.png"));
    }

    #[test]
    fn a_written_pack_reads_back() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!("ferrite-pack-{}.pack", std::process::id()));
        let mut writer = PackWriter::create(&path).unwrap();
        writer.add("assets/models/a.bin", Kind::Raw, b"hello").unwrap();
        writer.add("assets/textures/b.png", Kind::Bc7 { width: 4, height: 4, mips: 3 }, &[7u8; 48]).unwrap();
        writer.finish().unwrap();
        assert_eq!(open(&path).unwrap(), 2);
        let a = entry(Path::new("assets/models/a.bin")).unwrap();
        assert_eq!(read(&a).unwrap(), b"hello");
        let b = entry(Path::new("./assets/textures/b.png")).unwrap();
        assert_eq!(b.kind, Kind::Bc7 { width: 4, height: 4, mips: 3 });
        assert_eq!(read(&b).unwrap(), vec![7u8; 48]);
        assert!(entry(Path::new("assets/none")).is_none());
        assert_eq!(files_in(Path::new("assets/textures")), vec![PathBuf::from("assets/textures/b.png")]);
        close();
        assert!(entry(Path::new("assets/models/a.bin")).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
