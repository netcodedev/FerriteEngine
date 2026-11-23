//! What is left behind when a game goes wrong on a player's machine: a log
//! file per run, the machine's GPU and driver, and a crash report for a
//! panic or a GPU reset, so a tester can send us a folder instead of "it
//! went black".
//!
//! - `install` (from `Application`) opens the run's log file in the log
//!   folder (see `log_dir`), keeps the newest `KEEP_LOGS` runs and installs
//!   a panic hook that writes `crash-<time>.txt`.
//! - `context_created` (from `Window`) logs the GPU, the driver, the
//!   monitors and the free video memory, and turns on the GL debug output
//!   when asked for (`--gl-debug` or `FERRITE_GL_DEBUG=1`).
//! - `end_frame` (from `Application::start`) asks the driver once per frame
//!   whether the GPU was reset. The context is made with
//!   `LoseContextOnReset`, so after a reset (Windows' TDR, when the GPU
//!   hangs for about two seconds) the game writes a report and ends instead
//!   of crashing inside the driver. It also warns about frames that took
//!   longer than `SLOW_FRAME`, and logs the free video memory every
//!   `VRAM_EVERY`.
//! - Breadcrumbs (`mark_pass`, frame number, recent frame times) say where
//!   the renderer was when it went wrong.

use std::{
    backtrace::Backtrace,
    collections::VecDeque,
    ffi::CStr,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use log::Level;

use super::logging;

/// Log files kept per game (the newest; older ones are deleted).
pub const KEEP_LOGS: usize = 10;
/// Crash reports kept per game.
pub const KEEP_REPORTS: usize = 20;
/// A frame this long is logged as a warning: the GPU or the CPU stalled.
pub const SLOW_FRAME: Duration = Duration::from_millis(500);
/// Slow frames are warned about at most this often (the others counted).
const SLOW_FRAME_WARN_EVERY: Duration = Duration::from_secs(5);
/// How often the free video memory is logged.
pub const VRAM_EVERY: Duration = Duration::from_secs(60);
/// Frames at the start not warned about (shaders compile, chunks load).
const WARM_UP_FRAMES: u64 = 30;
/// Frame times kept for the breadcrumbs.
const FRAME_TIMES: usize = 120;
/// Log messages a crash report ends with.
const REPORT_LOG_LINES: usize = 200;

struct State {
    app: String,
    dir: PathBuf,
    log_file: Option<PathBuf>,
    /// The machine, the GPU and the driver, once the context is made.
    system: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The render pass last started, + 1 (0: none yet); see `mark_pass`.
static PASS: AtomicUsize = AtomicUsize::new(0);
static PASS_NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static FRAME: AtomicU64 = AtomicU64::new(0);
/// Whether the context can tell about a GPU reset.
static ROBUST: AtomicBool = AtomicBool::new(false);
/// A GPU reset was seen and handled (handled once).
static RESET: AtomicBool = AtomicBool::new(false);

struct Frames {
    last: Option<Instant>,
    times_ms: VecDeque<f32>,
    last_vram: Option<Instant>,
    vram: Option<String>,
    last_slow_warning: Option<Instant>,
    slow_unreported: u32,
}

static FRAMES: Mutex<Frames> = Mutex::new(Frames {
    last: None,
    times_ms: VecDeque::new(),
    last_vram: None,
    vram: None,
    last_slow_warning: None,
    slow_unreported: 0,
});

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Open the run's log file for the game `app` and install the panic hook.
/// Only the first call counts.
pub fn install(app: &str) {
    if lock(&STATE).is_some() {
        return;
    }
    let dir = log_dir(app);
    let stamp = file_stamp(SystemTime::now());
    let log_file = fs::create_dir_all(&dir).ok().and_then(|_| {
        prune(&dir, "", ".log", KEEP_LOGS.saturating_sub(1));
        let path = dir.join(format!("{stamp}_{}.log", std::process::id()));
        let file = fs::File::create(&path).ok()?;
        logging::set_file(Some(file));
        Some(path)
    });
    logging::write_raw(&format!(
        "{app} — Ferrite {} — started {} UTC\n{}",
        env!("CARGO_PKG_VERSION"),
        readable_stamp(SystemTime::now()),
        machine_summary(),
    ));
    match &log_file {
        Some(path) => log::info!("log file: {}", path.display()),
        None => log::warn!("could not open a log file in {}", dir.display()),
    }
    *lock(&STATE) = Some(State {
        app: app.to_owned(),
        dir,
        log_file,
        system: machine_summary(),
    });
    install_panic_hook();
    super::memory::heap::tune_malloc();
}

/// Where `app`'s logs and crash reports go: `FERRITE_LOG_DIR` if set, else
/// `%LOCALAPPDATA%\<app>\logs` on Windows, `~/Library/Logs/<app>` on macOS,
/// `~/.local/share/<app>/logs` elsewhere; `logs` next to the executable
/// when none of those is known.
pub fn log_dir(app: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("FERRITE_LOG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let app = folder_name(app);
    let env = |key: &str| {
        std::env::var_os(key)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").map(|d| d.join(&app).join("logs"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|d| d.join("Library/Logs").join(&app))
    } else {
        env("XDG_DATA_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".local/share")))
            .map(|d| d.join(&app).join("logs"))
    };
    base.or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("logs")))
    })
    .unwrap_or_else(|| std::env::temp_dir().join(app).join("logs"))
}

/// This run's log folder, once `install` ran.
pub fn folder() -> Option<PathBuf> {
    lock(&STATE).as_ref().map(|s| s.dir.clone())
}

/// `name` as a folder name every system accepts.
fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_owned();
    if cleaned.is_empty() {
        "Ferrite".to_owned()
    } else {
        cleaned
    }
}

/// Delete the oldest files in `dir` named `<prefix>…<suffix>` until
/// `keep` are left. The names start with a time stamp, so they sort by age.
fn prune(dir: &Path, prefix: &str, suffix: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        // Crash reports are not logs, whatever they end with.
        .filter(|n| !prefix.is_empty() || !n.starts_with("crash-"))
        .collect();
    names.sort();
    let excess = names.len().saturating_sub(keep);
    for name in &names[..excess] {
        let _ = fs::remove_file(dir.join(name));
    }
}

/// UTC date and time of `t` as (year, month, day, hour, minute, second).
fn utc(t: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        month,
        day,
        (rest / 3600) as u32,
        (rest / 60 % 60) as u32,
        (rest % 60) as u32,
    )
}

/// `2026-11-19_21-47-36`: sorts by time and is a valid file name.
fn file_stamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = utc(t);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

/// `2026-11-19 21:47:36`.
pub fn readable_stamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = utc(t);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

fn machine_summary() -> String {
    let cpus = std::thread::available_parallelism().map_or(0, |n| n.get());
    let args: Vec<String> = std::env::args().skip(1).collect();
    format!(
        "OS: {} {} ({cpus} CPU threads)\nArguments: {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if args.is_empty() {
            "-".to_owned()
        } else {
            args.join(" ")
        },
    )
}

/// Whether the GL debug output was asked for: `--gl-debug` on the command
/// line or `FERRITE_GL_DEBUG=1`. It makes the driver check every call and
/// costs frame time, so it is off by default.
pub fn gl_debug_requested() -> bool {
    std::env::args().any(|a| a == "--gl-debug")
        || std::env::var("FERRITE_GL_DEBUG").is_ok_and(|v| v == "1" || v == "true")
}

/// Log the GPU, the driver, the context and the monitors; call with the new
/// context current. `debug`: turn the GL debug output on.
pub fn context_created(glfw: &mut glfw::Glfw, debug: bool) {
    let mut text = String::new();
    let string = |name| unsafe {
        let p = gl::GetString(name);
        if p.is_null() {
            "?".to_owned()
        } else {
            CStr::from_ptr(p as *const _).to_string_lossy().into_owned()
        }
    };
    let integer = |name| {
        let mut v = 0;
        unsafe { gl::GetIntegerv(name, &mut v) };
        v
    };
    text += &format!("GPU: {} ({})\n", string(gl::RENDERER), string(gl::VENDOR));
    text += &format!(
        "OpenGL: {}, GLSL {}\n",
        string(gl::VERSION),
        string(gl::SHADING_LANGUAGE_VERSION)
    );
    let strategy = integer(gl::RESET_NOTIFICATION_STRATEGY) as u32;
    let robust = gl::GetGraphicsResetStatus::is_loaded() && strategy == gl::LOSE_CONTEXT_ON_RESET;
    ROBUST.store(robust, Ordering::Relaxed);
    let flags = integer(gl::CONTEXT_FLAGS) as u32;
    text += &format!(
        "Context: robust {}, debug {}, MSAA {}x, max texture {}\n",
        if robust {
            "yes"
        } else {
            "no (GPU resets go unnoticed)"
        },
        if flags & gl::CONTEXT_FLAG_DEBUG_BIT != 0 {
            "yes"
        } else {
            "no"
        },
        integer(gl::SAMPLES),
        integer(gl::MAX_TEXTURE_SIZE),
    );
    text += &monitors_text(glfw);
    if let Some(vram) = video_memory() {
        text += &format!("Video memory: {vram}\n");
        lock(&FRAMES).vram = Some(vram);
    }
    for line in text.lines() {
        log::info!("{line}");
    }
    if let Some(state) = lock(&STATE).as_mut() {
        state.system = format!("{}{text}", machine_summary());
    }
    if debug {
        enable_debug_output();
    }
}

fn monitors_text(glfw: &mut glfw::Glfw) -> String {
    let monitors = glfw.with_connected_monitors(|_, monitors| {
        monitors
            .iter()
            .map(|m| {
                let name = m.get_name().unwrap_or_else(|| "?".into());
                match m.get_video_mode() {
                    Some(v) => format!("{name} {}x{} @ {} Hz", v.width, v.height, v.refresh_rate),
                    None => name,
                }
            })
            .collect::<Vec<_>>()
    });
    format!(
        "Monitors: {}\n",
        if monitors.is_empty() {
            "-".into()
        } else {
            monitors.join("; ")
        }
    )
}

/// `context_created` for a window drawn with Vulkan (no GL context):
/// `description` is the device's (`gfx::Device::describe`).
pub fn vulkan_device_created(glfw: &mut glfw::Glfw, description: &str) {
    let mut text = format!("GPU: {description}\nRenderer: Vulkan\n");
    text += &monitors_text(glfw);
    if let Some(vram) = video_memory() {
        text += &format!("Video memory: {vram}\n");
        lock(&FRAMES).vram = Some(vram);
    }
    for line in text.lines() {
        log::info!("{line}");
    }
    if let Some(state) = lock(&STATE).as_mut() {
        state.system = format!("{}{text}", machine_summary());
    }
}

fn has_extension(name: &str) -> bool {
    let mut count = 0;
    unsafe { gl::GetIntegerv(gl::NUM_EXTENSIONS, &mut count) };
    (0..count.max(0) as u32).any(|i| unsafe {
        let p = gl::GetStringi(gl::EXTENSIONS, i);
        !p.is_null() && CStr::from_ptr(p as *const _).to_bytes() == name.as_bytes()
    })
}

/// Free (and on NVIDIA total) video memory, where the driver tells.
fn video_memory() -> Option<String> {
    use crate::core::gfx;
    if gfx::backend() != gfx::BackendKind::OpenGl {
        // No GL to ask: the device's own (Vulkan's memory budget).
        return gfx::device().video_memory().filter(|(_, total)| *total > 0).map(|(used, total)| {
            format!("{} MB free of {} MB", total.saturating_sub(used) >> 20, total >> 20)
        });
    }
    // GL_NVX_gpu_memory_info and GL_ATI_meminfo, in KiB.
    const NVX_TOTAL: u32 = 0x9048;
    const NVX_AVAILABLE: u32 = 0x9049;
    const ATI_TEXTURE_FREE: u32 = 0x87FC;
    if has_extension("GL_NVX_gpu_memory_info") {
        let (mut total, mut free) = (0, 0);
        unsafe {
            gl::GetIntegerv(NVX_TOTAL, &mut total);
            gl::GetIntegerv(NVX_AVAILABLE, &mut free);
        }
        // Software drivers report nonsense here.
        return (total > 0 && (0..=total).contains(&free))
            .then(|| format!("{} MB free of {} MB", free / 1024, total / 1024));
    }
    if has_extension("GL_ATI_meminfo") {
        let mut info = [0; 4];
        unsafe { gl::GetIntegerv(ATI_TEXTURE_FREE, info.as_mut_ptr()) };
        return (info[0] > 0).then(|| format!("{} MB free", info[0] / 1024));
    }
    None
}

fn enable_debug_output() {
    if !gl::DebugMessageCallback::is_loaded() {
        log::warn!("GL debug output asked for, but the driver has none");
        return;
    }
    unsafe {
        gl::Enable(gl::DEBUG_OUTPUT);
        // Messages arrive in the call that caused them, so the log shows
        // them in order with the engine's own.
        gl::Enable(gl::DEBUG_OUTPUT_SYNCHRONOUS);
        gl::DebugMessageCallback(Some(on_gl_message), std::ptr::null());
    }
    crate::core::renderer::profiler::set_enabled(true);
    log::info!("GL debug output on (and per-pass GPU timing)");
}

extern "system" fn on_gl_message(
    _source: gl::types::GLenum,
    kind: gl::types::GLenum,
    id: gl::types::GLuint,
    severity: gl::types::GLenum,
    length: gl::types::GLsizei,
    message: *const gl::types::GLchar,
    _user: *mut std::ffi::c_void,
) {
    if severity == gl::DEBUG_SEVERITY_NOTIFICATION || message.is_null() {
        return;
    }
    let text = unsafe {
        let bytes = std::slice::from_raw_parts(message as *const u8, length.max(0) as usize);
        String::from_utf8_lossy(bytes).into_owned()
    };
    let level = match (severity, kind) {
        (gl::DEBUG_SEVERITY_HIGH, _) | (_, gl::DEBUG_TYPE_ERROR) => Level::Error,
        (gl::DEBUG_SEVERITY_MEDIUM, _) => Level::Warn,
        _ => Level::Debug,
    };
    log::log!(target: "ferrite::gl", level, "{text} (id {id})");
}

/// The renderer starts `pass` (a breadcrumb for crash reports).
pub fn mark_pass(pass: &'static str) {
    let mut names = lock(&PASS_NAMES);
    let index = match names.iter().position(|n| *n == pass) {
        Some(i) => i,
        None => {
            names.push(pass);
            names.len() - 1
        }
    };
    PASS.store(index + 1, Ordering::Relaxed);
}

fn last_pass() -> &'static str {
    match PASS.load(Ordering::Relaxed) {
        0 => "-",
        i => lock(&PASS_NAMES).get(i - 1).copied().unwrap_or("-"),
    }
}

/// A frame was presented: check for a GPU reset, note the frame's time,
/// warn about a slow one and now and then log the free video memory.
pub fn end_frame() {
    let frame = FRAME.fetch_add(1, Ordering::Relaxed) + 1;
    check_gpu_reset();
    let now = Instant::now();
    let mut frames = lock(&FRAMES);
    if let Some(last) = frames.last.replace(now) {
        let took = now - last;
        if frames.times_ms.len() == FRAME_TIMES {
            frames.times_ms.pop_front();
        }
        frames.times_ms.push_back(took.as_secs_f32() * 1000.0);
        if took >= SLOW_FRAME && frame > WARM_UP_FRAMES {
            if frames
                .last_slow_warning
                .is_none_or(|t| now - t >= SLOW_FRAME_WARN_EVERY)
            {
                let more = match std::mem::take(&mut frames.slow_unreported) {
                    0 => String::new(),
                    n => format!(", {n} more slow frames since the last warning"),
                };
                frames.last_slow_warning = Some(now);
                log::warn!(
                    "frame {frame} took {} ms (last pass: {}{more}){}",
                    took.as_millis(),
                    last_pass(),
                    gpu_pass_summary()
                );
            } else {
                frames.slow_unreported += 1;
            }
        }
    }
    if frames.last_vram.is_none_or(|t| now - t >= VRAM_EVERY) {
        frames.last_vram = Some(now);
        drop(frames);
        if let Some(vram) = video_memory() {
            if frame > 1 {
                log::info!("video memory: {vram}");
            }
            lock(&FRAMES).vram = Some(vram);
        }
    }
}

/// `; GPU ms per pass: …` while the profiler runs, else nothing.
fn gpu_pass_summary() -> String {
    use crate::core::renderer::profiler::{pass_ms, Pass};
    match pass_ms() {
        Some(ms) => {
            let parts: Vec<String> = Pass::ALL
                .iter()
                .zip(ms)
                .filter(|(_, ms)| *ms >= 0.05)
                .map(|(p, ms)| format!("{} {ms:.1}", p.label()))
                .collect();
            format!("; GPU ms per pass: {}", parts.join(", "))
        }
        None => String::new(),
    }
}

fn check_gpu_reset() {
    if !ROBUST.load(Ordering::Relaxed) || RESET.load(Ordering::Relaxed) {
        return;
    }
    let status = unsafe { gl::GetGraphicsResetStatus() };
    if status == gl::NO_ERROR {
        return;
    }
    RESET.store(true, Ordering::Relaxed);
    let cause = match status {
        gl::GUILTY_CONTEXT_RESET => "caused by this game",
        gl::INNOCENT_CONTEXT_RESET => "caused by another program",
        _ => "cause unknown",
    };
    let details = format!(
        "The graphics driver was reset ({cause}). On Windows this is usually a \
         TDR: the GPU did not finish its work within about two seconds.\n"
    );
    log::error!(
        "GPU reset ({cause}) in frame {}",
        FRAME.load(Ordering::Relaxed)
    );
    let report = write_report("GPU reset", &details);
    show_error(
        "Grafiktreiber zurückgesetzt",
        &format!(
            "Der Grafiktreiber wurde zurückgesetzt, das Spiel wird beendet.{}",
            report
                .map(|p| format!("\n\nBericht: {}", p.display()))
                .unwrap_or_default()
        ),
    );
    log::logger().flush();
    // Every GL call would fail from here on: end before one crashes.
    std::process::exit(3);
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let location = info
            .location()
            .map_or_else(|| "?".into(), |l| format!("{}:{}", l.file(), l.line()));
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed").to_owned();
        log::error!("panic in thread '{thread}' at {location}: {message}");
        let details = format!(
            "Thread '{thread}' panicked at {location}:\n{message}\n\nBacktrace:\n{}\n",
            Backtrace::force_capture()
        );
        let report = write_report("panic", &details);
        log::logger().flush();
        // Worker threads' panics need not end the game: no dialog for them.
        if thread == "main" {
            show_error(
                "Absturz",
                &format!(
                    "Das Spiel ist abgestürzt:\n{message}{}",
                    report
                        .map(|p| format!("\n\nBericht: {}", p.display()))
                        .unwrap_or_default()
                ),
            );
        }
        previous(info);
    }));
}

/// Write `crash-<time>.txt` to the log folder: what happened, the
/// breadcrumbs, the machine and the last log messages. Returns its path.
pub fn write_report(kind: &str, details: &str) -> Option<PathBuf> {
    let (dir, app, log_file, system) = {
        let state = lock(&STATE);
        let state = state.as_ref()?;
        (
            state.dir.clone(),
            state.app.clone(),
            state.log_file.clone(),
            state.system.clone(),
        )
    };
    let text = report_text(kind, details, &app, log_file.as_deref(), &system);
    let _ = fs::create_dir_all(&dir);
    prune(&dir, "crash-", ".txt", KEEP_REPORTS.saturating_sub(1));
    // Numbered: two reports can come in the same second (a worker's
    // panic, then the main thread's).
    static REPORTS: AtomicU64 = AtomicU64::new(0);
    let path = dir.join(format!(
        "crash-{}_{}.txt",
        file_stamp(SystemTime::now()),
        REPORTS.fetch_add(1, Ordering::Relaxed) + 1
    ));
    match fs::write(&path, text) {
        Ok(()) => {
            log::error!("crash report: {}", path.display());
            Some(path)
        }
        Err(e) => {
            log::error!("could not write a crash report to {}: {e}", path.display());
            None
        }
    }
}

fn report_text(
    kind: &str,
    details: &str,
    app: &str,
    log_file: Option<&Path>,
    system: &str,
) -> String {
    let (times, vram) = {
        let frames = lock(&FRAMES);
        (
            frames.times_ms.iter().copied().collect::<Vec<f32>>(),
            frames.vram.clone(),
        )
    };
    let mut text = format!(
        "{app} crash report ({kind})\nFerrite {}, {} UTC\n\n{details}\n",
        env!("CARGO_PKG_VERSION"),
        readable_stamp(SystemTime::now()),
    );
    text += &format!(
        "Breadcrumbs\nFrame: {}\nLast render pass started: {}\n",
        FRAME.load(Ordering::Relaxed),
        last_pass()
    );
    if !times.is_empty() {
        let max = times.iter().copied().fold(0.0, f32::max);
        let average = times.iter().sum::<f32>() / times.len() as f32;
        let last: Vec<String> = times
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(|t| format!("{t:.1}"))
            .collect();
        text += &format!(
            "Frame times (last {}): average {average:.1} ms, max {max:.1} ms; newest: {}\n",
            times.len(),
            last.join(", ")
        );
    }
    if let Some(vram) = vram {
        text += &format!("Video memory (last sample): {vram}\n");
    }
    text += &format!("\nSystem\n{system}\n");
    if let Some(log_file) = log_file {
        text += &format!("Full log: {}\n", log_file.display());
    }
    text += &format!("\nLast {REPORT_LOG_LINES} log messages\n");
    for entry in logging::recent(REPORT_LOG_LINES, Level::Trace) {
        text += &logging::format_line(&entry);
        text.push('\n');
    }
    text
}

/// An error dialog on Windows (the game has no console there); stderr
/// elsewhere.
fn show_error(title: &str, text: &str) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                hwnd: *mut std::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                kind: u32,
            ) -> i32;
        }
        const MB_ICONERROR: u32 = 0x10;
        const MB_SETFOREGROUND: u32 = 0x1_0000;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (text, title) = (wide(text), wide(title));
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_ICONERROR | MB_SETFOREGROUND,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("{title}: {text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates_come_out_right() {
        let at = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(utc(at(0)), (1970, 1, 1, 0, 0, 0));
        // 2026-11-19 21:47:36 UTC.
        assert_eq!(file_stamp(at(1_795_124_856)), "2026-11-19_21-47-36");
        // A leap day.
        assert_eq!(readable_stamp(at(1_709_208_000)), "2024-02-29 12:00:00");
    }

    #[test]
    fn game_names_become_folder_names() {
        assert_eq!(folder_name("My Game"), "My Game");
        assert_eq!(folder_name("A/B: C?"), "A_B_ C_");
        assert_eq!(folder_name("  ..."), "Ferrite");
        assert_eq!(folder_name("Demo."), "Demo");
    }

    #[test]
    fn the_log_folder_can_be_set() {
        // Only this test sets the variable.
        std::env::set_var("FERRITE_LOG_DIR", "/tmp/ferrite-logs-test");
        assert_eq!(log_dir("Game"), PathBuf::from("/tmp/ferrite-logs-test"));
        std::env::remove_var("FERRITE_LOG_DIR");
        assert!(log_dir("Game").to_string_lossy().contains("Game"));
    }

    #[test]
    fn pruning_keeps_the_newest_logs_and_leaves_reports_alone() {
        let dir = std::env::temp_dir().join(format!("ferrite-prune-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "2026-01-01_00-00-00_1.log",
            "2026-01-02_00-00-00_1.log",
            "2026-01-03_00-00-00_1.log",
            "crash-2026-01-01_00-00-00.txt",
            "crash-2026-01-02_00-00-00.txt",
            "notes.txt",
        ] {
            fs::write(dir.join(name), "").unwrap();
        }
        prune(&dir, "", ".log", 2);
        prune(&dir, "crash-", ".txt", 1);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "2026-01-02_00-00-00_1.log",
                "2026-01-03_00-00-00_1.log",
                "crash-2026-01-02_00-00-00.txt",
                "notes.txt",
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_carry_details_breadcrumbs_and_system() {
        mark_pass("Sky + clouds");
        let text = report_text("GPU reset", "it hung\n", "Demo", None, "GPU: Test\n");
        assert!(text.starts_with("Demo crash report (GPU reset)"));
        assert!(text.contains("it hung"));
        assert!(text.contains("Last render pass started: Sky + clouds"));
        assert!(text.contains("GPU: Test"));
    }
}
