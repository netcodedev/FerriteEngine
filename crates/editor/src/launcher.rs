//! The screen the editor starts with when the working directory holds no
//! project: open a project file or create a new project.

use std::{cell::Cell, path::PathBuf, rc::Rc};

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

#[derive(Clone, Copy)]
enum Choice {
    Open,
    New,
}

pub struct Launcher {
    /// Buttons are handed a scene; there is nothing in it.
    scene: Scene,
    open: Button,
    create: Button,
    title: Text,
    subtitle: Text,
    message: Text,
    choice: Rc<Cell<Option<Choice>>>,
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
            open: button("Open project...", icons::FOLDER_OPEN, ButtonStyle::Primary, Choice::Open),
            create: button("New project...", icons::FILE_PLUS, ButtonStyle::Secondary, Choice::New),
            title: theme::heading("Ferrite", 28.0, theme::TEXT),
            subtitle: theme::text(
                "Open a project (.fp file) or create a new one.",
                theme::FONT_SIZE,
                theme::TEXT_DIM,
            ),
            message: theme::text(&message.unwrap_or_default(), theme::FONT_SIZE, theme::DANGER),
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
        unsafe { gl::Viewport(0, 0, window.width as i32, window.height as i32) };
        let (w, h) = (window.width as f32, window.height as f32);
        let x = ((w - BUTTON_WIDTH) / 2.0).floor();
        let top = (h / 2.0 - 110.0).floor();
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
                    .set_title("Open project")
                    .add_filter("Ferrite project", &[PROJECT_EXTENSION])
                    .pick_file()?;
                Project::load(&file)
            }
            Choice::New => {
                let folder: PathBuf = rfd::FileDialog::new()
                    .set_title("Folder for the new project (named after it)")
                    .pick_folder()?;
                Project::create(&folder)
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
    }

    pub fn handle_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        // The window starts with the cursor captured, as a game wants it.
        if window.get_cursor_mode() != glfw::CursorMode::Normal {
            window.set_cursor_mode(glfw::CursorMode::Normal);
        }
        if !self.open.handle_events(&mut self.scene, window, glfw, event) {
            self.create.handle_events(&mut self.scene, window, glfw, event);
        }
    }
}
