//! Development aid: with `FERRITE_SPIKES` set, a frame slower than 25 ms
//! is reported on stderr with what it spent its time on (scene update,
//! terrain streaming, scene render, generated models, uploaded data).
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
pub static UPDATE_US: AtomicU64 = AtomicU64::new(0);
pub static RENDER_US: AtomicU64 = AtomicU64::new(0);
pub static BUILD_US: AtomicU64 = AtomicU64::new(0);
pub static BUILDS: AtomicU64 = AtomicU64::new(0);
pub static STREAM_US: AtomicU64 = AtomicU64::new(0);
/// Bytes the Vulkan backend copied into staging memory for uploads.
pub static STAGED_BYTES: AtomicU64 = AtomicU64::new(0);
pub fn add(counter: &AtomicU64, since: std::time::Instant) {
    counter.fetch_add(since.elapsed().as_micros() as u64, Relaxed);
}
pub fn end_frame(frame: std::time::Duration) {
    let take = |c: &AtomicU64| c.swap(0, Relaxed) as f64 / 1000.0;
    let (u, r, b, n, s) = (take(&UPDATE_US), take(&RENDER_US), take(&BUILD_US), BUILDS.swap(0, Relaxed), take(&STREAM_US));
    let ms = frame.as_secs_f64() * 1000.0;
    let staged = STAGED_BYTES.swap(0, Relaxed) >> 20;
    if ms > 25.0 && std::env::var("FERRITE_SPIKES").is_ok() {
        eprintln!("[spike] frame {ms:.1} ms: scene update {u:.1} (terrain stream {s:.1}), scene render {r:.1}, generated models {n} in {b:.1}, uploads staged {staged} MB");
    }
}
