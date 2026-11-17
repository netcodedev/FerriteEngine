//! Docked console below the viewport: what the engine, the editor and the
//! game's scripts log (`log::info!`, `warn!`, ... ; see
//! `ferrite::core::logging`), coloured by level, with level and source
//! filters and a text search.

use std::time::Duration;

use ferrite::{
    core::{
        logging::{self, LogEntry, LogSource},
        primitives::{Offset, Position, Region, Size},
        renderer::{
            plane::{Plane, PlaneBuilder},
            text::{icons, Text},
        },
        scene::Scene,
        utils::DataSource,
    },
    log::Level,
    serde::{Deserialize, Serialize},
};
use ferrite_ui::ui::{
    element_handle::UIElementHandle,
    elements::{
        button::{Button, ButtonStyle},
        input::Input,
        panel::Panel,
    },
    theme::{self, Color},
    UIElement,
};
use glfw::{Action, Glfw, Key, Modifiers, MouseButton, WindowEvent};

use super::Shared;

const ACTIONS_HEIGHT: f32 = 30.0;
const BUTTON_HEIGHT: f32 = 24.0;
const ROW_HEIGHT: f32 = 20.0;
const FONT_SIZE: f32 = 12.5;
const SEARCH_WIDTH: f32 = 180.0;
/// Characters of the source column (`player`, `terrain::stream`); fewer
/// when the console is narrow.
const SOURCE_CHARS: usize = 18;
const SOURCE_CHARS_NARROW: usize = 12;
const ELLIPSIS: char = '\u{2026}';

/// What the console shows, kept in the project file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "ferrite::serde", default)]
pub struct ConsoleFilter {
    pub errors: bool,
    pub warnings: bool,
    pub infos: bool,
    /// Debug and trace.
    pub debug: bool,
    pub engine: bool,
    pub scripts: bool,
}

impl Default for ConsoleFilter {
    fn default() -> Self {
        Self {
            errors: true,
            warnings: true,
            infos: true,
            debug: true,
            engine: true,
            scripts: true,
        }
    }
}

impl ConsoleFilter {
    fn level(&self, level: Level) -> bool {
        match level {
            Level::Error => self.errors,
            Level::Warn => self.warnings,
            Level::Info => self.infos,
            Level::Debug | Level::Trace => self.debug,
        }
    }

    fn level_mut(&mut self, level: Level) -> &mut bool {
        match level {
            Level::Error => &mut self.errors,
            Level::Warn => &mut self.warnings,
            Level::Info => &mut self.infos,
            Level::Debug | Level::Trace => &mut self.debug,
        }
    }

    /// Whether `entry` is shown; `search` is lowercase, empty for all.
    pub fn shows(&self, entry: &LogEntry, search: &str) -> bool {
        let source = match entry.source() {
            LogSource::Engine => self.engine,
            LogSource::Script => self.scripts,
        };
        source
            && self.level(entry.level)
            && (search.is_empty()
                || entry.message.to_lowercase().contains(search)
                || entry.target.to_lowercase().contains(search))
    }
}

/// The level filter buttons, most severe first (debug stands for trace as
/// well).
const LEVELS: [Level; 4] = [Level::Error, Level::Warn, Level::Info, Level::Debug];

fn level_icon(level: Level) -> &'static str {
    match level {
        Level::Error => icons::CIRCLE_X,
        Level::Warn => icons::ALERT,
        Level::Info => icons::INFO,
        Level::Debug | Level::Trace => icons::BUG,
    }
}

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "errors",
        Level::Warn => "warnings",
        Level::Info => "info messages",
        Level::Debug | Level::Trace => "debug messages",
    }
}

/// Colour of a level's icon.
fn icon_color(level: Level) -> Color {
    match level {
        Level::Error => theme::DANGER,
        Level::Warn => theme::WARNING,
        Level::Info => theme::ACCENT_TEXT,
        Level::Debug | Level::Trace => theme::TEXT_DIM,
    }
}

/// Colour of a message's text.
fn text_color(level: Level) -> Color {
    match level {
        Level::Error => (1.0, 0.56, 0.56, 1.0),
        Level::Warn => (0.96, 0.78, 0.42, 1.0),
        Level::Info => theme::TEXT,
        Level::Debug => theme::TEXT_MUTED,
        Level::Trace => theme::TEXT_DIM,
    }
}

/// Tint behind a row of this level.
fn row_tint(level: Level) -> Color {
    match level {
        Level::Error => (0.88, 0.28, 0.28, 0.10),
        Level::Warn => (0.91, 0.67, 0.24, 0.07),
        _ => theme::TRANSPARENT,
    }
}

/// `mm:ss.mmm` since the editor started (`h:mm:ss.mmm` after an hour).
fn format_time(time: Duration) -> String {
    let ms = time.as_millis();
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m:02}:{s:02}.{ms:03}")
    }
}

fn level_label(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARN",
        Level::Info => "INFO",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    }
}

/// One message as plain text (for the clipboard).
fn entry_text(entry: &LogEntry) -> String {
    format!(
        "[{}] {} {}: {}",
        format_time(entry.time),
        level_label(entry.level),
        entry.target,
        entry.message
    )
}

/// `text` cut to `max` characters, with "…" when it was longer.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push(ELLIPSIS);
    out
}

fn count_label(n: usize) -> String {
    if n > 999 {
        "999+".into()
    } else {
        n.to_string()
    }
}

/// One line on screen: a message's first line, or a further line of the
/// selected (expanded) message.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Line {
    /// Index into `LogView::entries`.
    entry: usize,
    /// 0 for the first line.
    line: usize,
}

struct RowView {
    bg: Plane,
    icon: Text,
    time: Text,
    source: Text,
    message: Text,
}

/// The scrolling list of messages. Keeps its own copy of the logger's
/// history and draws only the rows on screen.
struct LogView {
    region: Region,
    frame: Plane,
    entries: Vec<LogEntry>,
    /// Sequence number of the next message to read from the logger.
    next_seq: u64,
    /// Messages per level (`LEVELS` order), whatever the filter.
    counts: [usize; 4],
    /// The filter and search the lines were built for.
    filter: ConsoleFilter,
    search: String,
    lines: Vec<Line>,
    /// Sequence number of the selected (expanded) message.
    selected: Option<u64>,
    hover: Option<usize>,
    scroll_y: f32,
    /// Stay at the newest message as messages come in.
    follow: bool,
    rows: Vec<RowView>,
    /// Width of one character of the monospace font.
    char_w: f32,
    empty: Text,
    z: f32,
    dirty: bool,
}

fn level_slot(level: Level) -> usize {
    match level {
        Level::Error => 0,
        Level::Warn => 1,
        Level::Info => 2,
        Level::Debug | Level::Trace => 3,
    }
}

impl LogView {
    fn new() -> Self {
        let probe = theme::mono("0", FONT_SIZE, theme::TEXT);
        Self {
            region: Region::new(Position::default(), Size::new(100.0, 100.0)),
            frame: PlaneBuilder::new()
                .color(theme::BG_FIELD)
                .border_color(theme::BORDER)
                .border_thickness(1.0)
                .border_radius_uniform(theme::RADIUS_SMALL)
                .build(),
            entries: Vec::new(),
            next_seq: 0,
            counts: [0; 4],
            filter: ConsoleFilter::default(),
            search: String::new(),
            lines: Vec::new(),
            selected: None,
            hover: None,
            scroll_y: 0.0,
            follow: true,
            rows: Vec::new(),
            char_w: probe.measure_str("0").max(1.0),
            empty: theme::text("", FONT_SIZE, theme::TEXT_DIM),
            z: 1.0,
            dirty: true,
        }
    }

    fn set_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let position = Position::new(x, y, self.z);
        let size = Size::new(w.max(10.0), h.max(10.0));
        if self.region.position != position || self.region.size != size {
            self.region = Region::new(position, size);
            self.frame.set_position(position);
            self.frame.set_size(size);
            self.dirty = true;
        }
    }

    /// Take the logger's new messages. True when anything changed.
    fn pull(&mut self) -> bool {
        let mut added = Vec::new();
        self.next_seq = logging::read_since(self.next_seq, |entry| added.push(entry.clone()));
        if added.is_empty() {
            return false;
        }
        for entry in &added {
            self.counts[level_slot(entry.level)] += 1;
        }
        let first_new = self.entries.len();
        self.entries.extend(added);
        let excess = self.entries.len().saturating_sub(logging::HISTORY);
        if excess > 0 {
            for entry in self.entries.drain(..excess) {
                self.counts[level_slot(entry.level)] -= 1;
            }
            self.rebuild_lines();
        } else {
            // Only new lines at the end: no need to filter everything again.
            let mut lines = std::mem::take(&mut self.lines);
            for index in first_new..self.entries.len() {
                self.push_lines(&mut lines, index);
            }
            self.lines = lines;
        }
        self.dirty = true;
        true
    }

    fn push_lines(&self, lines: &mut Vec<Line>, index: usize) {
        let entry = &self.entries[index];
        if !self.filter.shows(entry, &self.search) {
            return;
        }
        let count = if Some(entry.seq) == self.selected {
            entry.message.lines().count().max(1)
        } else {
            1
        };
        lines.extend((0..count).map(|line| Line { entry: index, line }));
    }

    fn rebuild_lines(&mut self) {
        let mut lines = Vec::with_capacity(self.entries.len());
        for index in 0..self.entries.len() {
            self.push_lines(&mut lines, index);
        }
        self.lines = lines;
        self.dirty = true;
    }

    fn set_filter(&mut self, filter: &ConsoleFilter, search: &str) {
        let search = search.trim().to_lowercase();
        if *filter != self.filter || search != self.search {
            self.filter = filter.clone();
            self.search = search;
            self.rebuild_lines();
        }
    }

    fn clear(&mut self) {
        logging::clear();
        self.entries.clear();
        self.lines.clear();
        self.counts = [0; 4];
        self.selected = None;
        self.hover = None;
        self.scroll_y = 0.0;
        self.follow = true;
        self.dirty = true;
    }

    fn max_scroll(&self) -> f32 {
        (self.lines.len() as f32 * ROW_HEIGHT - self.region.size.height + 4.0).max(0.0)
    }

    fn at_bottom(&self) -> bool {
        self.scroll_y >= self.max_scroll() - 1.0
    }

    fn scroll_to_bottom(&mut self) {
        self.follow = true;
        self.dirty = true;
    }

    fn line_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.region.contains(x, y) {
            return None;
        }
        let local = y - self.region.position.y - 2.0 + self.scroll_y;
        let index = (local / ROW_HEIGHT).floor();
        (index >= 0.0 && (index as usize) < self.lines.len()).then_some(index as usize)
    }

    fn selected_entry(&self) -> Option<&LogEntry> {
        let seq = self.selected?;
        self.entries.iter().find(|e| e.seq == seq)
    }

    fn toggle_selection(&mut self, line: usize) {
        let seq = self.entries[self.lines[line].entry].seq;
        self.selected = if self.selected == Some(seq) { None } else { Some(seq) };
        // Keep the clicked message where it is on screen.
        let entry = self.lines[line].entry;
        let top_before = self
            .lines
            .iter()
            .position(|l| l.entry == entry)
            .unwrap_or(line);
        let offset = top_before as f32 * ROW_HEIGHT - self.scroll_y;
        self.rebuild_lines();
        if let Some(top) = self.lines.iter().position(|l| l.entry == entry) {
            self.scroll_y = (top as f32 * ROW_HEIGHT - offset).clamp(0.0, self.max_scroll());
        }
        self.follow = self.at_bottom() && self.follow;
    }

    fn layout(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if self.follow {
            self.scroll_y = self.max_scroll();
        }
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());

        let top = self.region.position.y + 2.0;
        let height = self.region.size.height - 4.0;
        let first = (self.scroll_y / ROW_HEIGHT).floor().max(0.0) as usize;
        let shown = ((height / ROW_HEIGHT).ceil() as usize + 1).min(self.lines.len().saturating_sub(first));
        let z = self.z;
        while self.rows.len() < shown {
            self.rows.push(RowView {
                bg: PlaneBuilder::new().color(theme::TRANSPARENT).build(),
                icon: theme::icon("", 13.0, theme::TEXT),
                time: theme::mono("", FONT_SIZE, theme::TEXT_DIM),
                source: theme::mono("", FONT_SIZE, theme::TEXT_DIM),
                message: theme::mono("", FONT_SIZE, theme::TEXT),
            });
        }
        self.rows.truncate(shown);

        let x0 = self.region.position.x + 6.0;
        let width = self.region.size.width;
        let time_x = x0 + 20.0;
        // The time column goes first when the console gets narrow.
        let show_time = width > 420.0;
        let source_chars = if width > 600.0 { SOURCE_CHARS } else { SOURCE_CHARS_NARROW };
        let source_x = if show_time { time_x + self.char_w * 10.0 } else { time_x };
        let message_x = source_x + self.char_w * (source_chars as f32 + 1.0);
        let message_chars =
            ((self.region.position.x + width - 8.0 - message_x) / self.char_w).floor().max(0.0) as usize;

        for (row_index, line_index) in (first..first + shown).enumerate() {
            let line = self.lines[line_index];
            let entry = &self.entries[line.entry];
            let selected = Some(entry.seq) == self.selected;
            let row = &mut self.rows[row_index];
            let y = top + line_index as f32 * ROW_HEIGHT - self.scroll_y;
            row.bg.set_position(Position::new(self.region.position.x + 1.0, y, z + 1.0));
            row.bg.set_size(Size::new(width - 2.0, ROW_HEIGHT));
            row.bg.set_z_index(z + 1.0);
            let text_y = theme::center_y(&row.message, ROW_HEIGHT);
            let level = entry.level;
            if line.line == 0 {
                row.icon.set_content(level_icon(level));
                row.icon.set_color(icon_color(level));
                let icon_y = theme::center_y(&row.icon, ROW_HEIGHT);
                row.icon.prepare_render_at(Position::new(x0, y + icon_y, z + 2.0));
                let time = if show_time { format_time(entry.time) } else { String::new() };
                row.time.set_content(&time);
                row.time.prepare_render_at(Position::new(time_x, y + text_y, z + 2.0));
                row.source.set_content(&cut(entry.short_target(), source_chars));
                row.source.set_color(match entry.source() {
                    LogSource::Script => theme::ACCENT_TEXT,
                    LogSource::Engine => theme::TEXT_DIM,
                });
                row.source.prepare_render_at(Position::new(source_x, y + text_y, z + 2.0));
                let mut message_lines = entry.message.lines();
                let first_line = message_lines.next().unwrap_or("");
                let more = message_lines.count();
                let text = if more > 0 && !selected {
                    format!("{first_line}  (+{more} lines)")
                } else {
                    first_line.to_owned()
                };
                row.message.set_content(&cut(&text, message_chars));
                row.message.prepare_render_at(Position::new(message_x, y + text_y, z + 2.0));
            } else {
                // Further lines of the expanded message, below its first.
                row.icon.set_content("");
                row.time.set_content("");
                row.source.set_content("");
                let text = entry.message.lines().nth(line.line).unwrap_or("");
                row.message.set_content(&cut(text, message_chars));
                row.message.prepare_render_at(Position::new(message_x, y + text_y, z + 2.0));
            }
            row.message.set_color(text_color(level));
            for text in [&mut row.icon, &mut row.time, &mut row.source, &mut row.message] {
                text.set_z_index(z + 2.0);
            }
        }

        let empty = if self.lines.is_empty() {
            if self.entries.is_empty() {
                "No messages yet"
            } else {
                "No messages match the filter"
            }
        } else {
            ""
        };
        self.empty.set_content(empty);
        let w = self.empty.measure_width();
        self.empty.prepare_render_at(Position::new(
            (self.region.position.x + (width - w) * 0.5).round(),
            (self.region.position.y + (self.region.size.height - self.empty.line_height()) * 0.5).round(),
            z + 2.0,
        ));
        self.empty.set_z_index(z + 2.0);
        self.update_colors();
    }

    fn update_colors(&mut self) {
        let first = (self.scroll_y / ROW_HEIGHT).floor().max(0.0) as usize;
        for (row_index, row) in self.rows.iter_mut().enumerate() {
            let line_index = first + row_index;
            let Some(line) = self.lines.get(line_index) else {
                continue;
            };
            let entry = &self.entries[line.entry];
            let color = if Some(entry.seq) == self.selected {
                theme::ACCENT_SOFT
            } else if self
                .hover
                .and_then(|h| self.lines.get(h))
                .is_some_and(|h| h.entry == line.entry)
            {
                theme::BG_ROW_HOVER
            } else {
                row_tint(entry.level)
            };
            row.bg.set_color(color);
        }
    }

    fn render(&self) {
        self.frame.render();
        ferrite_ui::ui::clip::push(&self.frame);
        for row in &self.rows {
            row.bg.render();
            row.icon.render();
            row.time.render();
            row.source.render();
            row.message.render();
        }
        self.empty.render();
        ferrite_ui::ui::clip::pop(&self.frame);
    }

    fn set_z_index(&mut self, z: f32) {
        self.z = z;
        self.frame.set_z_index(z);
        self.region.position.z = z;
        self.dirty = true;
    }
}

/// A filter button: a level's icon and how many messages it has, or a
/// source's name.
fn chip(icon: &str, label: &str, tooltip: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Button {
    let mut button = Button::new(Position::default(), Size::new(10.0, BUTTON_HEIGHT), on_click);
    button.set_style(ButtonStyle::Ghost);
    button.set_icon(icon);
    button.set_label(label);
    button.set_tooltip(tooltip);
    button.fit(BUTTON_HEIGHT);
    button
}

fn icon_button(icon: &str, tooltip: &str, on_click: Box<dyn Fn(&mut Scene)>) -> Button {
    let mut button = Button::new(
        Position::default(),
        Size::new(BUTTON_HEIGHT + 2.0, BUTTON_HEIGHT),
        on_click,
    );
    button.set_style(ButtonStyle::Ghost);
    button.set_icon(icon);
    button.set_tooltip(tooltip);
    button
}

pub struct ConsolePanel {
    handle: UIElementHandle,
    state: Shared,
    panel: Panel,
    levels: Vec<Button>,
    engine: Button,
    scripts: Button,
    search: Input<String>,
    search_text: DataSource<String>,
    search_hint: Text,
    search_icon: Text,
    /// The search field is left out when the console is too narrow.
    search_shown: bool,
    /// Width of the source buttons with their names.
    sources_named_w: f32,
    z: f32,
    follow: Button,
    clear: Button,
    /// Set by the buttons, carried out on the next update.
    follow_requested: DataSource<bool>,
    clear_requested: DataSource<bool>,
    view: LogView,
}

impl ConsolePanel {
    pub fn new(state: Shared) -> Self {
        let mut panel = Panel::new("Console".into(), Position::default(), Size::new(600.0, 200.0));
        panel.set_movable(false);
        panel.set_fixed_size(true);
        panel.set_flat(true);

        let levels = LEVELS
            .iter()
            .map(|&level| {
                let state = state.clone();
                let mut button = chip(
                    level_icon(level),
                    "0",
                    &format!("Show {}", level_name(level)),
                    Box::new(move |_| {
                        let mut state = state.borrow_mut();
                        let shown = state.console.level_mut(level);
                        *shown = !*shown;
                    }),
                );
                button.set_icon_color(Some(icon_color(level)));
                button
            })
            .collect();
        let engine_state = state.clone();
        let engine = chip(
            icons::CPU,
            "Engine",
            "Show messages from the engine and the editor",
            Box::new(move |_| {
                let mut state = engine_state.borrow_mut();
                state.console.engine = !state.console.engine;
            }),
        );
        let scripts_state = state.clone();
        let scripts = chip(
            icons::FILE_CODE,
            "Scripts",
            "Show messages from the game's scripts",
            Box::new(move |_| {
                let mut state = scripts_state.borrow_mut();
                state.console.scripts = !state.console.scripts;
            }),
        );

        let search_text = DataSource::new(String::new());
        let search = Input::new(
            Position::default(),
            Size::new(SEARCH_WIDTH, BUTTON_HEIGHT),
            String::new(),
            Some(search_text.clone()),
        );

        let follow_requested = DataSource::new(false);
        let follow_flag = follow_requested.clone();
        let follow = icon_button(
            icons::ARROW_DOWN_TO_LINE,
            "Follow new messages",
            Box::new(move |_| follow_flag.write(true)),
        );
        let clear_requested = DataSource::new(false);
        let clear_flag = clear_requested.clone();
        let mut clear = icon_button(
            icons::TRASH,
            "Clear the console",
            Box::new(move |_| clear_flag.write(true)),
        );
        clear.set_style(ButtonStyle::Danger);

        let sources_named_w = engine.content_width() + scripts.content_width() + 2.0;
        Self {
            handle: UIElementHandle::new(),
            state,
            panel,
            levels,
            engine,
            scripts,
            search,
            search_text,
            search_hint: theme::text("Search", FONT_SIZE, theme::TEXT_DIM),
            search_icon: theme::icon(icons::SEARCH, 13.0, theme::TEXT_DIM),
            search_shown: true,
            sources_named_w,
            z: 0.0,
            follow,
            clear,
            follow_requested,
            clear_requested,
            view: LogView::new(),
        }
    }

    fn rect(&self) -> super::layout::Rect {
        self.state.borrow().layout.console
    }

    fn visible(&self) -> bool {
        let rect = self.rect();
        rect.w > 0.0 && rect.h > 0.0
    }

    fn widgets_mut(&mut self) -> Vec<&mut dyn UIElement> {
        let mut widgets: Vec<&mut dyn UIElement> = Vec::new();
        for button in &mut self.levels {
            widgets.push(button);
        }
        widgets.push(&mut self.engine);
        widgets.push(&mut self.scripts);
        if self.search_shown {
            widgets.push(&mut self.search);
        }
        widgets.push(&mut self.follow);
        widgets.push(&mut self.clear);
        widgets
    }

    fn widgets(&self) -> Vec<&dyn UIElement> {
        let mut widgets: Vec<&dyn UIElement> = Vec::new();
        for button in &self.levels {
            widgets.push(button);
        }
        widgets.push(&self.engine);
        widgets.push(&self.scripts);
        if self.search_shown {
            widgets.push(&self.search);
        }
        widgets.push(&self.follow);
        widgets.push(&self.clear);
        widgets
    }

    /// Lay the action row out: level and source filters on the left,
    /// search, follow and clear on the right. When the console gets narrow
    /// the source buttons lose their names first, then the search field
    /// goes.
    fn place_actions(&mut self, rect: super::layout::Rect, y: f32) {
        let left = rect.x + 6.0;
        let right = rect.x + rect.w - 6.0;
        let levels_w: f32 = self
            .levels
            .iter_mut()
            .map(|button| {
                button.fit(BUTTON_HEIGHT);
                button.get_size().width + 2.0
            })
            .sum();
        let tools_w = self.clear.get_size().width + 2.0 + self.follow.get_size().width;
        let room = right - left - levels_w - 8.0 - tools_w - 8.0;
        let sources_named = self.sources_named_w;
        let sources_icons = 2.0 * (BUTTON_HEIGHT + 4.0) + 2.0;
        const MIN_SEARCH: f32 = 90.0;
        let (named, search) = if room >= sources_named + 8.0 + MIN_SEARCH {
            (true, true)
        } else if room >= sources_icons + 8.0 + MIN_SEARCH {
            (false, true)
        } else {
            (false, false)
        };
        for (button, label) in [(&mut self.engine, "Engine"), (&mut self.scripts, "Scripts")] {
            if named {
                button.set_label(label);
                button.fit(BUTTON_HEIGHT);
            } else {
                button.set_label("");
                button.set_size(Size::new(BUTTON_HEIGHT + 4.0, BUTTON_HEIGHT));
            }
        }
        self.search_shown = search;

        let mut x = left;
        for button in &mut self.levels {
            button.set_offset(Offset { x, y });
            x += button.get_size().width + 2.0;
        }
        x += 6.0;
        for button in [&mut self.engine, &mut self.scripts] {
            button.set_offset(Offset { x, y });
            x += button.get_size().width + 2.0;
        }
        let clear_x = right - self.clear.get_size().width;
        self.clear.set_offset(Offset { x: clear_x, y });
        let follow_x = clear_x - 2.0 - self.follow.get_size().width;
        self.follow.set_offset(Offset { x: follow_x, y });
        // The search field takes what is left, up to its full width.
        let search_w = (follow_x - 8.0 - (x + 6.0)).min(SEARCH_WIDTH).max(MIN_SEARCH);
        let search_x = follow_x - 8.0 - search_w;
        self.search.set_size(Size::new(search_w, BUTTON_HEIGHT));
        self.search.set_offset(Offset { x: search_x, y });
        let hint_y = theme::center_y(&self.search_hint, BUTTON_HEIGHT);
        self.search_icon.prepare_render_at(Position::new(
            search_x + 8.0,
            y + theme::center_y(&self.search_icon, BUTTON_HEIGHT),
            self.z + 4.0,
        ));
        self.search_hint
            .prepare_render_at(Position::new(search_x + 26.0, y + hint_y, self.z + 4.0));
    }

    fn copy_selection(&self, window: &mut glfw::Window) -> bool {
        match self.view.selected_entry() {
            Some(entry) => {
                window.set_clipboard_string(&entry_text(entry));
                self.state.borrow_mut().set_status("Copied the message");
                true
            }
            None => false,
        }
    }
}

impl UIElement for ConsolePanel {
    fn update(&mut self, scene: &mut Scene) {
        // Read new messages even while hidden, so the counts are right when
        // the console is shown again.
        self.view.pull();
        let rect = self.rect();
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        if self.clear_requested.read() {
            self.clear_requested.write(false);
            self.view.clear();
        }
        if self.follow_requested.read() {
            self.follow_requested.write(false);
            self.view.scroll_to_bottom();
        }
        self.panel.set_size(Size::new(rect.w, rect.h));
        self.panel.set_position(Position::new(rect.x, rect.y, 0.0));

        let filter = self.state.borrow().console.clone();
        self.view.set_filter(&filter, &self.search_text.read());
        for (button, level) in self.levels.iter_mut().zip(LEVELS) {
            let shown = filter.level(level);
            button.set_label(&count_label(self.view.counts[level_slot(level)]));
            button.set_active(shown);
            let (r, g, b, _) = icon_color(level);
            button.set_icon_color(Some(if shown { (r, g, b, 1.0) } else { (r, g, b, 0.45) }));
        }
        self.engine.set_active(filter.engine);
        self.scripts.set_active(filter.scripts);
        self.follow.set_active(self.view.follow);

        let header = self.panel.header_height();
        let actions_y = rect.y + header + ((ACTIONS_HEIGHT - BUTTON_HEIGHT) * 0.5).round() + 2.0;
        self.place_actions(rect, actions_y);

        let list_top = rect.y + header + ACTIONS_HEIGHT + 4.0;
        self.view.set_rect(
            rect.x + 6.0,
            list_top,
            rect.w - 12.0,
            rect.y + rect.h - list_top - 6.0,
        );
        self.view.layout();

        self.panel.update(scene);
        for widget in self.widgets_mut() {
            widget.update(scene);
        }
    }

    fn render(&self) {
        if !self.visible() {
            return;
        }
        self.panel.render();
        self.view.render();
        for widget in self.widgets() {
            widget.render();
        }
        if self.search_shown && self.search_text.read().is_empty() && !self.search.is_focused() {
            self.search_icon.render();
            self.search_hint.render();
        }
    }

    fn render_overlay(&self) {
        if !self.visible() {
            return;
        }
        for widget in self.widgets() {
            widget.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        if !self.visible() {
            return false;
        }
        let mut handled = false;
        for widget in self.widgets_mut() {
            if widget.handle_events(scene, window, glfw, event) {
                handled = true;
                if !matches!(event, WindowEvent::CursorPos(..)) {
                    return true;
                }
            }
        }
        let (x, y) = window.get_cursor_pos();
        let (x, y) = (x as f32, y as f32);
        let inside = self.rect().contains(x, y);
        match event {
            WindowEvent::CursorPos(..) => {
                let hover = if handled { None } else { self.view.line_at(x, y) };
                if hover != self.view.hover {
                    self.view.hover = hover;
                    self.view.update_colors();
                }
                handled
            }
            WindowEvent::Scroll(_, dy) if self.view.region.contains(x, y) => {
                self.view.scroll_y = (self.view.scroll_y - *dy as f32 * ROW_HEIGHT * 3.0)
                    .clamp(0.0, self.view.max_scroll());
                self.view.follow = self.view.at_bottom();
                self.view.dirty = true;
                true
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                if let Some(line) = self.view.line_at(x, y) {
                    self.view.toggle_selection(line);
                }
                inside
            }
            WindowEvent::Key(Key::C, _, Action::Press, mods)
                if mods.contains(Modifiers::Control) && inside =>
            {
                self.copy_selection(window)
            }
            WindowEvent::MouseButton(..) | WindowEvent::Scroll(..) => inside,
            _ => false,
        }
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _parent: UIElementHandle, _child: Box<dyn UIElement>) {}
    fn contains_child(&self, _handle: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _handle: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _handle: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        self.panel.get_offset()
    }
    fn set_offset(&mut self, _offset: Offset) {}
    fn get_size(&self) -> &Size {
        self.panel.get_size()
    }
    fn set_z_index(&mut self, z_index: f32) {
        self.z = z_index;
        self.panel.set_z_index(z_index);
        self.view.set_z_index(z_index + 1.0);
        self.search_hint.set_z_index(z_index + 4.0);
        self.search_icon.set_z_index(z_index + 4.0);
        for widget in self.widgets_mut() {
            widget.set_z_index(z_index + 2.0);
        }
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
    fn filter_by_level_source_and_text() {
        let script = entry(Level::Warn, "ferrite_scripts::player", "Jumped too high");
        let engine = entry(Level::Debug, "ferrite::terrain", "chunk ready");
        let mut filter = ConsoleFilter::default();
        assert!(filter.shows(&script, ""));
        assert!(filter.shows(&engine, ""));
        assert!(filter.shows(&script, "jumped"));
        assert!(filter.shows(&script, "player"));
        assert!(!filter.shows(&engine, "jumped"));

        filter.warnings = false;
        assert!(!filter.shows(&script, ""));
        filter.warnings = true;
        filter.engine = false;
        assert!(!filter.shows(&engine, ""));
        assert!(filter.shows(&script, ""));
        filter.debug = false;
        filter.engine = true;
        assert!(!filter.shows(&entry(Level::Trace, "ferrite", "x"), ""));
    }

    #[test]
    fn times_and_cut_text() {
        assert_eq!(format_time(Duration::from_millis(12_345)), "00:12.345");
        assert_eq!(format_time(Duration::from_millis(3_723_004)), "1:02:03.004");
        assert_eq!(cut("player", 18), "player");
        assert_eq!(cut("abcdef", 4), "abc\u{2026}");
        assert_eq!(count_label(1000), "999+");
    }
}
