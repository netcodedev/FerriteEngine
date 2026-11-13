//! The project's scripts in the editor: loaded on start, built again in the
//! background when a file in the scripts folder changes (or on request),
//! and loaded again when the build succeeds.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant, SystemTime},
};

use ferrite::core::script::{self, load_library, ScriptCrate};

/// How often the scripts folder is checked for changes.
const POLL_INTERVAL: Duration = Duration::from_millis(700);

/// The engine library this editor runs with (the one in `deps`, which
/// cargo puts on the library path). Scripts built against a different one
/// must not be loaded.
fn engine_library_path() -> PathBuf {
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    ferrite::core::script::engine_workspace()
        .join("target")
        .join(profile)
        .join("deps")
        .join(format!("libferrite_dylib{}", std::env::consts::DLL_SUFFIX))
}

/// What identifies a build of the engine library: its GNU build id (a hash
/// of its contents), or its modification time where it has none.
#[derive(PartialEq, Debug)]
enum EngineStamp {
    BuildId(Vec<u8>),
    Modified(Option<SystemTime>),
}

fn engine_stamp() -> EngineStamp {
    let path = engine_library_path();
    match elf_build_id(&path) {
        Some(id) => EngineStamp::BuildId(id),
        None => EngineStamp::Modified(modified(&path)),
    }
}

/// The `.note.gnu.build-id` of an ELF64 little-endian file.
fn elf_build_id(path: &Path) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        let mut buffer = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        Some(buffer)
    };
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as usize;
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize;
    let u64_at = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());

    let header = read_at(0, 64)?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return None;
    }
    let (sh_offset, sh_size, sh_count, names_index) = (
        u64_at(&header, 0x28),
        u16_at(&header, 0x3a),
        u16_at(&header, 0x3c),
        u16_at(&header, 0x3e),
    );
    let sections = read_at(sh_offset, sh_size * sh_count)?;
    let section = |i: usize| &sections[i * sh_size..(i + 1) * sh_size];
    let names_section = section(names_index);
    let names = read_at(u64_at(names_section, 0x18), u64_at(names_section, 0x20) as usize)?;
    for i in 0..sh_count {
        let sh = section(i);
        let name_start = u32_at(sh, 0);
        let name_end = names[name_start..].iter().position(|&b| b == 0)? + name_start;
        if &names[name_start..name_end] != b".note.gnu.build-id" {
            continue;
        }
        let note = read_at(u64_at(sh, 0x18), u64_at(sh, 0x20) as usize)?;
        let (name_size, desc_size) = (u32_at(&note, 0), u32_at(&note, 4));
        let desc_start = 12 + name_size.div_ceil(4) * 4;
        return note.get(desc_start..desc_start + desc_size).map(<[u8]>::to_vec);
    }
    None
}

/// What the editor tells the user after something happened to the scripts.
pub enum ScriptEvent {
    /// A new set of scripts is installed; scenes should be rebuilt with it.
    Loaded { count: usize },
    Building,
    Failed(String),
}

pub struct Scripts {
    scripts: ScriptCrate,
    /// Newest change seen in the scripts folder.
    sources_changed: Option<SystemTime>,
    last_poll: Instant,
    build: Option<Receiver<Result<(), String>>>,
    /// A change came in while a build was running: build again after it.
    rebuild_after: bool,
    /// The engine library as it was when the editor started; a different
    /// one means the engine changed under the running editor.
    engine: EngineStamp,
}

impl Scripts {
    pub fn new(scripts: ScriptCrate) -> Self {
        Self {
            sources_changed: newest_change(scripts.sources()),
            scripts,
            last_poll: Instant::now(),
            build: None,
            rebuild_after: false,
            engine: engine_stamp(),
        }
    }

    pub fn folder(&self) -> &Path {
        self.scripts.sources()
    }

    /// Load the scripts that were just built (on opening a project, before
    /// the first scene is built from them).
    pub fn load_built(&mut self) -> ScriptEvent {
        self.load()
    }

    /// Build the scripts in the background (if not being built already).
    pub fn rebuild(&mut self) -> Option<ScriptEvent> {
        if self.build.is_some() {
            self.rebuild_after = true;
            return None;
        }
        let (send, receive) = mpsc::channel();
        let scripts = self.scripts.clone();
        std::thread::spawn(move || {
            let _ = send.send(scripts.build());
        });
        self.build = Some(receive);
        Some(ScriptEvent::Building)
    }

    /// Check for changed scripts and a finished build. Call every frame.
    pub fn poll(&mut self) -> Option<ScriptEvent> {
        if let Some(build) = &self.build {
            let result = match build.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => Err("the build stopped".into()),
            };
            self.build = None;
            if std::mem::take(&mut self.rebuild_after) {
                return self.rebuild();
            }
            return Some(match result {
                Ok(()) => self.load(),
                Err(e) => ScriptEvent::Failed(e),
            });
        }
        if self.last_poll.elapsed() < POLL_INTERVAL {
            return None;
        }
        self.last_poll = Instant::now();
        let changed = newest_change(self.scripts.sources());
        if changed != self.sources_changed {
            self.sources_changed = changed;
            return self.rebuild();
        }
        None
    }

    fn load(&mut self) -> ScriptEvent {
        if engine_stamp() != self.engine {
            return ScriptEvent::Failed(
                "the engine was rebuilt; restart the editor to load the new scripts".into(),
            );
        }
        match load_library(&self.scripts.library_path()) {
            Ok(registry) => {
                let count = registry.len();
                script::install(registry);
                ScriptEvent::Loaded { count }
            }
            Err(e) => ScriptEvent::Failed(e),
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Newest modification time of any file below `dir`, skipping `target`.
fn newest_change(dir: &Path) -> Option<SystemTime> {
    let mut newest = modified(dir);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return newest;
    };
    for path in entries.flatten().map(|e| e.path()) {
        let time = if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            newest_change(&path)
        } else {
            modified(&path)
        };
        newest = newest.max(time);
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_id_of_an_elf_file_is_read() {
        // This test binary itself is an ELF file with a build id.
        let exe = std::env::current_exe().unwrap();
        let id = elf_build_id(&exe).expect("no build id");
        assert!(id.len() >= 16);
        assert_eq!(elf_build_id(Path::new("Cargo.toml")), None);
    }
}
