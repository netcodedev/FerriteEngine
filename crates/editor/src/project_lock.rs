//! One editor per project: a project opened in an editor is locked
//! (`.ferrite-editor.lock` in its folder) until that editor closes it.
//!
//! Two editors on one project overwrite each other's scene saves and build
//! the scripts into the same library file, which the other still has
//! loaded (it crashed on exit). So a second editor does not open a locked
//! project: it stays at the launcher and says who has it.
//!
//! The lock holds the process id and when that editor last said it is
//! alive. A lock is stale, and taken over, when that process is gone (or
//! is no editor any more: its id was reused), as Linux's `/proc` tells;
//! elsewhere, when it has not been renewed for `STALE_AFTER`.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The lock's file name in the project folder.
pub const LOCK_FILE: &str = ".ferrite-editor.lock";
/// How often the editor renews the lock.
const HEARTBEAT: Duration = Duration::from_secs(30);
/// A lock not renewed this long is stale (where `/proc` cannot tell).
const STALE_AFTER: Duration = Duration::from_secs(120);

/// A lock on a project, held while it lives.
#[derive(Debug)]
pub struct ProjectLock {
    path: PathBuf,
    pid: u32,
    last_beat: Instant,
}

/// Why a project could not be locked.
#[derive(Debug, PartialEq)]
pub enum LockError {
    /// Another editor, process `pid`, has it.
    Held { pid: u32 },
    Io(String),
}

impl LockError {
    /// What to tell the user about project `name`.
    pub fn message(&self, name: &str) -> String {
        match self {
            LockError::Held { pid } => format!(
                "{name} is already open in another editor (process {pid}). Close it there \
                 first. If that editor is gone, delete {LOCK_FILE} in the project folder."
            ),
            LockError::Io(e) => format!("could not lock {name}: {e}"),
        }
    }
}

/// What a lock file says: its owner and when it last renewed it (seconds
/// since 1970).
fn read(path: &Path) -> Option<(u32, u64)> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let beat = lines.next().and_then(|l| l.trim().parse().ok()).unwrap_or(0);
    Some((pid, beat))
}

fn seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether process `pid` is a running editor: `Some` where the system
/// says, `None` where it cannot tell.
fn editor_alive(pid: u32) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let proc = Path::new("/proc").join(pid.to_string());
        if !proc.exists() {
            return Some(false);
        }
        // A reused id: some other program has it now.
        Some(match fs::read_link(proc.join("exe")) {
            Ok(exe) => exe
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ferrite-editor")),
            Err(_) => true,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Whether a lock by `owner`, last renewed at `beat`, no longer holds at
/// `now`.
fn stale(owner: u32, beat: u64, now: u64, alive: &dyn Fn(u32) -> Option<bool>) -> bool {
    match alive(owner) {
        Some(alive) => !alive,
        None => now.saturating_sub(beat) > STALE_AFTER.as_secs(),
    }
}

impl ProjectLock {
    /// Lock the project in `dir` for this process.
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        Self::acquire_as(dir, std::process::id(), SystemTime::now(), &editor_alive)
    }

    fn acquire_as(
        dir: &Path,
        pid: u32,
        now: SystemTime,
        alive: &dyn Fn(u32) -> Option<bool>,
    ) -> Result<Self, LockError> {
        let path = dir.join(LOCK_FILE);
        // Twice at most: once more after taking a stale lock away.
        for _ in 0..2 {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    write!(file, "{pid}\n{}\n", seconds(now)).map_err(|e| LockError::Io(e.to_string()))?;
                    return Ok(Self {
                        path,
                        pid,
                        last_beat: Instant::now(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let (owner, beat) = read(&path).unwrap_or((0, 0));
                    if owner == pid {
                        // Ours already (opened again after going back).
                        return Ok(Self {
                            path,
                            pid,
                            last_beat: Instant::now(),
                        });
                    }
                    if owner != 0 && !stale(owner, beat, seconds(now), alive) {
                        return Err(LockError::Held { pid: owner });
                    }
                    let _ = fs::remove_file(&path);
                }
                Err(e) => return Err(LockError::Io(e.to_string())),
            }
        }
        Err(LockError::Io(format!("{} keeps coming back", path.display())))
    }

    /// Say this editor is still alive, now and then.
    pub fn heartbeat(&mut self) {
        if self.last_beat.elapsed() < HEARTBEAT {
            return;
        }
        self.last_beat = Instant::now();
        if read(&self.path).is_some_and(|(owner, _)| owner == self.pid) {
            let _ = fs::write(&self.path, format!("{}\n{}\n", self.pid, seconds(SystemTime::now())));
        }
    }
}

impl Drop for ProjectLock {
    /// Unlock, unless another editor took the lock over meanwhile.
    fn drop(&mut self) {
        if read(&self.path).is_some_and(|(owner, _)| owner == self.pid) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// The lock of the project this editor has open.
static HELD: Mutex<Option<ProjectLock>> = Mutex::new(None);

/// Lock the project in `dir` (named `name`) as the one this editor has
/// open, releasing any it had. The error says why it cannot.
pub fn hold(name: &str, dir: &Path) -> Result<(), String> {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if held.as_ref().is_some_and(|lock| lock.path == dir.join(LOCK_FILE)) {
        return Ok(());
    }
    *held = None;
    *held = Some(ProjectLock::acquire(dir).map_err(|e| e.message(name))?);
    Ok(())
}

/// Release the project this editor had open.
pub fn release() {
    *HELD.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Renew the lock now and then (every frame is fine).
pub fn heartbeat() {
    if let Some(lock) = HELD.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        lock.heartbeat();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ferrite-lock-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn a_second_editor_cannot_lock_an_open_project() {
        let dir = folder("second");
        let alive = |_: u32| Some(true);
        let first = ProjectLock::acquire_as(&dir, 100, now(), &alive).unwrap();
        assert_eq!(
            ProjectLock::acquire_as(&dir, 200, now(), &alive).unwrap_err(),
            LockError::Held { pid: 100 }
        );
        // Closed in the first: the second gets it.
        drop(first);
        assert!(!dir.join(LOCK_FILE).exists());
        let second = ProjectLock::acquire_as(&dir, 200, now(), &alive).unwrap();
        assert_eq!(read(&dir.join(LOCK_FILE)).map(|(pid, _)| pid), Some(200));
        drop(second);
    }

    #[test]
    fn the_lock_of_an_editor_that_is_gone_is_taken_over() {
        let dir = folder("gone");
        let lock = ProjectLock::acquire_as(&dir, 100, now(), &|_| Some(true)).unwrap();
        // The first editor crashed: its lock is still there.
        std::mem::forget(lock);
        let taken = ProjectLock::acquire_as(&dir, 200, now(), &|pid| Some(pid != 100)).unwrap();
        assert_eq!(read(&dir.join(LOCK_FILE)).map(|(pid, _)| pid), Some(200));
        drop(taken);
    }

    #[test]
    fn where_the_system_cannot_tell_an_old_lock_is_stale() {
        let dir = folder("old");
        let unknown = |_: u32| None;
        let long_ago = now() - STALE_AFTER - Duration::from_secs(5);
        std::mem::forget(ProjectLock::acquire_as(&dir, 100, long_ago, &unknown).unwrap());
        assert!(ProjectLock::acquire_as(&dir, 200, now(), &unknown).is_ok());

        let dir = folder("fresh");
        std::mem::forget(ProjectLock::acquire_as(&dir, 100, now(), &unknown).unwrap());
        assert_eq!(
            ProjectLock::acquire_as(&dir, 200, now(), &unknown).unwrap_err(),
            LockError::Held { pid: 100 }
        );
    }

    #[test]
    fn a_lock_taken_over_is_not_removed_by_the_editor_that_lost_it() {
        let dir = folder("lost");
        let old = ProjectLock::acquire_as(&dir, 100, now(), &|_| Some(true)).unwrap();
        // Thought dead (it hung), taken over by 200.
        let new = ProjectLock::acquire_as(&dir, 200, now(), &|pid| Some(pid != 100)).unwrap();
        drop(old);
        assert_eq!(read(&dir.join(LOCK_FILE)).map(|(pid, _)| pid), Some(200));
        drop(new);
        assert!(!dir.join(LOCK_FILE).exists());
    }

    #[test]
    fn this_process_is_a_living_editor_or_not_by_its_name() {
        // The test binary is not called ferrite-editor.
        #[cfg(target_os = "linux")]
        assert_eq!(editor_alive(std::process::id()), Some(false));
        // No process has this id.
        #[cfg(target_os = "linux")]
        assert_eq!(editor_alive(u32::MAX - 7), Some(false));
    }

    #[test]
    fn the_message_says_who_has_it_and_what_to_delete() {
        let text = LockError::Held { pid: 42 }.message("demo");
        assert!(text.contains("process 42"));
        assert!(text.contains(".ferrite-editor.lock"));
    }
}
