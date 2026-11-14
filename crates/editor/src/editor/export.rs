//! Exporting the project as a standalone game (`ferrite::core::export`)
//! in the background, with its progress shown in the status line.

use std::{
    path::PathBuf,
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
            .set_title("Export the game into")
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
        self.step = "Exporting...".into();
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
                        Ok(dir) => format!("Exported to {}", dir.display()),
                        Err(e) => format!("Export failed: {e}"),
                    });
                }
                Err(mpsc::TryRecvError::Empty) => return Some(self.step.clone()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.running = None;
                    return Some("Export failed: it stopped".into());
                }
            }
        }
    }
}
