//! The timeline below the preview: keys per bone over the clip's length.

use cgmath::{InnerSpace, Matrix4, Point3, Vector3};
use ferrite::core::{
    model::clip_file::KEY_EPSILON,
    primitives::{Offset, Position, Size},
    renderer::{
        line::{Line, LineRenderer},
        plane::{Plane, PlaneBuilder},
        text::{icons, Text},
    },
    scene::Scene,
};
use ferrite_ui::ui::{
    elements::button::{Button, ButtonStyle},
    theme, UIElement,
};
use glfw::{Action, Glfw, MouseButton, WindowEvent};

use crate::editor::{layout::Rect, Shared};

use super::*;

/// Height of the timeline below the 3D view.
pub const TIMELINE_HEIGHT: f32 = 176.0;
const TOOLBAR_H: f32 = 32.0;
const RULER_H: f32 = 22.0;
const ROW_H: f32 = 19.0;
const LABEL_W: f32 = 168.0;
/// Room left and right of the time axis so keys at the ends can be taken.
const PAD: f32 = 10.0;
const KEY_PICK: f32 = 7.0;

/// What the timeline shows, taken from the clip each frame.
#[derive(Clone, Default)]
pub struct TimelineView {
    pub duration: f32,
    pub time: f32,
    pub playing: bool,
    pub looping: bool,
    pub auto_key: bool,
    pub selected: Option<String>,
    /// Rows below the summary: bone and its key times.
    pub rows: Vec<(String, Vec<f32>)>,
    /// Key times of all bones.
    pub all: Vec<f32>,
}

impl TimelineView {
    pub fn of(clip: &ClipEdit, order: &[String]) -> Self {
        let mut rows: Vec<(String, Vec<f32>)> = clip
            .file
            .tracks
            .iter()
            .map(|t| (t.bone.clone(), t.keys.iter().map(|k| k.time).collect()))
            .collect();
        let rank = |bone: &str| order.iter().position(|b| b == bone).unwrap_or(usize::MAX);
        rows.sort_by_key(|(bone, _)| rank(bone));
        if let Some(selected) = &clip.selected {
            if !rows.iter().any(|(bone, _)| bone == selected) {
                rows.insert(0, (selected.clone(), Vec::new()));
            }
        }
        Self {
            duration: clip.file.duration,
            time: clip.time,
            playing: clip.playing,
            looping: clip.looping,
            auto_key: clip.auto_key,
            selected: clip.selected.clone(),
            rows,
            all: clip.file.key_times(),
        }
    }
}

/// What the user did in the timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineAction {
    Scrub(f32),
    Select(Option<String>),
    /// A key was pressed: show its pose, and select its bone if it has one.
    TakeKey {
        bone: Option<String>,
        time: f32,
    },
    /// A key (of one bone, or all keys at a time) was dragged.
    MoveKey {
        bone: Option<String>,
        from: f32,
        to: f32,
    },
}

enum TimelineDrag {
    Scrub,
    Key {
        bone: Option<String>,
        from: f32,
        to: f32,
    },
}

pub struct Timeline {
    background: Plane,
    toolbar: Plane,
    selected_row: Plane,
    play: Button,
    looping: Button,
    key: Button,
    delete: Button,
    auto_key: Button,
    start: Button,
    time_text: Text,
    ruler_texts: Vec<Text>,
    row_texts: Vec<Text>,
    rect: Rect,
    view: TimelineView,
    /// First bone row shown (scrolled).
    scroll: usize,
    drag: Option<TimelineDrag>,
    cursor: (f32, f32),
}

fn request_button(
    state: &Shared,
    label: &str,
    icon: &str,
    tooltip: &str,
    request: ClipRequest,
) -> Button {
    let state = state.clone();
    let mut button = Button::new(
        Position::default(),
        Size::new(10.0, 24.0),
        Box::new(move |_| {
            if let Some(clip) = state
                .borrow_mut()
                .asset_edit
                .as_mut()
                .and_then(|edit| edit.clip.as_mut())
            {
                clip.requests.push(request);
            }
        }),
    );
    button.set_style(ButtonStyle::Secondary);
    button.set_icon(icon);
    button.set_label(label);
    button.fit(24.0);
    button.set_tooltip(tooltip);
    button
}

impl Timeline {
    pub fn new(state: Shared) -> Self {
        let play = request_button(
            &state,
            "Play",
            icons::PLAY,
            "Play the clip (Space)",
            ClipRequest::TogglePlay,
        );
        let looping = request_button(
            &state,
            "Loop",
            icons::ROTATE_CW,
            "Start over at the end",
            ClipRequest::ToggleLoop,
        );
        let start = request_button(
            &state,
            "",
            icons::ARROW_LEFT,
            "Back to the start (Home)",
            ClipRequest::ToStart,
        );
        let mut key = request_button(
            &state,
            "Key",
            icons::PLUS,
            "Key the selected bone and the posed bones at the playhead (K)",
            ClipRequest::Key,
        );
        key.set_style(ButtonStyle::Primary);
        let mut delete = request_button(
            &state,
            "Delete key",
            icons::TRASH,
            "Remove the selected bone's key at the playhead, or all keys there (Delete)",
            ClipRequest::DeleteKey,
        );
        delete.set_style(ButtonStyle::Danger);
        let auto_key = request_button(
            &state,
            "Auto key",
            icons::ZAP,
            "Key bones as soon as they are posed",
            ClipRequest::ToggleAutoKey,
        );
        Self {
            background: PlaneBuilder::new().color(theme::BG_PANEL).build(),
            toolbar: PlaneBuilder::new().color(theme::BG_HEADER).build(),
            selected_row: PlaneBuilder::new().color(theme::ACCENT_SOFT).build(),
            play,
            looping,
            key,
            delete,
            auto_key,
            start,
            time_text: theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT),
            ruler_texts: Vec::new(),
            row_texts: Vec::new(),
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            view: TimelineView::default(),
            scroll: 0,
            drag: None,
            cursor: (0.0, 0.0),
        }
    }

    fn buttons_mut(&mut self) -> [&mut Button; 6] {
        [
            &mut self.play,
            &mut self.start,
            &mut self.looping,
            &mut self.key,
            &mut self.delete,
            &mut self.auto_key,
        ]
    }

    fn axis(&self) -> (f32, f32) {
        let x0 = self.rect.x + LABEL_W + PAD;
        let x1 = (self.rect.x + self.rect.w - PAD).max(x0 + 1.0);
        (x0, x1)
    }

    fn x_of(&self, time: f32) -> f32 {
        let (x0, x1) = self.axis();
        x0 + (x1 - x0) * (time / self.view.duration.max(1e-3))
    }

    fn time_at(&self, x: f32) -> f32 {
        let (x0, x1) = self.axis();
        ((x - x0) / (x1 - x0) * self.view.duration).clamp(0.0, self.view.duration)
    }

    fn ruler_y(&self) -> f32 {
        self.rect.y + TOOLBAR_H
    }

    fn rows_y(&self) -> f32 {
        self.ruler_y() + RULER_H
    }

    /// Rows that fit, the summary row included.
    fn visible_rows(&self) -> usize {
        (((self.rect.y + self.rect.h) - self.rows_y()) / ROW_H)
            .floor()
            .max(1.0) as usize
    }

    /// Row under a window y: 0 is the summary, then `scroll + i - 1`.
    fn row_at(&self, y: f32) -> Option<Option<usize>> {
        if y < self.rows_y() {
            return None;
        }
        let i = ((y - self.rows_y()) / ROW_H) as usize;
        if i >= self.visible_rows() {
            return None;
        }
        if i == 0 {
            return Some(None);
        }
        let row = self.scroll + i - 1;
        (row < self.view.rows.len()).then_some(Some(row))
    }

    /// Steps between ruler ticks and labels so labels do not crowd.
    fn steps(&self) -> (f32, f32) {
        let (x0, x1) = self.axis();
        let per_second = (x1 - x0) / self.view.duration.max(1e-3);
        for label in [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0] {
            if label * per_second >= 56.0 {
                let tick = if label >= 1.0 {
                    label / 4.0
                } else {
                    label / 5.0
                };
                return (tick, label);
            }
        }
        (15.0, 60.0)
    }

    pub fn update(&mut self, scene: &mut Scene, rect: Rect, view: TimelineView, z: f32) {
        self.rect = rect;
        self.view = view;
        let max_scroll = self.view.rows.len().saturating_sub(self.visible_rows() - 1);
        self.scroll = self.scroll.min(max_scroll);

        self.background
            .set_position(Position::new(rect.x, rect.y, z));
        self.background.set_size(Size::new(rect.w, rect.h));
        self.toolbar
            .set_position(Position::new(rect.x, rect.y, z + 0.1));
        self.toolbar.set_size(Size::new(rect.w, TOOLBAR_H));

        self.play
            .set_label(if self.view.playing { "Pause" } else { "Play" });
        self.play.set_icon(if self.view.playing {
            icons::SQUARE
        } else {
            icons::PLAY
        });
        self.play.fit(24.0);
        self.looping.set_active(self.view.looping);
        self.auto_key.set_active(self.view.auto_key);
        let y = rect.y + 4.0;
        let mut x = rect.x + 6.0;
        for button in self.buttons_mut() {
            button.set_offset(Offset { x, y });
            x += button.get_size().width + 4.0;
            button.set_z_index(z + 1.0);
            button.update(scene);
        }
        self.time_text.set_content(&format!(
            "{:.2} / {:.2} s",
            self.view.time, self.view.duration
        ));
        let tw = self.time_text.measure_width();
        self.time_text.prepare_render_at(Position::new(
            (rect.x + rect.w - tw - 10.0).max(x + 8.0),
            y + 5.0,
            z + 1.0,
        ));

        // Ruler labels.
        let (_, label_step) = self.steps();
        let count = (self.view.duration / label_step).floor() as usize + 1;
        while self.ruler_texts.len() < count {
            self.ruler_texts.push(theme::text(
                "",
                theme::FONT_SIZE_SMALL - 1.5,
                theme::TEXT_DIM,
            ));
        }
        let ruler_y = self.ruler_y();
        for i in 0..count {
            let t = i as f32 * label_step;
            let label = if label_step < 1.0 {
                format!("{t:.2}")
            } else {
                format!("{t:.0}")
            };
            let x = self.x_of(t) + 3.0;
            let text = &mut self.ruler_texts[i];
            text.set_content(&label);
            text.prepare_render_at(Position::new(x, ruler_y + 3.0, z + 1.0));
        }
        self.ruler_texts.truncate(count);

        // Row labels.
        let shown = self.visible_rows();
        while self.row_texts.len() < shown {
            self.row_texts
                .push(theme::text("", theme::FONT_SIZE_SMALL, theme::TEXT_MUTED));
        }
        self.row_texts.truncate(shown);
        let rows_y = self.rows_y();
        self.selected_row.set_size(Size::new(0.0, 0.0));
        for i in 0..shown {
            let label = if i == 0 {
                Some(("All keys".to_string(), false))
            } else {
                self.view.rows.get(self.scroll + i - 1).map(|(bone, _)| {
                    let name = ferrite::core::model::ik::base_name(bone).to_string();
                    (name, self.view.selected.as_deref() == Some(bone.as_str()))
                })
            };
            let ry = rows_y + i as f32 * ROW_H;
            let text = &mut self.row_texts[i];
            match label {
                Some((label, selected)) => {
                    text.set_content(&label);
                    text.set_color(if selected {
                        theme::TEXT
                    } else {
                        theme::TEXT_MUTED
                    });
                    if selected {
                        self.selected_row
                            .set_position(Position::new(rect.x, ry, z + 0.2));
                        self.selected_row.set_size(Size::new(rect.w, ROW_H));
                    }
                }
                None => text.set_content(""),
            }
            text.prepare_render_at(Position::new(rect.x + 10.0, ry + 2.0, z + 1.0));
        }
    }

    pub fn render(&self, window: (f32, f32)) {
        self.background.render();
        self.toolbar.render();
        self.selected_row.render();

        let rect = self.rect;
        let ruler_y = self.ruler_y();
        let rows_y = self.rows_y();
        let bottom = rect.y + rect.h;

        // Separators and the label column.
        let mut frame = vec![
            seg((rect.x, ruler_y), (rect.x + rect.w, ruler_y)),
            seg((rect.x, rows_y), (rect.x + rect.w, rows_y)),
            seg((rect.x + LABEL_W, ruler_y), (rect.x + LABEL_W, bottom)),
        ];
        for i in 1..self.visible_rows() {
            let y = rows_y + i as f32 * ROW_H;
            frame.push(seg((rect.x, y), (rect.x + rect.w, y)));
        }
        draw_lines(window, &frame, color3(theme::BORDER), 1);

        // Ruler ticks.
        let (tick, label) = self.steps();
        let mut minor = Vec::new();
        let mut major = Vec::new();
        let mut t = 0.0;
        let mut i = 0;
        while t <= self.view.duration + 1e-4 {
            let x = self.x_of(t);
            let is_label = ((t / label) - (t / label).round()).abs() < 1e-3;
            if is_label {
                major.push(seg((x, ruler_y), (x, bottom)));
            } else {
                minor.push(seg((x, ruler_y + RULER_H - 6.0), (x, ruler_y + RULER_H)));
            }
            i += 1;
            t = i as f32 * tick;
        }
        draw_lines(window, &major, Vector3::new(0.13, 0.135, 0.16), 1);
        draw_lines(window, &minor, color3(theme::TEXT_DIM), 1);
        // The end of the clip.
        let end = self.x_of(self.view.duration);
        draw_lines(
            window,
            &vec![seg((end, ruler_y), (end, bottom))],
            color3(theme::BORDER_STRONG),
            1,
        );

        // Keys.
        let mut keys = Vec::new();
        let mut hot = Vec::new();
        let dragged = match &self.drag {
            Some(TimelineDrag::Key { bone, from, to }) => Some((bone.clone(), *from, *to)),
            _ => None,
        };
        let shown = self.visible_rows();
        for i in 0..shown {
            let (bone, times) = if i == 0 {
                (None, &self.view.all)
            } else {
                match self.view.rows.get(self.scroll + i - 1) {
                    Some((bone, times)) => (Some(bone.clone()), times),
                    None => continue,
                }
            };
            let cy = rows_y + i as f32 * ROW_H + ROW_H * 0.5;
            for &time in times {
                let mut time = time;
                let mut moving = false;
                if let Some((dragged_bone, from, to)) = &dragged {
                    let same_row = dragged_bone == &bone || dragged_bone.is_none();
                    if same_row && (time - from).abs() < KEY_EPSILON {
                        time = *to;
                        moving = true;
                    }
                }
                let at_playhead = (time - self.view.time).abs() < KEY_EPSILON;
                let target = if moving || at_playhead {
                    &mut hot
                } else {
                    &mut keys
                };
                target.extend(key_shape((self.x_of(time), cy), 4.5));
            }
        }
        draw_lines(window, &keys, color3(theme::WARNING), 2);
        draw_lines(window, &hot, Vector3::new(1.0, 0.85, 0.5), 2);

        // Playhead.
        let px = self.x_of(self.view.time);
        draw_lines(
            window,
            &vec![seg((px, ruler_y), (px, bottom))],
            color3(theme::ACCENT),
            2,
        );

        for button in [
            &self.play,
            &self.start,
            &self.looping,
            &self.key,
            &self.delete,
            &self.auto_key,
        ] {
            button.render();
        }
        self.time_text.render();
        for text in self.ruler_texts.iter().chain(&self.row_texts) {
            text.render();
        }
    }

    pub fn render_overlay(&self) {
        for button in [
            &self.play,
            &self.start,
            &self.looping,
            &self.key,
            &self.delete,
            &self.auto_key,
        ] {
            button.render_overlay();
        }
    }

    /// The key under a window position: its row's bone (`None` for the
    /// summary row) and time.
    fn key_at(&self, x: f32, y: f32) -> Option<(Option<String>, f32)> {
        let row = self.row_at(y)?;
        let (bone, times) = match row {
            None => (None, &self.view.all),
            Some(row) => {
                let (bone, times) = &self.view.rows[row];
                (Some(bone.clone()), times)
            }
        };
        times
            .iter()
            .map(|&t| (t, (self.x_of(t) - x).abs()))
            .filter(|(_, d)| *d <= KEY_PICK)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(t, _)| (bone, t))
    }

    /// Handle an event; returns whether it was used and what it asks for.
    pub fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> (bool, Option<TimelineAction>) {
        for button in self.buttons_mut() {
            if button.handle_events(scene, window, glfw, event)
                && !matches!(event, WindowEvent::CursorPos(..))
            {
                return (true, None);
            }
        }
        match event {
            WindowEvent::MouseButton(MouseButton::Button1, Action::Press, _) => {
                let (x, y) = window.get_cursor_pos();
                let (x, y) = (x as f32, y as f32);
                if !self.rect.contains(x, y) || y < self.ruler_y() {
                    return (self.rect.contains(x, y), None);
                }
                if x < self.rect.x + LABEL_W {
                    // A row label selects its bone.
                    return match self.row_at(y) {
                        Some(Some(row)) => {
                            let bone = self.view.rows[row].0.clone();
                            (true, Some(TimelineAction::Select(Some(bone))))
                        }
                        Some(None) => (true, Some(TimelineAction::Select(None))),
                        None => (true, None),
                    };
                }
                if let Some((bone, time)) = self.key_at(x, y) {
                    self.drag = Some(TimelineDrag::Key {
                        bone: bone.clone(),
                        from: time,
                        to: time,
                    });
                    return (true, Some(TimelineAction::TakeKey { bone, time }));
                }
                self.drag = Some(TimelineDrag::Scrub);
                (true, Some(TimelineAction::Scrub(self.time_at(x))))
            }
            WindowEvent::MouseButton(MouseButton::Button1, Action::Release, _) => {
                match self.drag.take() {
                    Some(TimelineDrag::Key { bone, from, to }) => {
                        (true, Some(TimelineAction::MoveKey { bone, from, to }))
                    }
                    Some(TimelineDrag::Scrub) => (true, None),
                    None => (false, None),
                }
            }
            WindowEvent::CursorPos(x, y) => {
                self.cursor = (*x as f32, *y as f32);
                let time = self.time_at(*x as f32);
                match &mut self.drag {
                    Some(TimelineDrag::Scrub) => (true, Some(TimelineAction::Scrub(time))),
                    Some(TimelineDrag::Key { to, .. }) => {
                        *to = snap(time, self.view.duration);
                        (true, None)
                    }
                    None => (false, None),
                }
            }
            WindowEvent::Scroll(_, dy) => {
                let (x, y) = window.get_cursor_pos();
                if !self.rect.contains(x as f32, y as f32) {
                    return (false, None);
                }
                if *dy < 0.0 {
                    self.scroll += 1;
                } else {
                    self.scroll = self.scroll.saturating_sub(1);
                }
                (true, None)
            }
            _ => (false, None),
        }
    }
}

/// Keys snap to hundredths of a second.
fn snap(time: f32, duration: f32) -> f32 {
    ((time * 100.0).round() / 100.0).clamp(0.0, duration)
}

fn color3(c: theme::Color) -> Vector3<f32> {
    Vector3::new(c.0, c.1, c.2)
}

fn seg(a: (f32, f32), b: (f32, f32)) -> Line {
    let d = Vector3::new(b.0 - a.0, b.1 - a.1, 0.0);
    let length = d.magnitude();
    let direction = if length > 1e-6 {
        d / length
    } else {
        Vector3::unit_x()
    };
    Line::new(Point3::new(a.0, a.1, 0.0), direction, length.max(1e-3))
}

/// A diamond at a key, filled by nesting.
fn key_shape(c: (f32, f32), size: f32) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut s = size;
    while s > 0.5 {
        lines.push(seg((c.0 - s, c.1), (c.0, c.1 - s)));
        lines.push(seg((c.0, c.1 - s), (c.0 + s, c.1)));
        lines.push(seg((c.0 + s, c.1), (c.0, c.1 + s)));
        lines.push(seg((c.0, c.1 + s), (c.0 - s, c.1)));
        s -= 1.5;
    }
    lines
}

/// Lines in window pixels, `width` px thick.
fn draw_lines(window: (f32, f32), lines: &Vec<Line>, color: Vector3<f32>, width: i32) {
    if lines.is_empty() {
        return;
    }
    let ortho = cgmath::ortho(0.0, window.0, window.1, 0.0, -100.0, 100.0);
    let half = width / 2;
    let extra = if width % 2 == 0 { 0 } else { half };
    for dy in -half..=extra {
        for dx in -half..=extra {
            let shift = Matrix4::from_translation(Vector3::new(
                dx as f32 * 2.0 / window.0,
                dy as f32 * 2.0 / window.1,
                0.0,
            ));
            LineRenderer::render_lines(&(shift * ortho), lines, color, true);
        }
    }
}
