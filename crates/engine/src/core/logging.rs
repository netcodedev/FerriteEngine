//! The engine's logger. Everything logged with the `log` macros (the engine,
//! the editor and the game's scripts, which share this library) is written
//! to stderr as before (`env_logger`, filtered by `RUST_LOG`) and kept in a
//! short in-memory history, which the editor's console shows.
//!
//! The history keeps debug messages and above from Ferrite's own crates and
//! the scripts, and info and above from everything else, whatever `RUST_LOG`
//! says: the console has its own level filter.
//!
//! Once `set_file` is given one, the same messages also go to a log file
//! (see `diagnostics`), so a player's run can be read after the fact.

use std::{
    collections::VecDeque,
    fs::File,
    io::{LineWriter, Write},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// How many messages the history keeps; older ones are dropped.
pub const HISTORY: usize = 5000;

/// Module path prefix of the game's scripts (see `crates/scripts`).
const SCRIPTS_CRATE: &str = "ferrite_scripts";

/// Where a message comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogSource {
    /// The engine, the editor or a library they use.
    Engine,
    /// One of the game's scripts.
    Script,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    /// Counts up across the whole run; never reused, not even after
    /// `clear`.
    pub seq: u64,
    /// Since the logger was installed.
    pub time: Duration,
    pub level: Level,
    /// Module path the message was logged from (`log`'s target).
    pub target: String,
    pub message: String,
}

impl LogEntry {
    pub fn source(&self) -> LogSource {
        source_of(&self.target)
    }

    /// The target without the crate a reader already knows from the
    /// source: `player` for `ferrite_scripts::player`, `terrain::stream`
    /// for `ferrite::terrain::stream`.
    pub fn short_target(&self) -> &str {
        short_target(&self.target)
    }
}

pub fn source_of(target: &str) -> LogSource {
    if crate_of(target) == SCRIPTS_CRATE {
        LogSource::Script
    } else {
        LogSource::Engine
    }
}

fn crate_of(target: &str) -> &str {
    target.split("::").next().unwrap_or(target)
}

pub fn short_target(target: &str) -> &str {
    match target.split_once("::") {
        Some((krate, rest)) if krate.starts_with("ferrite") && !rest.is_empty() => rest,
        _ => target,
    }
}

/// Whether the history keeps a message of this level from this target.
fn kept(level: Level, target: &str) -> bool {
    if crate_of(target).starts_with("ferrite") {
        level <= Level::Debug
    } else {
        level <= Level::Info
    }
}

struct History {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
}

struct Logger {
    stderr: env_logger::Logger,
    started: Instant,
    history: Mutex<History>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Where kept messages are written as well, line by line (see `set_file`).
static FILE: Mutex<Option<LineWriter<File>>> = Mutex::new(None);

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        kept(metadata.level(), metadata.target()) || self.stderr.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if self.stderr.matches(record) {
            self.stderr.log(record);
        }
        if !kept(record.level(), record.target()) {
            return;
        }
        let entry = LogEntry {
            seq: 0,
            time: self.started.elapsed(),
            level: record.level(),
            target: record.target().to_owned(),
            message: record.args().to_string(),
        };
        write_to_file(&entry);
        let mut history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        push(&mut history, entry);
    }

    fn flush(&self) {
        self.stderr.flush();
        if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = file.flush();
        }
    }
}

fn push(history: &mut History, mut entry: LogEntry) {
    entry.seq = history.next_seq;
    history.next_seq += 1;
    if history.entries.len() == HISTORY {
        history.entries.pop_front();
    }
    history.entries.push_back(entry);
}

/// One message as a line of the log file:
/// `[   12.345s WARN  terrain::stream] message`.
pub fn format_line(entry: &LogEntry) -> String {
    format!(
        "[{:>10.3}s {:<5} {}] {}",
        entry.time.as_secs_f64(),
        entry.level,
        entry.short_target(),
        entry.message
    )
}

fn write_to_file(entry: &LogEntry) {
    let mut file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(file) = file.as_mut() {
        // A line writer flushes every line: a hard crash right after a
        // message (a driver taking the process down) still leaves it in
        // the file.
        let _ = writeln!(file, "{}", format_line(entry));
    }
}

/// Write every kept message from now on to `file` as well (`None`: stop).
pub fn set_file(file: Option<File>) {
    *FILE.lock().unwrap_or_else(|e| e.into_inner()) = file.map(LineWriter::new);
}

/// Write `text` to the log file as it is, e.g. a block of system
/// information (nothing without a file).
pub fn write_raw(text: &str) {
    if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(file, "{text}");
        let _ = file.flush();
    }
}

/// Install the logger. `default_filter` is what stderr shows when
/// `RUST_LOG` is not set (`env_logger`'s syntax, e.g. "error" or "info").
/// Only the first call (or another logger installed before) counts.
pub fn init(default_filter: &str) {
    let stderr = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(default_filter),
    )
    .build();
    let max = stderr.filter().max(LevelFilter::Debug);
    let logger = LOGGER.get_or_init(|| Logger {
        stderr,
        started: Instant::now(),
        history: Mutex::new(History {
            entries: VecDeque::new(),
            next_seq: 0,
        }),
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(max);
    }
}

/// Call `f` with every kept message newer than `seq` (all with 0), oldest
/// first, and return the sequence number to pass next time.
pub fn read_since(seq: u64, mut f: impl FnMut(&LogEntry)) -> u64 {
    let Some(logger) = LOGGER.get() else {
        return seq;
    };
    let history = logger.history.lock().unwrap_or_else(|e| e.into_inner());
    // Sequence numbers are contiguous, so the first new one is found by
    // arithmetic instead of a search.
    let first = history.entries.front().map_or(history.next_seq, |e| e.seq);
    let skip = seq.saturating_sub(first) as usize;
    for entry in history.entries.iter().skip(skip) {
        f(entry);
    }
    history.next_seq
}

/// The newest `limit` kept messages at `level` or more severe, oldest
/// first (for tools reading the log, e.g. over the MCP bridge).
pub fn recent(limit: usize, level: Level) -> Vec<LogEntry> {
    let Some(logger) = LOGGER.get() else {
        return Vec::new();
    };
    let history = logger.history.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries: Vec<LogEntry> = history
        .entries
        .iter()
        .rev()
        .filter(|e| e.level <= level)
        .take(limit)
        .cloned()
        .collect();
    entries.reverse();
    entries
}

/// Forget the kept messages (sequence numbers keep counting).
pub fn clear() {
    if let Some(logger) = LOGGER.get() {
        logger
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: Level, target: &str, message: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            time: Duration::ZERO,
            level,
            target: target.into(),
            message: message.into(),
        }
    }

    #[test]
    fn scripts_are_told_apart_from_the_engine() {
        assert_eq!(source_of("ferrite_scripts::player"), LogSource::Script);
        assert_eq!(source_of("ferrite::terrain::stream"), LogSource::Engine);
        assert_eq!(source_of("ferrite_editor::editor_layer"), LogSource::Engine);
        assert_eq!(source_of("renet"), LogSource::Engine);
        assert_eq!(short_target("ferrite_scripts::player"), "player");
        assert_eq!(short_target("ferrite::terrain::stream"), "terrain::stream");
        assert_eq!(short_target("renet::server"), "renet::server");
        assert_eq!(short_target("ferrite"), "ferrite");
    }

    #[test]
    fn libraries_need_info_ferrite_keeps_debug() {
        assert!(kept(Level::Debug, "ferrite::terrain"));
        assert!(kept(Level::Debug, "ferrite_scripts::player"));
        assert!(!kept(Level::Trace, "ferrite_scripts::player"));
        assert!(kept(Level::Info, "renet"));
        assert!(!kept(Level::Debug, "renet"));
    }

    #[test]
    fn file_lines_carry_time_level_and_short_target() {
        let mut e = entry(Level::Warn, "ferrite::terrain::stream", "slow chunk");
        e.time = Duration::from_millis(12_345);
        assert_eq!(format_line(&e), "[    12.345s WARN  terrain::stream] slow chunk");
    }

    #[test]
    fn history_drops_the_oldest_and_counts_on() {
        let mut history = History {
            entries: VecDeque::new(),
            next_seq: 0,
        };
        for i in 0..HISTORY + 3 {
            push(&mut history, entry(Level::Info, "ferrite", &i.to_string()));
        }
        assert_eq!(history.entries.len(), HISTORY);
        assert_eq!(history.entries.front().unwrap().seq, 3);
        assert_eq!(history.entries.front().unwrap().message, "3");
        assert_eq!(history.next_seq, HISTORY as u64 + 3);
    }
}
