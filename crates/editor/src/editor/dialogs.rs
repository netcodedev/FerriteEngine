use std::path::{Path, PathBuf};

use ferrite::core::{
    primitives::{Offset, Size},
    scene::Scene,
    utils::DataSource,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, elements::popup::Popup, UIElement, UI};
use glfw::{Glfw, WindowEvent};

use super::{send, Command, Shared};

pub const SCENE_DIR: &str = "scenes";

/// A popup plus the flag that keeps it open (its close button and the
/// dialog's own buttons write `false`).
pub struct Modal {
    pub open: DataSource<bool>,
    pub popup: Popup,
}

/// Queue a popup for the modal host. `open` is shared with the dialog's
/// buttons, so either they or the popup's own "x" can close it.
fn show(state: &Shared, title: &str, open: DataSource<bool>, children: Vec<Box<dyn UIElement>>) {
    let popup = Popup::new(title.to_owned(), open.clone(), children);
    state.borrow_mut().pending_modal = Some(Modal { open, popup });
}

fn button_row(buttons: Vec<Box<dyn UIElement>>) -> Box<dyn UIElement> {
    let mut row = UI::container_horizontal();
    for button in buttons {
        row.add_child(button);
    }
    row
}

pub fn save_as(state: &Shared) {
    let current = state
        .borrow()
        .scene_path
        .clone()
        .unwrap_or_else(|| Path::new(SCENE_DIR).join("untitled.ron"));
    let path = DataSource::new(current.to_string_lossy().into_owned());
    let open_flag = DataSource::new(true);

    let save_state = state.clone();
    let save_path = path.clone();
    let save_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label("File path (relative to the project root):", 14.0),
        UI::input_sized(path, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                "Save",
                Box::new(move |_| {
                    let mut text = save_path.read().trim().to_owned();
                    if text.is_empty() {
                        return;
                    }
                    if !text.ends_with(".ron") {
                        text.push_str(".ron");
                    }
                    send(&save_state, Command::SaveAs(PathBuf::from(text)));
                    save_open.write(false);
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Save scene as", open_flag, children);
}

pub fn open(state: &Shared) {
    let files = scene_files();
    let selected = DataSource::new(0u32);
    let typed = DataSource::new(String::new());
    let open_flag = DataSource::new(true);

    let mut children: Vec<Box<dyn UIElement>> = Vec::new();
    if files.is_empty() {
        children.push(UI::label(&format!("No .ron files in {SCENE_DIR}/ yet."), 14.0));
    } else {
        children.push(UI::label(&format!("Scenes in {SCENE_DIR}/:"), 14.0));
        let labels = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let height = (files.len().min(8) as f32) * 20.0 + 4.0;
        children.push(UI::list(labels, selected.clone(), Size::new(380.0, height)));
    }
    children.push(UI::label("Or type a path:", 14.0));
    children.push(UI::input_sized(typed.clone(), Size::new(380.0, 22.0)));

    let open_state = state.clone();
    let accept_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    children.push(button_row(vec![
        UI::button(
            "Open",
            Box::new(move |_| {
                let typed = typed.read().trim().to_owned();
                let path = if !typed.is_empty() {
                    Some(PathBuf::from(typed))
                } else {
                    files.get(selected.read() as usize).cloned()
                };
                if let Some(path) = path {
                    send(&open_state, Command::Open(path));
                    accept_open.write(false);
                }
            }),
        ),
        UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
    ]));
    show(state, "Open scene", open_flag, children);
}

pub fn confirm_discard(state: &Shared, then: Command) {
    let open_flag = DataSource::new(true);
    let discard_state = state.clone();
    let discard_open = open_flag.clone();
    let save_state = state.clone();
    let save_open = open_flag.clone();
    let save_then = then.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label("The scene has unsaved changes.", 14.0),
        button_row(vec![
            UI::button(
                "Save first",
                Box::new(move |_| {
                    send(&save_state, Command::Save);
                    send(&save_state, save_then.clone());
                    save_open.write(false);
                }),
            ),
            UI::button(
                "Discard",
                Box::new(move |_| {
                    discard_state.borrow_mut().dirty = false;
                    send(&discard_state, then.clone());
                    discard_open.write(false);
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Unsaved changes", open_flag, children);
}

fn scene_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(SCENE_DIR)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |ext| ext == "ron"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// Shows the editor's current modal dialog (at most one at a time).
pub struct ModalHost {
    handle: UIElementHandle,
    state: Shared,
    current: Option<Modal>,
    offset: Offset,
    size: Size,
}

impl ModalHost {
    pub fn new(state: Shared) -> Self {
        Self {
            handle: UIElementHandle::new(),
            state,
            current: None,
            offset: Offset::default(),
            size: Size::default(),
        }
    }

}

impl UIElement for ModalHost {
    fn update(&mut self, scene: &mut Scene) {
        if let Some(modal) = self.state.borrow_mut().pending_modal.take() {
            self.current = Some(modal);
        }
        if self.current.as_ref().map_or(false, |m| !m.open.read()) {
            self.current = None;
        }
        if let Some(modal) = &mut self.current {
            modal.popup.update(scene);
        }
    }

    fn render(&self) {}

    fn render_overlay(&self) {
        if let Some(modal) = &self.current {
            modal.popup.render_overlay();
        }
    }

    fn handle_events(
        &mut self,
        scene: &mut Scene,
        window: &mut glfw::Window,
        glfw: &mut Glfw,
        event: &WindowEvent,
    ) -> bool {
        match &mut self.current {
            Some(modal) => modal.popup.handle_events(scene, window, glfw, event),
            None => false,
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
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, _z_index: f32) {}
}
