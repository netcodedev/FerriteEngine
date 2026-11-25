//! System file dialogs (`rfd`) on a thread of their own. Opened on the
//! main thread they blocked it until they were closed: the editor drew no
//! frame and answered no window events, and the desktop offered to kill
//! it as "not responding". Now the dialog runs beside the editor, which
//! asks every frame whether it has an answer (`Pending::poll`).

use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// A dialog open on its thread; `poll` hands its answer out once.
pub struct Pending<T> {
    answer: Receiver<T>,
}

/// What `poll` says.
pub enum Poll<T> {
    /// Still open.
    Open,
    /// Closed: what was chosen.
    Done(T),
    /// Its thread ended without an answer (the dialog failed).
    Gone,
}

/// Run `dialog` (which shows a system dialog and returns what was chosen)
/// on a thread of its own.
pub fn open<T: Send + 'static>(dialog: impl FnOnce() -> T + Send + 'static) -> Pending<T> {
    let (send, answer) = channel();
    let spawned = std::thread::Builder::new().name("file-dialog".into()).spawn(move || {
        let _ = send.send(dialog());
    });
    if let Err(e) = spawned {
        log::error!("could not open a file dialog: {e}");
    }
    Pending { answer }
}

impl<T> Pending<T> {
    pub fn poll(&self) -> Poll<T> {
        match self.answer.try_recv() {
            Ok(answer) => Poll::Done(answer),
            Err(TryRecvError::Empty) => Poll::Open,
            Err(TryRecvError::Disconnected) => Poll::Gone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_comes_once_the_dialog_returns() {
        let (go, wait) = channel::<()>();
        let pending = open(move || {
            let _ = wait.recv();
            42
        });
        assert!(matches!(pending.poll(), Poll::Open));
        go.send(()).unwrap();
        let mut answer = None;
        for _ in 0..500 {
            if let Poll::Done(n) = pending.poll() {
                answer = Some(n);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(answer, Some(42));
        assert!(matches!(pending.poll(), Poll::Gone));
    }
}
