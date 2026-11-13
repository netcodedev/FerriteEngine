//! Opening a project while showing that it is happening: what needs no GL
//! (scanning and registering assets, building the scripts) runs on a
//! background thread while a loading screen animates; the scene is built
//! after that on the main thread, with the screen saying so.

use std::{
    f32::consts::TAU,
    sync::{mpsc, Arc, Mutex},
    time::Instant,
};

use ferrite::core::{
    assets,
    primitives::{Position, Size},
    project::Project,
    renderer::{
        plane::{Plane, PlaneBuilder},
        text::Text,
    },
    window::Window,
};
use ferrite_ui::ui::theme;

const DOTS: usize = 8;
const DOT_SIZE: f32 = 7.0;
const SPINNER_RADIUS: f32 = 14.0;

/// What was done for a project before the editor is built for it.
pub struct Prepared {
    /// Files put into `assets/` by hand that became assets.
    pub registered: usize,
    pub asset_problems: Vec<String>,
    /// Whether the scripts could be built.
    pub scripts: Result<(), String>,
}

enum Stage {
    /// The background work is running.
    Preparing(mpsc::Receiver<Result<Prepared, String>>),
    /// Done; the next frame builds the editor (the scene, on this thread).
    /// This frame shows that it is coming.
    BuildingEditor(Option<Prepared>),
}

pub enum Progress {
    Working,
    /// Ready for the editor to be built.
    Ready(Project, Prepared),
    Failed(String),
}

pub struct Loading {
    project: Option<Project>,
    stage: Stage,
    /// What the background thread is doing, for the screen.
    step: Arc<Mutex<String>>,
    title: Text,
    step_text: Text,
    dots: Vec<Plane>,
    started: Instant,
}

impl Loading {
    pub fn new(project: Project) -> Self {
        let step = Arc::new(Mutex::new("Opening the project".to_string()));
        let (send, receive) = mpsc::channel();
        let thread_project = project.clone();
        let thread_step = step.clone();
        std::thread::spawn(move || {
            let _ = send.send(prepare(&thread_project, &thread_step));
        });
        let dots = (0..DOTS)
            .map(|_| {
                PlaneBuilder::new()
                    .size(Size::new(DOT_SIZE, DOT_SIZE))
                    .color(theme::ACCENT)
                    .border_radius_uniform(DOT_SIZE / 2.0)
                    .build()
            })
            .collect();
        Self {
            title: theme::heading(&format!("Opening {}", project.name()), 20.0, theme::TEXT),
            step_text: theme::text("", theme::FONT_SIZE, theme::TEXT_DIM),
            project: Some(project),
            stage: Stage::Preparing(receive),
            step,
            dots,
            started: Instant::now(),
        }
    }

    /// Advance and draw. Once it returns `Ready`, the caller builds the
    /// editor.
    pub fn update(&mut self, window: &Window) -> Progress {
        let progress = self.advance();
        self.draw(window);
        progress
    }

    fn advance(&mut self) -> Progress {
        match &mut self.stage {
            Stage::Preparing(receive) => match receive.try_recv() {
                Ok(Ok(prepared)) => {
                    *self.step.lock().unwrap() = "Loading the scene".to_string();
                    self.stage = Stage::BuildingEditor(Some(prepared));
                    Progress::Working
                }
                Ok(Err(e)) => Progress::Failed(e),
                Err(mpsc::TryRecvError::Empty) => Progress::Working,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Progress::Failed("opening the project stopped".into())
                }
            },
            Stage::BuildingEditor(prepared) => match (self.project.take(), prepared.take()) {
                (Some(project), Some(prepared)) => Progress::Ready(project, prepared),
                _ => Progress::Working,
            },
        }
    }

    fn draw(&mut self, window: &Window) {
        unsafe { gl::Viewport(0, 0, window.width as i32, window.height as i32) };
        let (w, h) = (window.width as f32, window.height as f32);
        let x = (w / 2.0 - 140.0).floor();
        let y = (h / 2.0 - 40.0).floor();

        // Dots on a circle, the brightest one going round.
        let center = (x + SPINNER_RADIUS, y + 60.0 + SPINNER_RADIUS);
        let turn = self.started.elapsed().as_secs_f32() * 1.2;
        for (i, dot) in self.dots.iter_mut().enumerate() {
            let angle = i as f32 / DOTS as f32 * TAU;
            dot.set_position(Position::new(
                center.0 + angle.cos() * SPINNER_RADIUS - DOT_SIZE / 2.0,
                center.1 + angle.sin() * SPINNER_RADIUS - DOT_SIZE / 2.0,
                1.0,
            ));
            let behind = (turn - i as f32 / DOTS as f32).rem_euclid(1.0);
            let (r, g, b, _) = theme::ACCENT;
            dot.set_color((r, g, b, 1.0 - behind * 0.85));
            dot.render();
        }

        self.title.prepare_render_at(Position::new(x, y, 1.0));
        self.title.render();
        let step = format!("{}...", self.step.lock().unwrap());
        self.step_text.set_content(&step);
        self.step_text
            .prepare_render_at(Position::new(x + 2.0 * SPINNER_RADIUS + 16.0, y + 60.0 + 6.0, 1.0));
        self.step_text.render();
    }
}

/// The work that needs no GL, on the background thread.
fn prepare(project: &Project, step: &Mutex<String>) -> Result<Prepared, String> {
    let set = |text: &str| *step.lock().unwrap() = text.to_string();
    set("Scanning the assets");
    project.enter()?;
    // Files that were put into the asset folder by hand become assets, so
    // everything in there can be referred to the same way.
    set("Registering new assets");
    let (registered, asset_problems) = assets::database().register_new_files();
    set("Building the scripts");
    let scripts = project.scripts().build();
    Ok(Prepared {
        registered,
        asset_problems,
        scripts,
    })
}
