//! Exporting the project as a standalone game (`ferrite::core::export`)
//! in the background, with its progress shown in the status line.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

use ferrite::core::{
    export::{self, Platform},
    project::Project,
};

enum Event {
    Progress(String),
    Done(Result<PathBuf, String>),
}

/// The project folders an export copies (as `ferrite::core::export` does).
const COPIED_FOLDERS: [&str; 3] = ["assets", "animations", "scenes"];

/// The folder of `project_dir` an export copies that `out` lies in, if any:
/// exporting there would copy the export into itself, endlessly.
pub fn copied_folder_containing(project_dir: &Path, out: &Path) -> Option<&'static str> {
    let out = out.canonicalize().ok()?;
    COPIED_FOLDERS.into_iter().find(|folder| {
        project_dir
            .join(folder)
            .canonicalize()
            .is_ok_and(|copied| out.starts_with(copied))
    })
}

#[derive(Default)]
pub struct Exporter {
    running: Option<Receiver<Event>>,
    /// The latest step, repeated while the export runs so it stays shown.
    step: String,
}

impl Exporter {
    /// Let the user choose where to export to (blocks while the dialog is
    /// open). `None` if cancelled.
    pub fn pick_folder() -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(tr!("export.pick_folder"))
            .pick_folder()
    }

    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Export `project` for `platform` into `out` in the background.
    pub fn start(&mut self, project: Project, out: PathBuf, platform: Platform) {
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let progress_send = send.clone();
            let progress = move |step: &str| {
                let _ = progress_send.send(Event::Progress(step.to_string()));
            };
            let result = export::export(&project, &out, platform, &progress);
            let _ = send.send(Event::Done(result));
        });
        self.running = Some(receive);
        self.step = tr!("export.running").into();
    }

    /// The status to show, while exporting and once when it ends. Call
    /// every frame.
    pub fn poll(&mut self) -> Option<String> {
        let receive = self.running.as_ref()?;
        loop {
            match receive.try_recv() {
                Ok(Event::Progress(step)) => self.step = step,
                Ok(Event::Done(result)) => {
                    self.running = None;
                    return Some(match result {
                        Ok(dir) => tr!("export.done", path = dir.display()),
                        Err(e) => tr!("export.failed", error = e),
                    });
                }
                Err(mpsc::TryRecvError::Empty) => return Some(self.step.clone()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.running = None;
                    return Some(tr!("export.stopped").into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_export_into_a_copied_folder_is_found() {
        let dir = std::env::temp_dir().join(format!("ferrite-export-into-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/models")).unwrap();
        std::fs::create_dir_all(dir.join("builds")).unwrap();
        assert_eq!(copied_folder_containing(&dir, &dir.join("assets")), Some("assets"));
        assert_eq!(copied_folder_containing(&dir, &dir.join("assets/models")), Some("assets"));
        assert_eq!(copied_folder_containing(&dir, &dir.join("builds")), None);
        assert_eq!(copied_folder_containing(&dir, &dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
