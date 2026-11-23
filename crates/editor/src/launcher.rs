//! The screen the editor starts with when the working directory holds no
//! project: open a project file, create a new project, or open one of the
//! projects opened lately (`preferences::recent_projects`).

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
};

use ferrite::core::{
    primitives::{Offset, Position, Size},
    project::{Project, PROJECT_EXTENSION},
    renderer::text::{icons, Text},
    scene::Scene,
    window::Window,
};
use ferrite_ui::ui::{
    elements::button::{Button, ButtonStyle},
    theme, UIElement,
};
use glfw::{Glfw, WindowEvent};

const BUTTON_WIDTH: f32 = 240.0;
const BUTTON_HEIGHT: f32 = 34.0;
/// The recent projects' column: its width, the gap to the buttons, a row's
/// height.
const RECENT_WIDTH: f32 = 440.0;
const COLUMN_GAP: f32 = 48.0;
const ROW_HEIGHT: f32 = 46.0;

#[derive(Clone, Copy)]
enum Choice {
    Open,
    New,
    /// Open the recent project at this index.
    Recent(usize),
    /// Take the recent project at this index off the list.
    Forget(usize),
}

/// A recent project: a button over its whole row, its name and folder,
/// and a button taking it off the list.
struct RecentRow {
    path: PathBuf,
    button: Button,
    name: Text,
    folder: Text,
    forget: Button,
}

pub struct Launcher {
    /// Buttons are handed a scene; there is nothing in it.
    scene: Scene,
    open: Button,
    create: Button,
    title: Text,
    subtitle: Text,
    message: Text,
    recent_title: Text,
    recent: Vec<RecentRow>,
    choice: Rc<Cell<Option<Choice>>>,
}

/// `path` with the home folder as `~`, cut at the front to `max` characters.
fn short_path(path: &Path, max: usize) -> String {
    let text = path.display().to_string();
    let text = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && text.starts_with(&home) => format!("~{}", &text[home.len()..]),
        _ => text,
    };
    let count = text.chars().count();
    if count <= max {
        text
    } else {
        let tail: String = text.chars().skip(count - (max - 1)).collect();
        format!("\u{2026}{tail}")
    }
}

/// The rows for the recent projects that still exist.
fn recent_rows(choice: &Rc<Cell<Option<Choice>>>) -> Vec<RecentRow> {
    crate::preferences::get()
        .recent_projects
        .into_iter()
        .filter(|path| path.is_file())
        .enumerate()
        .map(|(index, path)| {
            let pick = choice.clone();
            let mut button = Button::new(
                Position::default(),
                Size::new(RECENT_WIDTH, ROW_HEIGHT),
                Box::new(move |_| pick.set(Some(Choice::Recent(index)))),
            );
            button.set_style(ButtonStyle::Ghost);
            let pick = choice.clone();
            let mut forget = Button::new(
                Position::default(),
                Size::new(24.0, 24.0),
                Box::new(move |_| pick.set(Some(Choice::Forget(index)))),
            );
            forget.set_style(ButtonStyle::Ghost);
            forget.set_icon(icons::X);
            forget.set_tooltip(tr!("launcher.forget_recent"));
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let folder = path.parent().map(|p| short_path(p, 60)).unwrap_or_default();
            RecentRow {
                button,
                name: theme::text(&name, theme::FONT_SIZE, theme::TEXT),
                folder: theme::text(&folder, theme::FONT_SIZE_SMALL, theme::TEXT_DIM),
                forget,
                path,
            }
        })
        .collect()
}

impl Launcher {
    /// `message`: why it is shown (e.g. a project that could not be opened).
    pub fn new(message: Option<String>) -> Self {
        let choice = Rc::new(Cell::new(None));
        let button = |label: &str, icon: &str, style: ButtonStyle, pick: Choice| {
            let choice = choice.clone();
            let mut button = Button::new(
                Position::default(),
                Size::new(BUTTON_WIDTH, BUTTON_HEIGHT),
                Box::new(move |_| choice.set(Some(pick))),
            );
            button.set_style(style);
            button.set_icon(icon);
            button.set_label(label);
            button
        };
        Self {
            scene: Scene::new(),
            open: button(tr!("launcher.open_project"), icons::FOLDER_OPEN, ButtonStyle::Primary, Choice::Open),
            create: button(tr!("launcher.new_project"), icons::FILE_PLUS, ButtonStyle::Secondary, Choice::New),
            title: theme::heading("Ferrite", 28.0, theme::TEXT),
            subtitle: theme::text(
                tr!("launcher.subtitle"),
                theme::FONT_SIZE,
                theme::TEXT_DIM,
            ),
            message: theme::text(&message.unwrap_or_default(), theme::FONT_SIZE, theme::DANGER),
            recent_title: theme::heading(tr!("launcher.recent"), theme::FONT_SIZE, theme::TEXT_DIM),
            recent: recent_rows(&choice),
            choice,
        }
    }

    fn set_message(&mut self, message: &str) {
        self.message.set_content(message);
    }

    /// Lay out, and handle a button that was pressed. Returns the project
    /// the user opened or created.
    pub fn update(&mut self, window: &Window) -> Option<Project> {
        // The whole window; the editor leaves the viewport on its 3D view.
        window.reset_viewport();
        let (w, h) = (window.width as f32, window.height as f32);
        // With recent projects, they stand in a column right of the buttons.
        let total = if self.recent.is_empty() { BUTTON_WIDTH } else { BUTTON_WIDTH + COLUMN_GAP + RECENT_WIDTH };
        let x = ((w - total) / 2.0).floor();
        let top = (h / 2.0 - 110.0).floor();
        let recent_x = x + BUTTON_WIDTH + COLUMN_GAP;
        self.recent_title.prepare_render_at(Position::new(recent_x + 12.0, top + 56.0, 1.0));
        for (index, row) in self.recent.iter_mut().enumerate() {
            let y = top + 84.0 + index as f32 * (ROW_HEIGHT + 4.0);
            row.button.set_offset(Offset { x: recent_x, y });
            row.forget.set_offset(Offset { x: recent_x + RECENT_WIDTH - 34.0, y: y + (ROW_HEIGHT - 24.0) / 2.0 });
            row.name.prepare_render_at(Position::new(recent_x + 12.0, y + 6.0, 2.0));
            row.folder.prepare_render_at(Position::new(recent_x + 12.0, y + 26.0, 2.0));
            row.button.update(&mut self.scene);
            row.forget.update(&mut self.scene);
        }
        self.title.prepare_render_at(Position::new(x, top, 1.0));
        self.subtitle.prepare_render_at(Position::new(x, top + 44.0, 1.0));
        self.open.set_offset(Offset { x, y: top + 84.0 });
        self.create.set_offset(Offset {
            x,
            y: top + 84.0 + BUTTON_HEIGHT + 12.0,
        });
        self.message
            .prepare_render_at(Position::new(x, top + 84.0 + 2.0 * BUTTON_HEIGHT + 36.0, 1.0));
        self.open.update(&mut self.scene);
        self.create.update(&mut self.scene);

        let choice = self.choice.take()?;
        let result = match choice {
            Choice::Open => {
                let file = rfd::FileDialog::new()
                    .set_title(tr!("launcher.open_title"))
                    .add_filter(tr!("launcher.project_filter"), &[PROJECT_EXTENSION])
                    .pick_file()?;
                Project::load(&file)
            }
            Choice::New => {
                let folder: PathBuf = rfd::FileDialog::new()
                    .set_title(tr!("launcher.new_folder_title"))
                    .pick_folder()?;
                Project::create(&folder)
            }
            Choice::Recent(index) => {
                let path = self.recent.get(index)?.path.clone();
                Project::load(&path)
            }
            Choice::Forget(index) => {
                if let Some(row) = self.recent.get(index) {
                    crate::preferences::forget_project(&row.path);
                }
                self.recent = recent_rows(&self.choice);
                return None;
            }
        };
        match result {
            Ok(project) => Some(project),
            Err(e) => {
                self.set_message(&e);
                None
            }
        }
    }

    pub fn render(&self) {
        self.title.render();
        self.subtitle.render();
        self.open.render();
        self.create.render();
        self.message.render();
        if !self.recent.is_empty() {
            self.recent_title.render();
        }
        for row in &self.recent {
            row.button.render();
            row.name.render();
            row.folder.render();
            row.forget.render();
        }
    }

    pub fn handle_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        // The window starts with the cursor captured, as a game wants it.
        if window.get_cursor_mode() != glfw::CursorMode::Normal {
            window.set_cursor_mode(glfw::CursorMode::Normal);
        }
        if self.open.handle_events(&mut self.scene, window, glfw, event)
            || self.create.handle_events(&mut self.scene, window, glfw, event)
        {
            return;
        }
        for row in &mut self.recent {
            // The remove button sits on the row: it goes first.
            if row.forget.handle_events(&mut self.scene, window, glfw, event)
                || row.button.handle_events(&mut self.scene, window, glfw, event)
            {
                return;
            }
        }
    }
}
