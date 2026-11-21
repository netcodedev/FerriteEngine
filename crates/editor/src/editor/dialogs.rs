use std::path::{Path, PathBuf};

use ferrite::core::{
    assets::{self, AssetDatabase, AssetId, AssetKind, AssetRef},
    entity::EntityHandle,
    primitives::{Offset, Size},
    scene::{scene_file::ComponentData, Scene},
    utils::DataSource,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, elements::popup::Popup, UIElement, UI};
use glfw::{Glfw, WindowEvent};

use ferrite::core::material::asset::TextureSetting;

use super::{send, Command, Shared};

pub const SCENE_DIR: &str = "scenes";

/// Let the user choose files to import with the system's file dialog
/// (Windows: the native dialog; Linux: the desktop portal, or zenity
/// without one). Blocks until the dialog is closed; empty if cancelled.
pub fn pick_import_files() -> Vec<PathBuf> {
    let extensions = AssetDatabase::importable_extensions();
    rfd::FileDialog::new()
        .set_title("Import assets")
        .add_filter("Models and textures", &extensions)
        .add_filter("All files", &["*"])
        .pick_files()
        .unwrap_or_default()
}

/// The model assets a scatter layer can use: each asset as a whole and,
/// for assets with several variants, each variant.
fn model_choices() -> Vec<(String, AssetRef)> {
    let database = assets::database();
    let mut models: Vec<_> = database
        .iter()
        .filter_map(|entry| Some((entry, entry.meta.as_model()?)))
        .collect();
    models.sort_by_key(|(entry, _)| entry.meta.name.to_lowercase());
    let mut choices = Vec::new();
    for (entry, model) in models {
        let id = entry.meta.id;
        if model.variants.len() > 1 {
            choices.push((
                format!("{} (all {} variants)", entry.meta.name, model.variants.len()),
                AssetRef::whole(id),
            ));
            for variant in &model.variants {
                choices.push((
                    format!("{} / {}", entry.meta.name, variant.name),
                    AssetRef::part(id, variant.name.clone()),
                ));
            }
        } else {
            choices.push((entry.meta.name.clone(), AssetRef::whole(id)));
        }
    }
    choices
}

/// Ask which model asset a scatter layer of `entity`'s terrain should use.
/// Without a layer index the choice becomes a new layer.
/// A scatter layer plants a scatter graph.
pub fn pick_scatter_asset(state: &Shared, entity: EntityHandle, layer: Option<usize>) {
    pick_asset_of(state, scatter_graph_choices(), move |asset| Command::SetScatterAsset {
        entity,
        layer,
        asset,
    });
}

/// Let the user choose one of the project's scripts to add to `entity`.
pub fn pick_script(state: &Shared, entity: EntityHandle) {
    let names = ferrite::core::script::names();
    let open_flag = DataSource::new(true);
    let cancel_open = open_flag.clone();
    if names.is_empty() {
        let children: Vec<Box<dyn UIElement>> = vec![
            UI::label("The project has no scripts yet.", 14.0),
            UI::label("Create one with File > New script.", 14.0),
            button_row(vec![UI::button(
                "Close",
                Box::new(move |_| cancel_open.write(false)),
            )]),
        ];
        show(state, "Add a script", open_flag, children);
        return;
    }
    let selected = DataSource::new(0u32);
    let height = (names.len().min(10) as f32) * 20.0 + 4.0;
    let use_state = state.clone();
    let use_open = open_flag.clone();
    let use_selected = selected.clone();
    let choices = names.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label("Scripts of the project:", 14.0),
        UI::list(names, selected, Size::new(380.0, height)),
        button_row(vec![
            UI::button(
                "Add",
                Box::new(move |_| {
                    if let Some(name) = choices.get(use_selected.read() as usize) {
                        send(
                            &use_state,
                            Command::AddComponent(entity, ComponentData::script(name)),
                        );
                        use_open.write(false);
                    }
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Add a script", open_flag, children);
}

/// Ask for the name of a new script.
pub fn new_script(state: &Shared) {
    let name = DataSource::new(String::new());
    let open_flag = DataSource::new(true);
    let create_state = state.clone();
    let create_name = name.clone();
    let create_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label("Name of the script (e.g. Rotator):", 14.0),
        UI::input_sized(name, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                "Create",
                Box::new(move |_| {
                    let name = create_name.read().trim().to_owned();
                    if name.is_empty() {
                        return;
                    }
                    send(&create_state, Command::NewScript(name));
                    create_open.write(false);
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "New script", open_flag, children);
}

/// Ask for the name of a new animation clip posed on `model`.
pub fn new_clip(state: &Shared, model: AssetId) {
    let name = DataSource::new(String::new());
    let open_flag = DataSource::new(true);
    let create_state = state.clone();
    let create_name = name.clone();
    let create_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label("Name of the clip (e.g. Axe Swing):", 14.0),
        UI::input_sized(name, Size::new(380.0, 22.0)),
        button_row(vec![
            UI::button(
                "Create",
                Box::new(move |_| {
                    let name = create_name.read().trim().to_owned();
                    if name.is_empty() {
                        return;
                    }
                    send(&create_state, Command::NewClip { model, name });
                    create_open.write(false);
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "New animation clip", open_flag, children);
}

/// Ask which model asset `entity` should show.
pub fn pick_entity_model(state: &Shared, entity: EntityHandle) {
    pick_model_asset(state, move |asset| Command::SetEntityModel { entity, asset });
}

/// Let the user choose a model asset (or one of its variants) and queue
/// the command `then` makes of the choice.
/// Ask which model a scatter graph's `Spawn mesh` node plants.
pub fn pick_scatter_model(state: &Shared, node: ferrite::terrain::scatter::graph::NodeId) {
    pick_model_asset(state, move |asset| Command::SetScatterModel { node, asset });
}

fn pick_model_asset(state: &Shared, then: impl Fn(AssetRef) -> Command + 'static) {
    pick_asset_of(state, model_choices(), then);
}

/// The project's scatter graphs, by name.
fn scatter_graph_choices() -> Vec<(String, AssetRef)> {
    let database = assets::database();
    let mut graphs: Vec<(String, AssetRef)> = database
        .iter()
        .filter(|entry| matches!(entry.meta.kind, AssetKind::ScatterGraph(_)))
        .map(|entry| (format!("{} (scatter graph)", entry.meta.name), AssetRef::whole(entry.meta.id)))
        .collect();
    graphs.sort_by_key(|(name, _)| name.to_lowercase());
    graphs
}

fn pick_asset_of(state: &Shared, choices: Vec<(String, AssetRef)>, then: impl Fn(AssetRef) -> Command + 'static) {
    let open_flag = DataSource::new(true);
    let cancel_open = open_flag.clone();
    if choices.is_empty() {
        let children: Vec<Box<dyn UIElement>> = vec![
            UI::label("There are no model assets yet.", 14.0),
            UI::label("Use Import in the Assets panel to add one.", 14.0),
            button_row(vec![UI::button(
                "Close",
                Box::new(move |_| cancel_open.write(false)),
            )]),
        ];
        show(state, "Choose a model", open_flag, children);
        return;
    }

    let selected = DataSource::new(0u32);
    let labels: Vec<String> = choices.iter().map(|(label, _)| label.clone()).collect();
    let height = (labels.len().min(10) as f32) * 20.0 + 4.0;
    let use_state = state.clone();
    let use_open = open_flag.clone();
    let use_selected = selected.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label("Model assets:", 14.0),
        UI::list(labels, selected, Size::new(380.0, height)),
        button_row(vec![
            UI::button(
                "Use",
                Box::new(move |_| {
                    if let Some((_, asset)) = choices.get(use_selected.read() as usize) {
                        send(&use_state, then(asset.clone()));
                        use_open.write(false);
                    }
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Choose a model", open_flag, children);
}

/// A list to choose one of `choices` from; `then` gets the chosen value.
fn pick_from<T: Clone + 'static>(
    state: &Shared,
    title: &str,
    heading: &str,
    empty_note: Option<&str>,
    choices: Vec<(String, T)>,
    then: impl Fn(T) -> Command + 'static,
) {
    let open_flag = DataSource::new(true);
    let cancel_open = open_flag.clone();
    let selected = DataSource::new(0u32);
    let labels: Vec<String> = choices.iter().map(|(label, _)| label.clone()).collect();
    let height = (labels.len().min(10) as f32) * 20.0 + 4.0;
    let use_state = state.clone();
    let use_open = open_flag.clone();
    let use_selected = selected.clone();
    let mut children: Vec<Box<dyn UIElement>> = vec![
        UI::label(heading, 14.0),
        UI::list(labels, selected, Size::new(380.0, height)),
    ];
    if let Some(note) = empty_note {
        children.push(UI::label(note, 14.0));
    }
    children.push(button_row(vec![
        UI::button(
            "Use",
            Box::new(move |_| {
                if let Some((_, value)) = choices.get(use_selected.read() as usize) {
                    send(&use_state, then(value.clone()));
                    use_open.write(false);
                }
            }),
        ),
        UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
    ]));
    show(state, title, open_flag, children);
}

/// Assets of a kind, by name.
fn assets_of(kind: impl Fn(&AssetKind) -> bool) -> Vec<(String, AssetId)> {
    let database = assets::database();
    let mut found: Vec<(String, AssetId)> = database
        .iter()
        .filter(|entry| kind(&entry.meta.kind))
        .map(|entry| (entry.meta.name.clone(), entry.meta.id))
        .collect();
    found.sort_by_key(|(name, _)| name.to_lowercase());
    found
}

/// Ask which material asset a material slot of the edited model asset is
/// drawn with. The first choice goes back to the file's own material.
pub fn pick_model_material(state: &Shared, material: usize) {
    let mut choices: Vec<(String, Option<AssetId>)> = vec![("from the file".to_string(), None)];
    choices.extend(assets_of(|k| matches!(k, AssetKind::Material(_))).into_iter().map(|(n, id)| (n, Some(id))));
    let empty = (choices.len() == 1).then_some("No material assets yet.");
    pick_from(state, "Choose a material", "Material assets:", empty, choices, move |asset| {
        Command::SetModelMaterial { material, asset }
    });
}

/// Ask which texture a texture parameter of the material being edited
/// uses: automatic (what the model loader finds), none, or a texture asset.
pub fn pick_material_texture(state: &Shared, param: String) {
    let mut choices: Vec<(String, TextureSetting)> = vec![
        ("automatic (from the model)".to_string(), TextureSetting::Auto),
        ("none".to_string(), TextureSetting::None),
    ];
    choices.extend(
        assets_of(|k| matches!(k, AssetKind::Texture(_))).into_iter().map(|(n, id)| (n, TextureSetting::Asset(id))),
    );
    let empty = (choices.len() == 2).then_some("No texture assets yet. Import images in the Assets panel.");
    let heading = format!("Texture for {param}:");
    pick_from(state, "Choose a texture", &heading, empty, choices, move |texture| {
        Command::SetMaterialTexture { param: param.clone(), texture }
    });
}

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

/// Ask where to save the scene. `then` runs once it is saved.
pub fn save_as(state: &Shared, then: Option<Command>) {
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
                    let path = PathBuf::from(text);
                    let then = then.clone().map(Box::new);
                    let current = save_state.borrow().scene_path.clone();
                    let replaces_another = path.exists()
                        && !current.is_some_and(|current| same_file(&current, &path));
                    if replaces_another {
                        // Replaces this popup with the question.
                        confirm_overwrite(&save_state, path, then);
                    } else {
                        send(&save_state, Command::SaveAs { path, then });
                        save_open.write(false);
                    }
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

/// Whether two paths name the same file (`a` may be relative, `b` absolute).
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

/// `path` exists already and is not the open scene: replace it?
fn confirm_overwrite(state: &Shared, path: PathBuf, then: Option<Box<Command>>) {
    let open_flag = DataSource::new(true);
    let replace_state = state.clone();
    let replace_open = open_flag.clone();
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(&format!("{} exists already.", path.display()), 14.0),
        UI::label("Replace it with this scene?", 14.0),
        button_row(vec![
            UI::button(
                "Replace",
                Box::new(move |_| {
                    send(
                        &replace_state,
                        Command::SaveAs {
                            path: path.clone(),
                            then: then.clone(),
                        },
                    );
                    replace_open.write(false);
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Replace scene?", open_flag, children);
}

/// What has not been saved yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Unsaved {
    pub scene: bool,
    pub graph: bool,
    pub asset: bool,
}

impl Unsaved {
    pub fn any(&self) -> bool {
        self.scene || self.graph || self.asset
    }

    fn describe(&self) -> String {
        let parts: Vec<&str> = [
            (self.scene, "the scene"),
            (self.graph, "the animation graph"),
            (self.asset, "the edited asset"),
        ]
        .into_iter()
        .filter_map(|(unsaved, what)| unsaved.then_some(what))
        .collect();
        let list = match parts.as_slice() {
            [] => "nothing".to_string(),
            [one] => one.to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        format!("There are unsaved changes in {list}.")
    }
}

/// Unsaved changes stand in the way of `then`: save them first, discard
/// them, or cancel. With `go_on`, its button replaces "Discard" and runs its
/// command with the changes kept (an export leaves them out).
pub fn confirm_discard(
    state: &Shared,
    then: Command,
    unsaved: Unsaved,
    go_on: Option<(&str, Command)>,
) {
    let open_flag = DataSource::new(true);
    let save_state = state.clone();
    let save_open = open_flag.clone();
    let save_then = then.clone();
    let cancel_open = open_flag.clone();
    let other_state = state.clone();
    let other_open = open_flag.clone();
    let other = match go_on {
        Some((label, command)) => UI::button(
            label,
            Box::new(move |_| {
                send(&other_state, command.clone());
                other_open.write(false);
            }),
        ),
        None => UI::button(
            "Discard",
            Box::new(move |_| {
                other_state.borrow_mut().dirty = false;
                send(&other_state, then.clone());
                other_open.write(false);
            }),
        ),
    };
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(&unsaved.describe(), 14.0),
        button_row(vec![
            UI::button(
                "Save first",
                Box::new(move |_| {
                    // Runs `then` only once everything is saved.
                    send(&save_state, Command::SaveFirst(Box::new(save_then.clone())));
                    save_open.write(false);
                }),
            ),
            other,
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Unsaved changes", open_flag, children);
}

/// The open graph has unsaved edits and another graph should be opened.
pub fn confirm_graph_switch(state: &Shared, current: &str, then: Command) {
    let open_flag = DataSource::new(true);
    let (save_state, save_open, save_then) = (state.clone(), open_flag.clone(), then.clone());
    let (discard_state, discard_open) = (state.clone(), open_flag.clone());
    let cancel_open = open_flag.clone();
    let children: Vec<Box<dyn UIElement>> = vec![
        UI::label(&format!("{current} has unsaved changes."), 14.0),
        button_row(vec![
            UI::button(
                "Save first",
                Box::new(move |_| {
                    // Switches only once the graph is saved.
                    send(&save_state, Command::SaveFirst(Box::new(save_then.clone())));
                    save_open.write(false);
                }),
            ),
            UI::button(
                "Discard",
                Box::new(move |_| {
                    discard_state.borrow_mut().anim.dirty = false;
                    send(&discard_state, then.clone());
                    discard_open.write(false);
                }),
            ),
            UI::button("Cancel", Box::new(move |_| cancel_open.write(false))),
        ]),
    ];
    show(state, "Unsaved animation graph", open_flag, children);
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

    fn is_modal(&self) -> bool {
        self.current.is_some()
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsaved_changes_are_named() {
        let scene = Unsaved {
            scene: true,
            ..Default::default()
        };
        assert!(scene.any());
        assert!(!Unsaved::default().any());
        assert_eq!(scene.describe(), "There are unsaved changes in the scene.");
        let all = Unsaved {
            scene: true,
            graph: true,
            asset: true,
        };
        assert_eq!(
            all.describe(),
            "There are unsaved changes in the scene, the animation graph and the edited asset."
        );
    }
}
