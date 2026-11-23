// Links the engine as a shared library, which the game scripts share.
#[allow(unused_imports)]
use ferrite_dylib;

// First: the other modules use its `tr!`.
#[macro_use]
mod i18n;
mod editor;
mod editor_layer;
mod launcher;
mod loading;
mod new_script;
mod preferences;
mod project_lock;

use std::path::{Path, PathBuf};

use ferrite::core::{
    application::{Application, Layer},
    project::Project,
    window::Window,
};
use editor_layer::EditorLayer;
use glfw::{CursorMode, Glfw, WindowEvent};
use launcher::Launcher;
use loading::{Loading, Progress};

fn main() {
    // The user's language, before anything is shown.
    preferences::load();
    // The editor draws its own title bar.
    let mut application = Application::with_decorations(1280, 720, "Ferrite Editor", false);
    application.set_clear_color(ferrite_ui::ui::theme::BG_APP);
    // The project given as the first argument (a .fp file or its folder),
    // or one in the working directory, is opened right away; without one
    // the user picks or creates one.
    let app = match project_to_open() {
        Ok(project) => EditorApp::new(project),
        Err(e) => EditorApp::Launcher(Launcher::new(Some(e))),
    };
    application.add_layer(Box::new(app));
    application.start();
}

fn project_to_open() -> Result<Option<PathBuf>, String> {
    match std::env::args_os().nth(1) {
        Some(arg) => find_project(Path::new(&arg)).map(Some),
        None => Ok(std::env::current_dir()
            .ok()
            .as_deref()
            .and_then(Project::find_in)),
    }
}

/// The project file `given` names: itself, or the one in the folder.
fn find_project(given: &Path) -> Result<PathBuf, String> {
    if given.is_dir() {
        Project::find_in(given)
            .ok_or_else(|| tr!("main.no_project", path = given.display()))
    } else if given.is_file() {
        Ok(given.to_path_buf())
    } else {
        Err(tr!("main.not_exist", path = given.display()))
    }
}

/// The launcher until a project is chosen, the loading screen while it is
/// opened, then the editor.
enum EditorApp {
    Launcher(Launcher),
    Loading(Loading),
    Editor(Box<EditorLayer>),
}

impl EditorApp {
    fn new(project: Option<PathBuf>) -> Self {
        let Some(path) = project else {
            return EditorApp::Launcher(Launcher::new(None));
        };
        match Project::load(&path) {
            Ok(project) => Self::open(project),
            Err(e) => EditorApp::Launcher(Launcher::new(Some(e))),
        }
    }

    /// Start opening `project`, if no other editor has it open.
    fn open(project: Project) -> Self {
        match project_lock::hold(project.name(), project.dir()) {
            Ok(()) => EditorApp::Loading(Loading::new(project)),
            Err(e) => {
                log::warn!("{e}");
                EditorApp::Launcher(Launcher::new(Some(e)))
            }
        }
    }
}

impl Layer for EditorApp {
    fn on_update(&mut self, window: &Window, delta_time: f64) {
        project_lock::heartbeat();
        match self {
            EditorApp::Launcher(launcher) => match launcher.update(window) {
                Some(project) => *self = EditorApp::open(project),
                None => launcher.render(),
            },
            EditorApp::Loading(loading) => match loading.update(window) {
                Progress::Working => {}
                Progress::Ready(project, prepared) => {
                    *self = EditorApp::Editor(Box::new(EditorLayer::new(project, prepared)));
                }
                Progress::Failed(e) => {
                    project_lock::release();
                    *self = EditorApp::Launcher(Launcher::new(Some(e)));
                }
            },
            EditorApp::Editor(editor) => editor.on_update(window, delta_time),
        }
    }

    fn on_event(&mut self, glfw: &mut Glfw, window: &mut glfw::Window, event: &WindowEvent) {
        match self {
            EditorApp::Launcher(launcher) => launcher.handle_event(glfw, window, event),
            EditorApp::Loading(_) => {
                if window.get_cursor_mode() != CursorMode::Normal {
                    window.set_cursor_mode(CursorMode::Normal);
                }
            }
            EditorApp::Editor(editor) => editor.on_event(glfw, window, event),
        }
    }

    fn get_name(&self) -> &str {
        "Editor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_argument_is_a_file_or_its_folder() {
        let dir = std::env::temp_dir().join(format!("ferrite-editor-arg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(find_project(&dir).unwrap_err().contains("no project"));
        let file = dir.join("Game.fp");
        std::fs::write(&file, "(version: 1, name: \"Game\")").unwrap();
        assert_eq!(find_project(&dir).unwrap(), file);
        assert_eq!(find_project(&file).unwrap(), file);
        assert!(find_project(&dir.join("missing")).unwrap_err().contains("does not exist"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
