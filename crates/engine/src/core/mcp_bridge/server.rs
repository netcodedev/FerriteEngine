//! The TCP listener and connection handling for the live bridge.
//!
//! Each connection runs on its own thread, reads JSON requests line by
//! line, and waits for the editor's main thread to answer. Requests
//! travel through a shared queue; responses travel back over an mpsc
//! channel, one per pending request, so a long-running screenshot never
//! blocks a quick status query behind it.

use std::{
    io::{BufRead, BufReader, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread,
    time::Duration,
};

use super::protocol::{BridgeRequest, BridgeResponse};

/// Lockfile the editor writes the bridge port to.
pub const LOCKFILE_NAME: &str = ".ferrite-mcp.lock";

/// A request waiting for the editor's main thread.
pub struct PendingRequest {
    pub request: BridgeRequest,
    reply: mpsc::Sender<BridgeResponse>,
    /// To find it in the queue again (see `wait`).
    id: u64,
}

impl PendingRequest {
    /// Answer this request.
    pub fn respond(self, response: BridgeResponse) {
        // Ignore the send error; the client may have disconnected.
        let _ = self.reply.send(response);
    }
}

struct State {
    /// Requests that have come in but have not been serviced yet.
    pending: Mutex<Vec<PendingRequest>>,
    /// Where we wrote the lockfile. We remove it on stop.
    lockfile: Mutex<Option<PathBuf>>,
    /// Which bridge runs: `start` and `stop` both count it up. The threads
    /// of a bridge end once it is no longer theirs.
    generation: AtomicU64,
    /// The running bridge's open connections (by id), closed on `stop`.
    connections: Mutex<Vec<(u64, TcpStream)>>,
    /// Ids of requests and connections.
    next_id: AtomicU64,
}

static STATE: OnceLock<State> = OnceLock::new();

fn state() -> &'static State {
    STATE.get_or_init(|| State {
        pending: Mutex::new(Vec::new()),
        lockfile: Mutex::new(None),
        generation: AtomicU64::new(0),
        connections: Mutex::new(Vec::new()),
        next_id: AtomicU64::new(0),
    })
}

/// How long a client waits for the editor to answer a request.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// Start the bridge for a project. Safe to call twice: subsequent
/// calls are no-ops if a bridge is already running.
///
/// Returns the port the bridge is listening on.
pub fn start(project_dir: &Path) -> std::io::Result<u16> {
    let state = state();
    {
        let lockfile = state.lockfile.lock().unwrap();
        if lockfile.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "bridge already running",
            ));
        }
    }

    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();

    let path = project_dir.join(LOCKFILE_NAME);
    std::fs::write(&path, port.to_string())?;
    *state.lockfile.lock().unwrap() = Some(path);
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;

    thread::spawn(move || listen(listener, generation));
    Ok(port)
}

/// Stop the bridge and remove the lockfile: it accepts no more
/// connections, closes the open ones, and answers what is still queued
/// with an error. Safe to call twice.
pub fn stop() {
    let state = state();
    // First: a connection accepted from now on is not registered.
    state.generation.fetch_add(1, Ordering::SeqCst);
    for (_, stream) in state.connections.lock().unwrap().drain(..) {
        let _ = stream.shutdown(Shutdown::Both);
    }
    // Their clients hear that it stopped (see `wait`).
    state.pending.lock().unwrap().clear();
    if let Some(path) = state.lockfile.lock().unwrap().take() {
        let _ = std::fs::remove_file(path);
    }
}

/// Take every pending request. Called by the editor once per frame.
pub fn drain_pending() -> Vec<PendingRequest> {
    std::mem::take(&mut *state().pending.lock().unwrap())
}

/// Whether the bridge `generation` was stopped (or another started).
fn is_shutdown(generation: u64) -> bool {
    state().generation.load(Ordering::SeqCst) != generation
}

fn listen(listener: TcpListener, generation: u64) {
    loop {
        if is_shutdown(generation) {
            break;
        }
        match listener.accept() {
            Ok((stream, _addr)) => {
                let id = state().next_id.fetch_add(1, Ordering::Relaxed);
                {
                    let mut connections = state().connections.lock().unwrap();
                    // Checked under the lock `stop` closes them with.
                    if is_shutdown(generation) {
                        break;
                    }
                    match stream.try_clone() {
                        Ok(clone) => connections.push((id, clone)),
                        Err(e) => {
                            log::warn!("mcp_bridge clone: {e}");
                            continue;
                        }
                    }
                }
                thread::spawn(move || {
                    handle_connection(stream, generation);
                    state().connections.lock().unwrap().retain(|(other, _)| *other != id);
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                log::warn!("mcp_bridge accept: {e}");
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

fn handle_connection(stream: TcpStream, generation: u64) {
    // Blocking reads on the connection are fine: each connection has
    // its own thread.
    if let Err(e) = stream.set_nonblocking(false) {
        log::warn!("mcp_bridge nonblocking off: {e}");
        return;
    }
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("mcp_bridge clone: {e}");
            return;
        }
    };
    let reader = BufReader::new(read_stream);
    let mut writer = stream;
    for line in reader.lines() {
        if is_shutdown(generation) {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = dispatch_line(line);
        let text = match serde_json::to_string(&response) {
            Ok(t) => t,
            Err(e) => format!(
                "{{\"result\":\"err\",\"message\":\"serialize: {}\"}}",
                e
            ),
        };
        if writeln!(writer, "{text}").is_err() {
            break;
        }
        if writer.flush().is_err() {
            break;
        }
    }
    let _ = writer.shutdown(Shutdown::Both);
}

fn dispatch_line(line: &str) -> BridgeResponse {
    let request: BridgeRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return BridgeResponse::err(format!("bad request: {e}")),
    };

    // Screenshots and other non-trivial ops are routed to the editor's
    // main thread. Simple ones that need no engine state go here.
    if let BridgeRequest::Ping = request {
        return BridgeResponse::data(serde_json::json!({"alive": true}));
    }

    let (id, rx) = queue(request);
    wait(id, rx, ANSWER_TIMEOUT)
}

/// Put `request` in the queue for the editor.
fn queue(request: BridgeRequest) -> (u64, mpsc::Receiver<BridgeResponse>) {
    let (tx, rx) = mpsc::channel();
    let id = state().next_id.fetch_add(1, Ordering::Relaxed);
    state().pending.lock().unwrap().push(PendingRequest {
        request,
        reply: tx,
        id,
    });
    (id, rx)
}

/// The editor's answer to the queued request `id`. One it has not taken
/// by `timeout` is taken out of the queue again, so it does not run after
/// the client was told it failed (and maybe tried again).
fn wait(id: u64, rx: mpsc::Receiver<BridgeResponse>, timeout: Duration) -> BridgeResponse {
    match rx.recv_timeout(timeout) {
        Ok(response) => response,
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            BridgeResponse::err("the editor stopped the bridge; the request was not run")
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let mut pending = state().pending.lock().unwrap();
            match pending.iter().position(|request| request.id == id) {
                Some(index) => {
                    pending.remove(index);
                    BridgeResponse::err("editor did not answer in time; the request was not run")
                }
                // Taken: the editor is at it, and its answer may still come.
                None => {
                    drop(pending);
                    rx.recv_timeout(timeout).unwrap_or_else(|_| {
                        BridgeResponse::err(
                            "editor did not answer in time; it started the request, which \
                             may still take effect",
                        )
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::sync::MutexGuard;

    // The server uses process-global state; run tests one at a time.
    fn test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let lock = LOCK.get_or_init(|| Mutex::new(()));
        match lock.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ferrite-bridge-{}-{}-{:?}",
            name,
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn reset() {
        stop();
        *state().pending.lock().unwrap() = Vec::new();
    }

    #[test]
    fn start_writes_a_lockfile_and_stop_removes_it() {
        let _g = test_lock();
        reset();
        let dir = tmp_dir("lockfile");
        let port = start(&dir).unwrap();
        let path = dir.join(LOCKFILE_NAME);
        assert!(path.is_file());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.trim().parse::<u16>().unwrap(), port);
        stop();
        assert!(!path.exists());
    }

    #[test]
    fn ping_answers_without_touching_the_queue() {
        let _g = test_lock();
        reset();
        let dir = tmp_dir("ping");
        let port = start(&dir).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(stream, r#"{{"op":"ping"}}"#).unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("\"result\":\"ok\""));
        assert!(line.contains("alive"));
        // The queue must be empty — ping never reached it.
        assert!(drain_pending().is_empty());
        stop();
    }

    #[test]
    fn other_requests_land_on_the_queue_and_wait_for_a_reply() {
        let _g = test_lock();
        reset();
        let dir = tmp_dir("queue");
        let port = start(&dir).unwrap();
        // A background thread acting as "the editor main loop".
        let joiner = std::thread::spawn(|| {
            // Give the client a moment to send.
            for _ in 0..20 {
                let pending = drain_pending();
                if let Some(req) = pending.into_iter().next() {
                    req.respond(BridgeResponse::data(serde_json::json!({"saw": true})));
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(stream, r#"{{"op":"status"}}"#).unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        joiner.join().unwrap();
        assert!(line.contains("\"saw\":true"));
        stop();
    }

    #[test]
    fn a_request_not_taken_in_time_is_not_run_later() {
        let _g = test_lock();
        reset();
        let (id, rx) = queue(BridgeRequest::Ping);
        let response = wait(id, rx, Duration::from_millis(20));
        let text = serde_json::to_string(&response).unwrap();
        assert!(text.contains("not run"), "{text}");
        assert!(drain_pending().is_empty());
    }

    #[test]
    fn stop_closes_open_connections_and_the_listener() {
        let _g = test_lock();
        reset();
        let dir = tmp_dir("stop");
        let port = start(&dir).unwrap();
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Until the connection is registered.
        for _ in 0..100 {
            if !state().connections.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        stop();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        // Closed by the bridge: end of stream, not a timeout.
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        // A new bridge right after; the old one's port is let go.
        let again = start(&dir).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        if again != port {
            assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
        }
        stop();
    }

    #[test]
    fn a_malformed_line_gets_an_err_response() {
        let _g = test_lock();
        reset();
        let dir = tmp_dir("malformed");
        let port = start(&dir).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        writeln!(stream, "not json").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("\"result\":\"err\""));
        stop();
    }
}
