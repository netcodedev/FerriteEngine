//! Model files read in the background: a loader asks with `poll` every
//! frame until its `PreparedModel` is there, and uploads it then (see
//! `prepared`). Meanwhile the frame goes on. `pending` counts what is on
//! its way, so a loading screen can wait for it.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
};

use crate::core::assets::AssetId;

use super::PreparedModel;

/// Threads reading models. Each decodes its maps on all cores, so a few
/// are enough.
const WORKERS: usize = 2;

/// Model files being read now.
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// A read model not taken this long is dropped (its loader gave up on it,
/// e.g. a scatter graph changed meanwhile).
const KEEP_UNTAKEN: std::time::Duration = std::time::Duration::from_secs(30);

/// Model files being read now: a loading screen waits while this is above
/// zero (what is read is taken and uploaded by its loader the next frame).
pub fn pending() -> usize {
    PENDING.load(Ordering::Relaxed)
}

/// What `poll` says about a model.
pub enum Poll {
    /// Read: upload it now (`ModelBuilder::from_prepared`).
    Ready(PreparedModel),
    /// Still being read: ask again next frame.
    Waiting,
    Failed(String),
}

struct Job {
    key: String,
    path: String,
    materials: Vec<AssetId>,
}

struct State {
    jobs: Sender<Job>,
    done: Receiver<(String, Result<PreparedModel, String>)>,
    /// Asked for, not yet read.
    waiting: HashSet<String>,
    /// Read, not yet taken, since when.
    ready: HashMap<String, (std::time::Instant, Result<PreparedModel, String>)>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn start() -> State {
    let (jobs, inbox) = channel::<Job>();
    let (outbox, done) = channel();
    let inbox = Arc::new(Mutex::new(inbox));
    for index in 0..WORKERS {
        let inbox = inbox.clone();
        let outbox = outbox.clone();
        let _ = std::thread::Builder::new().name(format!("model-loader-{index}")).spawn(move || loop {
            let job = match inbox.lock() {
                Ok(inbox) => inbox.recv(),
                Err(_) => break,
            };
            let Ok(job) = job else { break };
            let result = PreparedModel::load(&job.path).map(|p| p.with_material_textures(&job.materials));
            if outbox.send((job.key, result)).is_err() {
                break;
            }
        });
    }
    State { jobs, done, waiting: HashSet::new(), ready: HashMap::new() }
}

/// The model file `path` (relative to the working folder) for the loader
/// that knows it as `key`, with the texture assets of `materials` decoded
/// too. The first call starts reading it; call again each frame until it
/// is `Ready` (handed out once) or `Failed`.
pub fn poll(key: &str, path: &str, materials: &[AssetId]) -> Poll {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let state = state.get_or_insert_with(start);
        while let Ok((key, result)) = state.done.try_recv() {
            if state.waiting.remove(&key) {
                PENDING.fetch_sub(1, Ordering::Relaxed);
                state.ready.insert(key, (std::time::Instant::now(), result));
            }
        }
        state.ready.retain(|_, (since, _)| since.elapsed() < KEEP_UNTAKEN);
        if let Some((_, result)) = state.ready.remove(key) {
            return match result {
                Ok(prepared) => Poll::Ready(prepared),
                Err(e) => Poll::Failed(e),
            };
        }
        if state.waiting.insert(key.to_string()) {
            PENDING.fetch_add(1, Ordering::Relaxed);
            let job = Job { key: key.to_string(), path: path.to_string(), materials: materials.to_vec() };
            if state.jobs.send(job).is_err() {
                state.waiting.remove(key);
                PENDING.fetch_sub(1, Ordering::Relaxed);
                return Poll::Failed("the model loader stopped".into());
            }
        }
        Poll::Waiting
    })
}

/// Forget a request nobody will take any more (the loader gave up on it).
pub fn cancel(key: &str) {
    STATE.with(|state| {
        if let Some(state) = state.borrow_mut().as_mut() {
            if state.waiting.remove(key) {
                PENDING.fetch_sub(1, Ordering::Relaxed);
            }
            state.ready.remove(key);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_fails_and_is_no_longer_pending() {
        let before = pending();
        let key = "test:missing";
        let mut result = poll(key, "no/such/model.glb", &[]);
        assert_eq!(pending(), before + 1);
        for _ in 0..500 {
            if !matches!(result, Poll::Waiting) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            result = poll(key, "no/such/model.glb", &[]);
        }
        assert!(matches!(result, Poll::Failed(_)));
        assert_eq!(pending(), before);
        // Not taken again.
        assert!(matches!(poll(key, "no/such/model.glb", &[]), Poll::Waiting));
        cancel(key);
        assert_eq!(pending(), before);
    }
}
